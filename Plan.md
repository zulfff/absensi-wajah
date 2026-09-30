# Plan: Sistem Absensi Siswa Berbasis Pengenalan Wajah

Backend Rust, kiosk cukup browser + kamera + layar + internet, enroll wajah lewat web admin.

---

## 1. Tujuan & Batasan

**Tujuan**
- Siswa absen dengan menghadap kamera kiosk, tanpa kartu / input manual.
- Admin mendaftarkan (enroll) wajah siswa lewat dashboard web.
- Kiosk = perangkat "bodoh": hanya browser fullscreen + kamera + layar. Semua proses berat ada di server.

**Target keamanan (dibaca jujur)**
- "Nol kesalahan" secara absolut tidak mungkin dicapai oleh sistem ML apa pun. Yang bisa dicapai: **False Accept Rate (FAR) sangat rendah** (target ≤ 1 dalam 100.000 percobaan) dengan cara:
  1. threshold ketat,
  2. konfirmasi multi-frame,
  3. liveness / anti-spoof,
  4. lebih baik **menolak** (minta ulang) daripada salah menerima.
- Prinsip: **fail-closed**. Ragu = tidak absen + minta coba lagi / fallback manual oleh guru.
- Absen salah orang (false positive) jauh lebih buruk daripada gagal absen (false negative).

**Non-goals (v1)**
- Tidak ada aplikasi native di kiosk.
- Tidak ada training model dari nol. Pakai model pretrained.

---

## 2. Konsep Penting: "Training" = Enrollment

Tidak perlu melatih ulang model tiap ada siswa baru. Cara kerjanya:

1. Model pretrained (ArcFace) mengubah wajah jadi **embedding** (vektor 512 dimensi).
2. Saat admin "training" siswa, sistem menyimpan beberapa embedding wajah siswa itu.
3. Saat absen, embedding wajah live dibandingkan (cosine similarity) dengan semua embedding tersimpan.

Keuntungan: enroll instan, tidak butuh GPU untuk training, tidak ada risiko model "lupa" siswa lama.

---

## 3. Arsitektur

```
┌──────────────┐   HTTPS/WSS    ┌───────────────────────────────┐
│ Kiosk        │ ─────────────► │ Server (Rust, axum)           │
│ Browser      │  frame/crop    │  ├─ API & WebSocket           │
│ kamera+layar │ ◄───────────── │  ├─ Face pipeline (ONNX/ort)  │
└──────────────┘   hasil absen  │  │   detect→align→liveness→   │
                                │  │   embed→match              │
┌──────────────┐   HTTPS        │  ├─ PostgreSQL (+pgvector)    │
│ Admin Web    │ ─────────────► │  └─ Object storage (foto)     │
└──────────────┘                └───────────────────────────────┘
```

**Stack** — lihat `Cargo.toml` untuk versi terpin.

| Layer | Pilihan |
|---|---|
| Web framework | `axum` + `tokio` |
| Inference | `ort` (ONNX Runtime binding untuk Rust) |
| Deteksi wajah | SCRFD / RetinaFace (ONNX) |
| Embedding | ArcFace (buffalo_l / w600k_r50) |
| Anti-spoof | MiniFASNet (Silent-Face-Anti-Spoofing) + challenge |
| DB | PostgreSQL + `pgvector`, akses via `sqlx` |
| Auth | Argon2 + JWT (admin), device token (kiosk) |
| Frontend | Vue 3 atau Leptos (Phase 2/3, belum dibuat) |
| Migrasi DB | `sqlx migrate` |
| Logging | `tracing` |
| Deploy | Docker + Caddy (HTTPS) |

**Keputusan arsitektur: inference di server, bukan di browser.** Kiosk bisa
perangkat lemah; server satu tempat = model konsisten dan keputusan absen tidak
bisa dimanipulasi dari sisi client.

---

## 4. Alur Enrollment (Admin, Web) — 1..8
Lihat implementasi: `crates/server/src/routes/enroll.rs`,
`crates/domain/src/enroll.rs`, `crates/domain/src/quality.rs`.

1. Admin login → pilih siswa → **Daftarkan Wajah**.
2. Capture ±10–15 frame dengan arahan pose.
3. **Quality gate** per frame (`domain::quality`): tepat 1 wajah, ukuran minimum,
   tidak blur (variance of Laplacian), pencahayaan, pose, mata terbuka.
4. **Anti-duplikat** (`enroll::evaluate_enrollment`): embedding baru vs seluruh
   siswa lain; terlalu mirip → diblokir.
5. **Konsistensi internal**: frame outlier dibuang otomatis.
6. **Cek pemisahan**: margin ke siswa terdekat; terlalu kecil → "berisiko tertukar".
7. Simpan embedding + foto referensi; status pending → **Aktifkan**.
8. Re-enroll / tambah sampel kapan saja.

---

## 5. Alur Absensi (Kiosk) — 1..8
Lihat implementasi: `crates/server/src/routes/ws.rs`,
`crates/server/src/attendance.rs`, `crates/domain/src/decision.rs`.

1. Kiosk buka `/kiosk` fullscreen, login **device token**.
2. Browser kirim frame via WebSocket, ±3–5 fps.
3. Server per frame: deteksi → quality gate → liveness → embedding → nearest
   neighbor (`gallery`).
4. **Keputusan (fail-closed)**, semua syarat AND: similarity ≥ `T_ACCEPT`, margin
   ≥ `T_MARGIN`, konsensus N frame, liveness lolos.
5. Lolos → catat absensi, tampil nama + jam + **bunyi** (chime WebAudio di kiosk;
   nada berbeda untuk absen baru vs. sudah absen).
6. Tidak → tampil "Tidak dikenali", **tanpa** menebak nama.
7. **Cooldown**: tidak dicatat dobel dalam jendela waktu (`domain::cooldown`).
8. Semua percobaan dicatat (`attempts`) untuk audit & tuning.

---

## 6. Strategi Menekan False Positive

| Lapisan | Mekanisme | Lokasi |
|---|---|---|
| Threshold | `T_ACCEPT` dari data nyata sekolah, FAR ≤ 1e-5 | `eval/thresholds.py`, `.env` |
| Margin | Tolak jika top-1 dan top-2 terlalu dekat | `decision.rs` |
| Multi-frame | Konsistensi N frame | `decision.rs` |
| Liveness | Blokir foto/layar/video | `LivenessChecker` |
| Open-set | Wajah tak terdaftar ditolak, tidak dipaksa cocok | `decision.rs` |
| Enroll ketat | Quality gate, duplikat, margin | `quality.rs`, `enroll.rs` |
| Fallback manusia | Guru absen manual | `POST /api/attendance` (`mark_attendance_manual`) status `manual` |
| Koreksi | Admin batalkan absen salah | `attendance_repo::correct` (`POST /api/attendance/{id}/correct`) |

Threshold **tidak boleh** ditebak.

---

## 7. Model Data (PostgreSQL)

Lihat `crates/db/migrations/0001_initial.sql`: `users`, `classes`, `students`,
`face_templates` (vector(512)), `devices`, `attendance`, `attempts`,
`audit_log`. Index HNSW pada `face_templates.embedding`; untuk sekolah cukup
brute-force in-memory (`crates/server/src/gallery.rs`).

---

## 8. API — lihat tabel di `README.md` dan `crates/server/src/routes/`.

---

## 9. Privasi & Keamanan

- Persetujuan wali dicatat (`students.consent_granted`); API menolak enroll tanpa itu.
- Biometrik yang disimpan = **embedding** (kolom `vector(512)`) di PostgreSQL.
  **Foto referensi tidak disimpan sama sekali** (`source_image_ref` ada di skema
  tetapi jalur penyimpanan aktif tidak pernah menulisnya), jadi tidak ada foto
  biometrik yang perlu dienkripsi at-rest. Enkripsi volume DB adalah tanggung
  jawab operator (mis. LUKS pada host).
- HTTPS di semua jalur; device token di-hash (SHA-256) di DB.
- Rate limiting login (per-IP via `X-Forwarded-For`) & kiosk (`crates/server/src/rate_limit.rs`).
- Kebijakan retensi: `DELETE /api/students/{id}/face` (hapus semua template + audit).
- Audit log setiap akses/perubahan data biometrik (`db::audit`).
- Frame kamera tidak pernah dikirim ke pihak ketiga.

---

## 10. Testing & Evaluasi

1. Unit test: quality gate, keputusan, cooldown, parsing — **53 tes di domain**
   (49 unit + 4 regresi); seluruh workspace **118 tes**.
2. Integration test: enroll → aktivasi → absen — `crates/server/tests/kiosk_flow.rs`
   (jalan penuh bila `ABSENSI_TEST_*` diisi; di-skip bila tidak ada server).
3. Dataset evaluasi sekolah sendiri (dengan consent) — `eval/data/`.
4. Ukur FAR, FRR, ROC — `eval/thresholds.py`.
5. Tes serangan: foto cetak, foto HP, video replay, kembar — **kode liveness sudah
   menolak spoof; pengujian fisik butuh sampel nyata (belum)**.
6. Load test: beberapa kiosk bersamaan — **belum**.
7. Pilot 1 kelas dengan absen manual paralel — **belum**.
8. Monitoring produksi: `GET /api/monitoring/summary`.

Kriteria lulus: 0 false accept pada set uji internal + pilot, FRR < 5% dengan retry.

---

## 11. Roadmap

| Fase | Isi | Status |
|---|---|---|
| 0 | Workspace, DB, PoC ONNX, HTTPS | **selesai** — SCRFD + ArcFace + alignment terverifikasi pada foto asli |
| 1 | Backend inti | **selesai**, terverifikasi e2e |
| 2 | Web admin + enroll feedback | **selesai** (8 view) |
| 3 | Kiosk web | **selesai**, terverifikasi e2e (termasuk bunyi konfirmasi) |
| 4 | Liveness + multi-frame + cooldown | **selesai** — MiniFASNet terpasang & terverifikasi (foto asli 0.99 live) |
| 5 | Evaluasi nyata, tuning | **alat selesai** (`eval/thresholds.py`); angkanya menunggu dataset sekolah |
| 6 | Hardening privasi/keamanan | **selesai untuk kode**; enkripsi foto tidak berlaku karena foto tidak disimpan |
| 7 | Pilot 1 kelas | **belum** — butuh manusia & jadwal sekolah, tidak bisa dikerjakan lewat kode |

**Yang benar-benar tersisa (butuh manusia / perangkat, bukan kode):**
- Pilot 1 kelas dengan absen manual paralel (Fase 7).
- Menjalankan uji serangan fisik: foto cetak, layar HP, video replay (kode liveness
  sudah menolak input spoof; angka FAR pada serangan fisik butuh sampel nyata).
- Mengumpulkan dataset sekolah untuk mengisi `eval/data/scores.csv` dan menetapkan
  `T_ACCEPT`/`T_MARGIN` dari data (bukan tebakan).
- Load test beberapa kiosk bersamaan.

Semua item lain sudah dikerjakan dan diverifikasi lewat tes otomatis + inferensi
model nyata.

---

## 12. Risiko & Mitigasi
Lihat tabel di README. Poin penting: ekspektasi "0 error" harus dijelaskan ke
stakeholder — targetnya FAR sangat rendah + fallback manusia, bukan kesempurnaan.

---

## 13. Struktur Repo — lihat `README.md`.

---

## 14. Langkah Pertama

1. Cargo workspace + PostgreSQL + pgvector — **selesai**.
2. PoC load SCRFD + ArcFace via `ort`, hitung cosine — **selesai & terverifikasi**
   (Obama vs Obama 0.98, Obama vs Biden -0.07). Lihat
   `crates/face-core/examples/poc_onnx.rs`.
3. Halaman kiosk minimal: `getUserMedia` → frame → server — **backend selesai**
   (WebSocket); UI belum.
4. Domain/HTTPS untuk kamera browser — `docker/Caddyfile` siap.
