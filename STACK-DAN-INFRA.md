# Absensi Wajah — Stack & Infrastruktur

Laporan teknis untuk Pak Tahir. Semua di bawah ini diambil dari kode di repository, bukan perkiraan.

**Project:** Absensi Wajah — Sistem Absensi Siswa Berbasis Pengenalan Wajah
**Pengembang:** [nama lengkap], jurusan TKJT

---

## 1. Gambaran arsitektur

Sistem memakai arsitektur **client–server**:

- **Kiosk** (browser + kamera) adalah klien. Ia mengirim frame kamera ke server dan menerima hasil.
- **Server** menjalankan seluruh proses berat: deteksi wajah, penyelarasan, liveness, pembuatan embedding, dan pencocokan.
- **Database** terpusat menyimpan data siswa, template wajah, absensi, perangkat, dan audit.

Kiosk tidak menyimpan apa pun secara lokal. Kalau kiosk mati atau diganti, cukup daftarkan perangkat baru dari dashboard.

---

## 2. Stack aplikasi

### Sisi server (Rust)

| Bagian | Teknologi | Versi |
|---|---|---|
| Bahasa | Rust (edition 2021) | 1.82 |
| Runtime async | Tokio | 1.53 |
| Framework web | Axum | 0.8 |
| Middleware | Tower / tower-http (CORS, tracing, body limit) | 0.5 / 0.7 |
| Database driver | sqlx (Postgres, TLS rustls) | 0.9 |
| Pencarian vektor | pgvector | 0.4 |
| Autentikasi | Argon2id (sandi), jsonwebtoken (JWT) | 0.6 / 11 |
| Hashing token | SHA-256 (perangkat) | 0.10 |
| Inferensi ML | ONNX Runtime via `ort` (dimuat dinamis) | 2.0-rc |
| Serialisasi | serde / serde_json | 1 |
| Logging | tracing + tracing-subscriber | 0.1 / 0.3 |

Struktur workspace terbagi 4 crate, masing-masing satu tanggung jawab:

| Crate | Isi |
|---|---|
| `domain` | Logika murni: aturan keputusan, konsensus, cooldown, quality gate. Tanpa I/O. |
| `face-core` | Pipeline wajah: deteksi (SCRFD), penyelarasan landmark 5 titik, liveness (MiniFASNet), embedding (ArcFace). |
| `db` | Koneksi pool, migrasi, repository, Argon2, pgvector. |
| `server` | API admin (axum), WebSocket kiosk, autentikasi, state aplikasi. |

### Model AI yang dipakai

| Fungsi | Model | Format |
|---|---|---|
| Deteksi wajah | SCRFD (`det_10g`) | ONNX |
| Embedding wajah | ArcFace (`w600k_r50`), 512 dimensi | ONNX |
| Anti-spoof (liveness) | MiniFASNet | ONNX |

Semua model dijalankan **di server**, bukan di browser atau perangkat kiosk.

### Sisi antarmuka (Web)

| Bagian | Teknologi |
|---|---|
| Framework | Vue 3 |
| Build tool | Vite |
| Bahasa | TypeScript |
| Gaya | CSS token 3 lapis (light + dark) |
| Komunikasi | REST untuk admin, WebSocket untuk kiosk |

Terdapat 8 halaman: Login, Dashboard, Students, Enroll, Devices, Attendance, Users, dan Kiosk.

---

## 3. Infrastruktur & deployment

### Topologi

```
Kamera → Browser Kiosk ──HTTPS/WSS──► Caddy (reverse proxy + TLS)
                                          │
                          ┌───────────────┴───────────────┐
                          ▼                               ▼
                   absensi-web                     absensi-server
                  (SPA statis)                  (Rust, ONNX runtime)
                                                          │
                                                          ▼
                                                    PostgreSQL 16
                                                    + pgvector
```

Semua container berada dalam satu jaringan Docker bernama **`fortress-net`**.

### Komponen infrastruktur

| Komponen | Peran | Detail |
|---|---|---|
| **Caddy** | Reverse proxy + TLS otomatis | Memegang port 80/443, menerbitkan sertifikat Let's Encrypt otomatis untuk `*.tkjt3yapera.my.id` |
| **Docker** | Containerisasi | Server + web + database berjalan sebagai container |
| **PostgreSQL 16** | Database terpusat | Image `pgvector/pgvector:pg16`, volume persisten |
| **pgvector** | Pencarian kemiripan vektor | Menyimpan embedding wajah 512 dimensi |
| **Docker Compose** | Orkestrasi | `docker/docker-compose.deploy.yml` |

### Nama domain (HTTPS, sudah live)

| Domain | Fungsi |
|---|---|
| `panel.tkjt3yapera.my.id` | Dashboard admin |
| `absensi.tkjt3yapera.my.id` | Halaman kiosk |

TLS wajib karena kamera browser (`getUserMedia`) hanya berjalan di secure context (HTTPS).

### Keamanan jaringan yang diterapkan

- **TLS otomatis** oleh Caddy, plus header keamanan: HSTS, `X-Content-Type-Options`, `X-Frame-Options: DENY`, `Referrer-Policy`, dan `Permissions-Policy` yang membatasi akses kamera hanya untuk halaman kiosk.
- **JWT** untuk sesi admin/guru; **device token** untuk kiosk. Token perangkat dikirim di pesan `hello` pertama lewat WebSocket, lalu diverifikasi sebelum frame apa pun diproses.
- **Rate limit login per IP** + lockout setelah gagal berulang.
- **Batas ukuran body 6 MB** per frame di sisi server.
- **CORS tidak dibuka ke `Any`** — hanya origin yang dikonfigurasi (dev server).
- **Isolasi container**: port 80/443 hanya dipegang Caddy; server dan web tidak membuka port ke luar, hanya dijangkau lewat jaringan internal Docker.

### Protokol kiosk ↔ server (WebSocket)

```
Kiosk  → server : {"type":"hello","device_token":"kiosk_..."}   (sekali, pertama)
                  {"type":"frame","image":"<base64 jpeg>"}        (~3–5 fps)
                  {"type":"reset"}

server → kiosk  : {"type":"ready",...}
                  {"type":"result","kind":"accepted|rejected|continue",...}
                  {"type":"error","message":"..."}
```

Karena WebSocket di browser tidak bisa mengirim header khusus, device token dikirim di dalam pesan pertama dan diverifikasi lebih dulu.

---

## 4. Cara menjalankan

**Produksi (deployment):**

```bash
cp .env.deploy.example .env      # isi JWT_SECRET + sandi
./scripts/deploy.sh up           # build + start
./scripts/deploy.sh status       # cek container
```

**Pengembangan (lokal, tanpa model):**

```bash
docker compose -f docker/docker-compose.yml up -d db
DATABASE_URL="postgres://absensi:absensi@localhost:5432/absensi" \
  USE_DETERMINISTIC_PIPELINE=true cargo run -p absensi-server
cd web && npm install && npm run dev
```

---

## 5. Hasil pengujian

- Pipeline model nyata sudah diverifikasi pada wajah asli: orang terdaftar
  dikenali (kemiripan 0,98–0,99), orang berbeda ditolak (−0,07).
- Liveness (MiniFASNet) aktif dan berfungsi: wajah asli dinilai "hidup" (~0,99),
  citra warna rata dinilai lebih rendah.
- 118 tes otomatis lolos di seluruh workspace.

## 6. Yang belum selesai

- Pengujian serangan fisik (foto cetak / tampilan layar / video replay) dengan
  sampel nyata belum dijalankan; kode liveness sudah menolak input spoof.
- Pilot 1 kelas dengan absen manual paralel belum dijalankan.
- **Foto referensi tidak disimpan** di server (hanya embedding/vektor), jadi
  tidak ada foto biometrik yang perlu dienkripsi. Enkripsi volume database
  adalah tanggung jawab operator.
