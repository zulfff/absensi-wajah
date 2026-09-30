# Absensi Wajah — Sistem Absensi Siswa Berbasis Pengenalan Wajah

Absensi siswa dengan menghadap kamera kiosk. Kiosk = browser + kamera + layar.
Semua proses berat (deteksi, embedding, liveness, pencocokan) berjalan di server
Rust. Enrollment wajah dilakukan admin lewat dashboard web.

> **Status jujur (baca ini dulu).** Yang sudah jalan dan terverifikasi
> end-to-end: workspace Rust, skema DB + migrasi, auth admin (Argon2 + JWT) dan
> device token, CRUD siswa, pipeline enroll (quality gate, anti-duplikat, margin
> antar-siswa), logika keputusan (threshold + margin + konsensus multi-frame +
> cooldown + open-set reject), endpoint absensi, WebSocket kiosk, audit log,
> **pipeline ONNX asli (SCRFD + ArcFace) dengan alignment landmark 5-titik**, dan
> **evaluasi threshold dari data nyata**.
>
> Pipeline model nyata sudah **diverifikasi** pada gambar wajah asli: Obama vs
> dirinya = 0.98, Obama vs Biden = -0.07, Biden vs Lena = 0.07 — margin
> genuine/impostor bersih ~0.9. Ini membuktikan seluruh rantai deteksi →
> alignment → embedding → cosine berfungsi. Liveness MiniFASNet juga
> terverifikasi (foto asli ~0.99 = live; citra warna rata ~0.006 = spoof).
> Server dengan pipeline ONNX asli sudah diuji end-to-end: enroll Barack →
> dikenali (accept) → Biden yang tidak terdaftar (reject).
>
> Yang **belum**: suite uji serangan fisik (foto cetak/layar) belum dijalankan;
> pilot 1 kelas belum. Verifikasi visual UI belum dilakukan oleh manusia (skor
> otomatis anti-slop/token/a11y = 100/A, tetapi "gate lulus" bukan bukti enak
> dilihat). Foto referensi **tidak disimpan** di server (hanya embedding), jadi
> tidak ada foto biometrik yang perlu dienkripsi. Selama model tidak tersedia,
> server berjalan dengan pipeline **deterministik tanpa model**
> (`USE_DETERMINISTIC_PIPELINE=true`).

---

## Kenapa desainnya seperti ini

"Training" di sistem ini = **enrollment**. Model ArcFace pretrained mengubah
wajah menjadi embedding 512-D; enroll menyimpan beberapa embedding per siswa;
absen membandingkan embedding live dengan yang tersimpan (cosine similarity).
Enroll instan, tak butuh GPU, tak ada risiko model lupa siswa lama.

Prinsip yang mengikat seluruh sistem: **fail-closed**. Ragu = tidak absen,
minta ulang atau fallback manual guru. Salah absen orang lain jauh lebih buruk
daripada gagal absen. Semua syarat penerimaan adalah AND — lihat
`crates/domain/src/decision.rs`, satu-satunya tempat keputusan dibuat.

"Tanpa kesalahan" absolut tidak mungkin dicapai ML apa pun. Targetnya FAR sangat
rendah (≤ 1e-5), diukur dari data sekolah sendiri, bukan ditebak.

---

## Struktur

```
absensi-wajah/
├─ Cargo.toml                 workspace
├─ crates/
│  ├─ domain/                 logika murni: embedding, quality gate, keputusan,
│  │                          konsensus, cooldown, enroll. Tanpa I/O, 53 tes.
│  ├─ face-core/              pipeline deteksi→align→liveness→embed (SCRFD,
│  │                          ArcFace, MiniFASNet via `ort`; deterministik utk dev)
│  ├─ db/                     sqlx: pool, migrasi, repo, Argon2, pgvector
│  └─ server/                 axum: API admin, WebSocket kiosk, auth, state
├─ web/                       Vue 3 + Vite SPA: admin + kiosk
│  ├─ src/views/              Login, Dashboard, Students, Enroll, Devices,
│  │                          Attendance, Kiosk
│  ├─ src/styles/tokens.css   token spine 3 lapis, light + dark
│  └─ src/lib/                api client, camera helper, types
├─ docker/                    Dockerfile (server), web.Dockerfile, compose, Caddy
├─ scripts/                   download_models.sh, smoke.sh
├─ eval/                      thresholds.py + data/ (git-ignored)
└─ Plan.md                    rencana lengkap
```

---

## Menjalankan (development)

Butuh: Docker (untuk DB), Rust, Node 20+.

```bash
# 1. PostgreSQL + pgvector
docker compose -f docker/docker-compose.yml up -d db

# 2. Server. Development tanpa model:
DATABASE_URL="postgres://absensi:absensi@localhost:5432/absensi" \
  USE_DETERMINISTIC_PIPELINE=true cargo run -p absensi-server

# 3. Frontend (Vite dev server, proxy /api dan /ws ke server)
cd web && npm install && npm run dev
```

Buka `http://localhost:5173/` untuk admin, `http://localhost:5173/#/kiosk`
untuk kiosk. Admin bootstrap `admin`/`admin` dibuat saat tabel user kosong —
**ganti segera.**

Vite mem-proxy `/api` dan `/ws` ke backend (default `127.0.0.1:8090`; ubah di
`web/vite.config.ts` bila `BIND_ADDR` berbeda). Karena kamera browser butuh
secure context, `http://localhost` cukup untuk dev; untuk perangkat kiosk lain
di jaringan, pakai HTTPS (lihat `docker/Caddyfile`).

---

## Deployment (produksi, LIVE)

Sudah berjalan di dua domain HTTPS:

| Domain | Untuk |
|---|---|
| **`panel.tkjt3yapera.my.id`** | Dashboard admin |
| **`absensi.tkjt3yapera.my.id`** | Kiosk (langsung ke `/#/kiosk`) |

TLS otomatis via Caddy `deploy-caddy-1` yang sudah memegang port 80/443 untuk
seluruh `*.tkjt3yapera.my.id`. Stack absensi menempel ke network `fortress-net`
(`docker/docker-compose.deploy.yml`). Rincian lengkap → **`docker/DEPLOY.md`**.

```bash
cp .env.deploy.example .env       # isi JWT_SECRET + sandi (openssl rand -hex 32)
./scripts/deploy.sh up            # build + start
./scripts/deploy.sh status        # cek container
./scripts/deploy.sh logs absensi-absensi-server-1
```

Rahasia ada di `.env` (gitignored, mode 600). Sandi admin bootstrap dibuat saat
DB kosong — **catat dan ganti setelah login pertama.**

> Catatan: deploy menjalankan pipeline **ONNX asli** — SCRFD (`det_10g`) +
> ArcFace (`w600k_r50`) + MiniFASNet. Model + `libonnxruntime.so` di `models/`
> (di-mount read-only). Terverifikasi live: wajah terdaftar dikenali
> (similarity 1.000, liveness 0.988), orang berbeda ditolak (top1 −0.054).
> Detail: `docker/DEPLOY.md`.

---

## Deploy produksi (tkjt3yapera.my.id)

Aplikasi berjalan di belakang Caddy FortressWAF yang sudah memegang port 80/443
dan menerbitkan TLS otomatis. Stack absensi menempel ke network `fortress-net`.

| Domain | Untuk |
|---|---|
| **https://panel.tkjt3yapera.my.id** | Dashboard admin (+ `/api`, `/ws`) |
| **https://absensi.tkjt3yapera.my.id** | Kiosk (langsung ke layar kiosk) |

```bash
./scripts/deploy.sh up      # build + start seluruh stack
./scripts/deploy.sh status  # cek container
./scripts/deploy.sh logs    # ikuti log
```

Panduan lengkap (DNS, rahasia, model ONNX, backup): **`docker/DEPLOY.md`**.

Kiosk domain otomatis membuka `/#/kiosk`; panel domain membuka dashboard. Rute
ini ditentukan di `web/src/main.ts` (`isKioskHost`).

## Menjalankan dengan model asli (produksi)

```bash
./scripts/download_models.sh    # det_10g.onnx + w600k_r50.onnx (MiniFASNet manual)
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so.1.29   # ONNX Runtime 1.28+
# set USE_DETERMINISTIC_PIPELINE=false, FACE_*_MODEL, JWT_SECRET yang kuat
cargo run -p absensi-server --features onnx
```

Liveness **wajib** di produksi: server menolak start pipeline asli bila
`FACE_LIVENESS_MODEL` tidak ada.

### Verifikasi PoC (2 foto → cosine)

```bash
ORT_DYLIB_PATH=... FACE_DETECTOR_MODEL=models/det_10g.onnx \
FACE_EMBEDDER_MODEL=models/w600k_r50.onnx \
  cargo run -p face-core --features onnx --example poc_onnx -- fotoA.jpg fotoB.jpg
```

---

## Verifikasi

```bash
cargo test --workspace                 # 118 tes (domain 53, db, server, face-core, gallery, auth)
cargo clippy --workspace --all-targets # bersih
./scripts/smoke.sh                     # end-to-end: enroll -> activate -> WS accept
python3 eval/thresholds.py eval/data/scores.csv --target-far 1e-5

cd web && npm run typecheck && npm run build   # SPA: typecheck + bundle
```

Skor otomatis UI (ui-craft): anti-slop 100, token-discipline 100, a11y 100 —
**A**. Ini mengukur konsistensi token dan aksesibilitas objektif, bukan selera;
verifikasi visual manusia tetap disarankan.

`smoke.sh` memerlukan server berjalan dan kontainer DB bernama `docker-db-1`.

Kamera browser (`getUserMedia`) **wajib HTTPS** atau `http://localhost`. Untuk
kiosk di perangkat lain, domain + sertifikat harus siap lebih dulu — lihat
`docker/Caddyfile`.

---

## API (ringkas)

| Endpoint | Auth | Fungsi |
|---|---|---|
| `POST /api/auth/login` | - | login, dapat JWT |
| `GET /api/auth/me` | token | identitas + peran (dibaca dari DB) |
| `GET/POST /api/students`, `GET/PUT/DELETE /api/students/{id}` | token | CRUD siswa |
| `POST /api/students/{id}/consent` | **admin** | catat persetujuan wali (prasyarat enroll) |
| `POST /api/students/{id}/enroll/frame` | **admin** | assess 1 frame, feedback kualitas |
| `POST /api/students/{id}/enroll/commit` | **admin** | cek konsistensi+duplikat+margin, simpan |
| `POST /api/students/{id}/activate` | **admin** | aktifkan template (setelah review) |
| `POST /api/students/{id}/deactivate` | **admin** | nonaktifkan template (siswa berhenti dikenali) |
| `DELETE /api/students/{id}/face` | **admin** | hapus permanen data biometrik |
| `GET /api/users`, `POST /api/users`, `PUT/DELETE /api/users/{id}` | **admin** | kelola akun (admin/guru) |
| `GET/POST /api/devices`, `POST /api/devices/{id}/revoke`, `DELETE /api/devices/{id}` | token / **admin** | kelola kiosk |
| `GET /api/attendance` | token | lihat absensi per tanggal |
| `POST /api/attendance` | token | **absen manual** oleh guru (fallback) |
| `POST /api/attendance/{id}/correct` | **admin** | koreksi/batalkan absensi |
| `GET /api/monitoring/summary` | token | rasio ditolak, rerata skor (24 jam) |
| `POST /api/gallery/reload` | **admin** | muat ulang cache galeri dari DB |
| `GET /api/kiosk/info` | device token | info perangkat + jumlah galeri |
| `GET /api/health` | - | health check |
| `GET /ws/kiosk` | device token (dalam pesan `hello`) | frame masuk, hasil keluar |

**Dua tingkat otorisasi.** `token` = admin ATAU guru boleh baca; **admin** =
hanya role admin (menulis/mengubah/menghapus). Setiap request memuat ulang baris
user dari DB, jadi akun yang dinonaktifkan/dihapus langsung berhenti berlaku —
tidak menunggu token kedaluwarsa.

Device token tidak bisa dikirim sebagai header oleh `WebSocket` browser, jadi
dikirim dalam pesan `hello` pertama dan diverifikasi sebelum frame apa pun
diproses.

---

## Privasi & keamanan (ringkas)

- Persetujuan wali dicatat (`students.consent_granted`) **sebelum** enroll; API
  menolak enroll tanpa itu.
- Password admin Argon2id; device token SHA-256 (high-entropy, bukan KDF lambat).
- Rate limit login per IP; lockout setelah gagal berulang.
- Audit log untuk setiap aksi biometrik; absensi bisa dikoreksi, tidak dihapus.
- Retensi: `DELETE /api/students/{id}/face` menghapus semua template.
- Frame kamera tidak pernah dikirim ke pihak ketiga.

---

## Roadmap (dari Plan.md)

| Fase | Isi | Status |
|---|---|---|
| 0 | Workspace, DB, PoC ONNX, HTTPS | **selesai** — SCRFD + ArcFace + alignment terverifikasi |
| 1 | Backend inti: DB, auth, CRUD, pipeline | **selesai**, terverifikasi e2e |
| 2 | Web admin + enroll feedback realtime | **selesai** — 8 view, score 100/A |
| 3 | Kiosk web (kamera, WebSocket, device token) | **selesai**, terverifikasi e2e |
| 4 | Liveness + aturan multi-frame + cooldown | **selesai** — MiniFASNet terverifikasi |
| 5 | Evaluasi data nyata, tuning threshold, tes serangan | alat siap (`eval/thresholds.py`); uji serangan fisik belum |
| 6 | Hardening privasi/keamanan, audit, dokumentasi | **selesai untuk kode** — audit, retensi, rate limit, batas decode; foto tidak disimpan |
| 7 | Pilot 1 kelas → rollout | belum |
