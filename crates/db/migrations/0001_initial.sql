-- Absensi Wajah — initial schema (plan Section 7).
-- Enables pgvector for 512-D face embeddings.

CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- ---------- Users (admin / guru) ----------
CREATE TABLE users (
    id            UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL CHECK (role IN ('admin', 'guru')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    disabled      BOOLEAN NOT NULL DEFAULT FALSE
);

-- ---------- Classes ----------
CREATE TABLE classes (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    nama       TEXT NOT NULL,
    tingkat    TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ---------- Students ----------
CREATE TABLE students (
    id                UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    nis               TEXT NOT NULL UNIQUE,
    nama              TEXT NOT NULL,
    kelas_id          UUID REFERENCES classes(id) ON DELETE SET NULL,
    status            TEXT NOT NULL DEFAULT 'active'
                      CHECK (status IN ('active', 'inactive', 'graduated')),
    -- Biometric consent (plan Section 9): required before enrollment.
    consent_granted   BOOLEAN NOT NULL DEFAULT FALSE,
    consent_by        TEXT,
    consent_at        TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_students_kelas ON students(kelas_id);
CREATE INDEX idx_students_status ON students(status);

-- ---------- Face templates ----------
CREATE TABLE face_templates (
    id               UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    student_id       UUID NOT NULL REFERENCES students(id) ON DELETE CASCADE,
    embedding        vector(512) NOT NULL,
    quality_score    REAL NOT NULL DEFAULT 0,
    -- Reference photo, encrypted at rest (plan Section 9).
    source_image_ref TEXT,
    is_centroid      BOOLEAN NOT NULL DEFAULT FALSE,
    active           BOOLEAN NOT NULL DEFAULT FALSE,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_face_templates_student ON face_templates(student_id);
CREATE INDEX idx_face_templates_active ON face_templates(student_id) WHERE active;

-- HNSW index for approximate nearest-neighbour search on the embedding.
-- Cosine distance operator class. For a school (<= a few thousand students)
-- brute-force in-memory matching is also fine and simpler; HNSW is here for
-- scale and can be dropped without touching application code.
CREATE INDEX idx_face_templates_embedding_hnsw
    ON face_templates USING hnsw (embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 64);

-- ---------- Devices (kiosks) ----------
CREATE TABLE devices (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    nama       TEXT NOT NULL,
    lokasi     TEXT,
    -- SHA-256 of the device token; the plaintext is never stored (Section 9).
    token_hash TEXT NOT NULL UNIQUE,
    revoked    BOOLEAN NOT NULL DEFAULT FALSE,
    last_seen  TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ---------- Attendance ----------
CREATE TABLE attendance (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    student_id     UUID NOT NULL REFERENCES students(id) ON DELETE CASCADE,
    device_id      UUID REFERENCES devices(id) ON DELETE SET NULL,
    "timestamp"    TIMESTAMPTZ NOT NULL DEFAULT now(),
    similarity     REAL NOT NULL DEFAULT 0,
    margin         REAL NOT NULL DEFAULT 0,
    liveness_score REAL NOT NULL DEFAULT 0,
    status         TEXT NOT NULL DEFAULT 'present'
                   CHECK (status IN ('present', 'manual', 'corrected')),
    note           TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_attendance_student_time ON attendance(student_id, "timestamp" DESC);
CREATE INDEX idx_attendance_time ON attendance("timestamp" DESC);

-- ---------- Attempts (audit & threshold tuning) ----------
CREATE TABLE attempts (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    device_id       UUID REFERENCES devices(id) ON DELETE SET NULL,
    "timestamp"     TIMESTAMPTZ NOT NULL DEFAULT now(),
    outcome         TEXT NOT NULL,
    top1_student_id UUID REFERENCES students(id) ON DELETE SET NULL,
    top1_score      REAL,
    top2_score      REAL,
    margin          REAL,
    liveness_score  REAL,
    reason          TEXT,
    frame_count     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_attempts_time ON attempts("timestamp" DESC);
CREATE INDEX idx_attempts_outcome ON attempts(outcome);

-- ---------- Audit log ----------
CREATE TABLE audit_log (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id    UUID REFERENCES users(id) ON DELETE SET NULL,
    action     TEXT NOT NULL,
    target     TEXT,
    detail     JSONB,
    "timestamp" TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_audit_log_time ON audit_log("timestamp" DESC);

-- ---------- Attendance cooldown helper view ----------
-- Latest attendance timestamp per student, for cooldown checks.
CREATE VIEW student_last_attendance AS
SELECT student_id, MAX("timestamp") AS last_marked
FROM attendance
WHERE status <> 'corrected'
GROUP BY student_id;
