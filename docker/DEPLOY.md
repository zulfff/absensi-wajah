# Deployment — Absensi Wajah di tkjt3yapera.my.id

Dua domain publik, TLS otomatis lewat Caddy yang sudah menjalankan seluruh
ekosistem `tkjt3yapera.my.id`:

| Domain | Untuk | Route |
|---|---|---|
| `panel.tkjt3yapera.my.id` | Dashboard admin | SPA + `/api/*` + `/ws/*` → `absensi-server` |
| `absensi.tkjt3yapera.my.id` | Kiosk | SPA (langsung ke `/#/kiosk`) + `/api/*` + `/ws/*` |

Caddy (`deploy-caddy-1`, image `caddy:2-alpine`) sudah memegang port 80/443 dan
menerbitkan sertifikat Let's Encrypt. Stack absensi **menempel** ke network
`fortress-net` miliknya; tidak ada proxy kedua yang berebut port.

## Arsitektur di server ini

```
Internet :443
   │
   ▼
deploy-caddy-1 (TLS, *.tkjt3yapera.my.id)
   ├─ panel.*   → absensi-web:80   (SPA admin),  /api,/ws → absensi-server:8080
   └─ absensi.* → absensi-web:80   (SPA kiosk),  /api,/ws → absensi-server:8080

fortress-net:
   absensi-web-1     (Caddy serve SPA statis)
   absensi-server-1  (axum: API + WebSocket)
   absensi-db-1      (Postgres 16 + pgvector)
```

## Deploy / update

```bash
cd /home/zulfff/pameran/absensi-wajah

# Build + start (detached; build Rust release bisa ~3 menit)
sudo docker compose -f docker/docker-compose.deploy.yml --env-file .env up -d --build

# Lihat status & log
sudo docker ps --format '{{.Names}}\t{{.Status}}' | grep absensi
sudo docker logs -f absensi-absensi-server-1
```

Setelah mengubah `docker/Caddyfile` (di FortressWAF) atau menambah domain,
reload Caddy:

```bash
sudo docker exec deploy-caddy-1 caddy validate --config /etc/caddy/Caddyfile
sudo docker exec deploy-caddy-1 caddy reload  --config /etc/caddy/Caddyfile
```

## Rahasia

`.env` (gitignored, mode 600) berisi `JWT_SECRET`, `ABSENSI_DB_PASSWORD`, dan
`BOOTSTRAP_ADMIN_PASSWORD`. Sandi admin bootstrap dibuat sekali saat DB kosong —
**catat dan ganti setelah login pertama.** Kalau `.env` hilang, sandi admin bisa
di-reset dengan menjalankan ulang bootstrap di DB yang baru (volume
`absensi_absensi-db`).

## DNS yang dibutuhkan

Kedua hostname harus **A record (DNS only / grey cloud)** ke `103.253.147.149`.
Kalau diproksikan Cloudflare (orange cloud), challenge Let's Encrypt dijawab
Cloudflare dan penerbitan sertifikat gagal — persis catatan di Caddyfile
FortressWAF.

```
panel.tkjt3yapera.my.id     A   103.253.147.149   (DNS only)
absensi.tkjt3yapera.my.id   A   103.253.147.149   (DNS only)
```

## Pipeline ONNX asli — SUDAH AKTIF

Deploy produksi sekarang menjalankan pipeline **asli**: SCRFD (`det_10g`) →
landmark alignment 5-titik → ArcFace (`w600k_r50`) → MiniFASNet liveness.

Setup yang membuat ini jalan:
- Model + runtime di `models/` (di-mount read-only ke `/app/models`):
  `det_10g.onnx`, `w600k_r50.onnx`, `minifasnet.onnx`, `libonnxruntime.so`.
- `docker/docker-compose.deploy.yml`: `FEATURES: "onnx"` +
  `USE_DETERMINISTIC_PIPELINE: false`.
- `docker/Dockerfile` runtime stage memasang `libgomp1` + `libstdc++6` (dibutuhkan
  `libonnxruntime.so`; ORT CPU pakai OpenMP).
- `ORT_DYLIB_PATH=/app/models/libonnxruntime.so`.

**Terverifikasi live:** enroll wajah asli → kiosk mengenali via `wss://`
(similarity 1.000, liveness 0.988); orang berbeda yang tidak terdaftar
**ditolak** (top1 −0.054, `rejected_unknown`), tidak dicatat hadir.

### Mengganti / memperbarui model

```bash
# taruh file baru di models/, lalu rebuild:
./scripts/deploy.sh server
```

### Kembali ke mode deterministik (tanpa model)

Set `USE_DETERMINISTIC_PIPELINE=true` di `.env`, ubah `FEATURES: "onnx"` menjadi
`""` di compose, lalu `./scripts/deploy.sh server`. Berguna untuk debugging
lapisan HTTP/DB tanpa model.

## Verifikasi

```bash
# TLS + health
curl -s -o /dev/null -w "%{http_code} ssl=%{ssl_verify_result}\n" https://panel.tkjt3yapera.my.id/
curl -s https://panel.tkjt3yapera.my.id/api/health

# Alur kiosk penuh lewat WSS (butuh device token + gambar frame)
cd ~/pameran/absensi-wajah
ABSENSI_TEST_URL=https://panel.tkjt3yapera.my.id \
ABSENSI_TEST_DEVICE_TOKEN='kiosk_...' \
ABSENSI_TEST_FRAME_FILE=@/tmp/frame.jpg \
  cargo test -p absensi-server --test kiosk_flow -- --nocapture
```

## Backup

DB ada di volume Docker `absensi_absensi-db`. Backup:

```bash
sudo docker exec absensi-db-1 pg_dump -U absensi absensi | gzip > absensi-$(date +%F).sql.gz
```
