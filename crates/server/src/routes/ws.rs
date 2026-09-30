//! Kiosk WebSocket: frames in, results out (plan Section 5).
//!
//! Protocol (both directions JSON text frames):
//!
//! Client -> server:
//!   `{"type":"hello","device_token":"kiosk_..."}`   (once, first message)
//!   `{"type":"frame","image":"<base64 jpeg>"}`      (repeated, ~3-5 fps)
//!   `{"type":"reset"}`                              (start a new attempt)
//!
//! Server -> client:
//!   `{"type":"ready","device":"...","gallery_students":N}`
//!   `{"type":"result","kind":"continue","prompt":"..."}`
//!   `{"type":"result","kind":"accepted","student":{...},"similarity":0.72}`
//!   `{"type":"result","kind":"rejected","message":"..."}`
//!   `{"type":"error","message":"..."}`
//!
//! The device token is sent in the first `hello` message rather than a header,
//! because browser `WebSocket` cannot set custom headers. It is verified before
//! any frame is processed.

use crate::attendance::{resolve, AttendanceSession, SessionVerdict};
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use base64::Engine;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// Max base64 length of one frame (~2 MB decoded). The kiosk sends ~640x480 at
/// q0.7, orders of magnitude below this.
const MAX_FRAME_B64_LEN: usize = 3 * 1024 * 1024;

/// Max frames processed per second per connection. A kiosk sends ~4/s; this
/// leaves headroom while stopping a runaway client from pinning the CPU.
const MAX_FRAMES_PER_SEC: f64 = 15.0;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Hello { device_token: String },
    Frame { image: String },
    Reset,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Ready {
        device_id: Uuid,
        device_name: String,
        gallery_students: usize,
    },
    Result {
        kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        student: Option<StudentView>,
        #[serde(skip_serializing_if = "Option::is_none")]
        similarity: Option<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        already_marked: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        frames_seen: Option<usize>,
    },
    Error {
        message: String,
    },
}

#[derive(Serialize)]
struct StudentView {
    id: Uuid,
    nama: String,
    nis: String,
}

pub async fn kiosk_ws(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle(socket, state))
}

async fn handle(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // --- Handshake: wait for the hello message and authenticate the device. ---
    let device = match await_hello(&mut receiver, &state).await {
        Ok(device) => device,
        Err(e) => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::Error {
                        message: e.public_message(),
                    })
                    .unwrap_or_default()
                    .into(),
                ))
                .await;
            return;
        }
    };

    let _ = db::device_repo::touch(&state.db, device.id).await;

    let ready = ServerMessage::Ready {
        device_id: device.id,
        device_name: device.nama.clone(),
        gallery_students: state.gallery.student_count(),
    };
    if send(&mut sender, &ready).await.is_err() {
        return;
    }

    // --- Frame loop. ---
    let window_size = state.config.consensus_window;
    let mut session = AttendanceSession::new(device, window_size);
    let face = Arc::clone(&state.face);

    // Per-connection frame rate limiter (token bucket). Keeps a runaway client
    // from pinning a CPU core with back-to-back inference passes.
    let mut frame_budget = MAX_FRAMES_PER_SEC;
    let mut last_refill = std::time::Instant::now();

    while let Some(Ok(msg)) = receiver.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        let parsed: ClientMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(_) => {
                let _ = send(
                    &mut sender,
                    &ServerMessage::Error {
                        message: "pesan tidak dikenal".into(),
                    },
                )
                .await;
                continue;
            }
        };

        match parsed {
            ClientMessage::Hello { .. } => {
                // Duplicate hello — ignore; already authenticated.
            }
            ClientMessage::Reset => {
                session.reset();
                let _ = send(
                    &mut sender,
                    &ServerMessage::Result {
                        kind: "continue",
                        prompt: Some("Silakan hadapkan wajah ke kamera.".into()),
                        message: None,
                        student: None,
                        similarity: None,
                        already_marked: None,
                        frames_seen: Some(0),
                    },
                )
                .await;
            }
            ClientMessage::Frame { image } => {
                // Refill the bucket, then spend one token. Over budget, drop the
                // frame without running inference.
                let now = std::time::Instant::now();
                frame_budget = (frame_budget
                    + now.duration_since(last_refill).as_secs_f64() * MAX_FRAMES_PER_SEC)
                    .min(MAX_FRAMES_PER_SEC);
                last_refill = now;
                if frame_budget < 1.0 {
                    continue;
                }
                frame_budget -= 1.0;

                // Guard before any decoding: an oversized payload must not reach
                // base64/JPEG/inference. The kiosk sends ~640x480 at q0.7, well
                // under this; anything larger is a bug or abuse.
                if image.len() > MAX_FRAME_B64_LEN {
                    let _ = send(
                        &mut sender,
                        &ServerMessage::Error {
                            message: "Frame terlalu besar.".into(),
                        },
                    )
                    .await;
                    continue;
                }

                // Decode (cheap) on the async task; run the CPU-bound pipeline
                // on a blocking thread so we never stall the runtime.
                let payload = image
                    .split_once(',')
                    .map(|(_, b)| b)
                    .unwrap_or(&image)
                    .to_string();
                let decoded = tokio::task::spawn_blocking(move || {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(payload.trim())
                        .ok()?;
                    face_core::RgbImage::decode(&bytes).ok()
                })
                .await
                .ok()
                .flatten();

                let Some(frame) = decoded else {
                    let _ = send(
                        &mut sender,
                        &ServerMessage::Result {
                            kind: "continue",
                            prompt: Some("Frame tidak dapat dibaca.".into()),
                            message: None,
                            student: None,
                            similarity: None,
                            already_marked: None,
                            frames_seen: None,
                        },
                    )
                    .await;
                    continue;
                };

                let face_engine = Arc::clone(&face);
                let thresholds = state.config.thresholds();
                let gallery = Arc::clone(&state.gallery);

                // observe() is CPU-bound (inference + matching). Move the
                // session into a blocking task and bring it back, so the async
                // runtime thread is never stalled.
                let moved_session = std::mem::replace(
                    &mut session,
                    AttendanceSession::new_placeholder(window_size),
                );
                let (session_back, obs) = tokio::task::spawn_blocking(move || {
                    let mut s = moved_session;
                    let o = s.observe(&face_engine, &frame, &gallery, &thresholds);
                    (s, o)
                })
                .await
                .expect("blocking task panicked");
                session = session_back;

                match resolve(&state, &mut session, obs).await {
                    Ok(SessionVerdict::Continue {
                        prompt,
                        frames_seen,
                    }) => {
                        let _ = send(
                            &mut sender,
                            &ServerMessage::Result {
                                kind: "continue",
                                prompt: Some(prompt),
                                message: None,
                                student: None,
                                similarity: None,
                                already_marked: None,
                                frames_seen: Some(frames_seen),
                            },
                        )
                        .await;
                    }
                    Ok(SessionVerdict::Accepted {
                        student_id,
                        similarity,
                        already_marked,
                        ..
                    }) => {
                        let student = db::student_repo::find(&state.db, student_id)
                            .await
                            .ok()
                            .flatten()
                            .map(|s| StudentView {
                                id: s.id,
                                nama: s.nama,
                                nis: s.nis,
                            });
                        let _ = send(
                            &mut sender,
                            &ServerMessage::Result {
                                kind: "accepted",
                                prompt: None,
                                message: Some(if already_marked {
                                    "Sudah absen.".into()
                                } else {
                                    "Absensi tercatat.".into()
                                }),
                                student,
                                similarity: Some(similarity),
                                already_marked: Some(already_marked),
                                frames_seen: None,
                            },
                        )
                        .await;
                    }
                    Ok(SessionVerdict::Rejected { message }) => {
                        let _ = send(
                            &mut sender,
                            &ServerMessage::Result {
                                kind: "rejected",
                                prompt: None,
                                message: Some(message),
                                student: None,
                                similarity: None,
                                already_marked: None,
                                frames_seen: None,
                            },
                        )
                        .await;
                    }
                    Err(e) => {
                        let _ = send(
                            &mut sender,
                            &ServerMessage::Error {
                                message: e.public_message(),
                            },
                        )
                        .await;
                    }
                }
            }
        }
    }
}

async fn await_hello<S>(receiver: &mut S, state: &AppState) -> Result<db::models::Device, ApiError>
where
    S: futures::Stream<Item = Result<Message, axum::Error>> + Unpin,
{
    // Bound the wait so a silent client cannot hold the slot forever.
    let hello = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(t) = msg {
                if let Ok(ClientMessage::Hello { device_token }) =
                    serde_json::from_str::<ClientMessage>(&t)
                {
                    return Some(device_token);
                }
            }
        }
        None
    })
    .await
    .map_err(|_| ApiError::Unauthorized)?;

    let token = hello.ok_or(ApiError::Unauthorized)?;
    let device = db::device_repo::authenticate(&state.db, &token)
        .await
        .map_err(ApiError::Db)?
        .ok_or(ApiError::Unauthorized)?;
    Ok(device)
}

async fn send<S>(sender: &mut S, msg: &ServerMessage) -> Result<(), ()>
where
    S: futures::Sink<Message> + Unpin,
{
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sender
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| ())
}
