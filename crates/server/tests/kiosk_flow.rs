//! End-to-end integration test for the kiosk WebSocket flow.
//!
//! Requires a running server (`ABSENSI_TEST_URL`) and a device token
//! (`ABSENSI_TEST_DEVICE_TOKEN`) plus the enrollment frame to replay
//! (`ABSENSI_TEST_FRAME_FILE` — either base64 JPEG or `@/path/to.jpg`). All
//! three come from the smoke-test script. When any is missing the test is
//! skipped, so `cargo test` stays green on a machine with no database.
//!
//! This is the automated form of the manual smoke test in `scripts/smoke.sh`.

use futures::{SinkExt, Stream, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

#[tokio::test]
async fn kiosk_websocket_accepts_enrolled_student() {
    let Some(base) = env("ABSENSI_TEST_URL") else {
        eprintln!("skipping: ABSENSI_TEST_URL not set");
        return;
    };
    let Some(token) = env("ABSENSI_TEST_DEVICE_TOKEN") else {
        eprintln!("skipping: ABSENSI_TEST_DEVICE_TOKEN not set");
        return;
    };
    let Some(frame_env) = env("ABSENSI_TEST_FRAME_FILE") else {
        eprintln!("skipping: ABSENSI_TEST_FRAME_FILE not set");
        return;
    };
    // The value is either a base64 JPEG, or `@/path/to/image.jpg` to have the
    // test read and encode the file (avoids a huge environment variable).
    let frame = if let Some(path) = frame_env.strip_prefix('@') {
        let bytes = std::fs::read(path).expect("read frame file");
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    } else {
        frame_env
    };

    let ws_url = base.replacen("http", "ws", 1) + "/ws/kiosk";
    let (mut socket, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("connect");

    // hello
    socket
        .send(Message::Text(
            json!({"type":"hello","device_token":token})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let ready = next_json(&mut socket).await;
    assert_eq!(ready["type"], "ready", "expected ready, got {ready}");

    // Send enough frames to reach consensus (window default 7, required 5).
    let mut accepted = None;
    for i in 0..10 {
        socket
            .send(Message::Text(
                json!({"type":"frame","image":frame}).to_string().into(),
            ))
            .await
            .unwrap();
        let msg = next_json(&mut socket).await;
        if msg["type"] == "result" && msg["kind"] == "accepted" {
            accepted = Some(msg);
            break;
        }
        if msg["type"] == "error" {
            panic!("server error on frame {i}: {msg}");
        }
    }

    let accepted = accepted.expect("student should be accepted within 10 frames");
    let name = accepted["student"]["nama"].as_str().unwrap_or("");
    assert!(
        !name.is_empty(),
        "accepted result must name a student: {accepted}"
    );
}

async fn next_json<S>(socket: &mut S) -> Value
where
    S: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(t))) => {
                return serde_json::from_str(&t).expect("server sent valid JSON")
            }
            Some(Ok(_)) => continue,
            other => panic!("websocket closed unexpectedly: {other:?}"),
        }
    }
}
