# Multi-stage build for the absensi-server.
# Build with ONNX support:  docker build --build-arg FEATURES=onnx ...
# Default builds the model-free server (dev / CI).

FROM rust:1-slim-bookworm AS builder
WORKDIR /app

ARG FEATURES=""

RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies: copy manifests first, build a stub, then copy sources.
COPY Cargo.toml Cargo.lock ./
COPY crates/domain/Cargo.toml crates/domain/Cargo.toml
COPY crates/face-core/Cargo.toml crates/face-core/Cargo.toml
COPY crates/db/Cargo.toml crates/db/Cargo.toml
COPY crates/server/Cargo.toml crates/server/Cargo.toml

RUN mkdir -p crates/domain/src crates/face-core/src crates/db/src crates/server/src \
    && echo 'fn main() {}' > crates/server/src/main.rs \
    && echo '' > crates/domain/src/lib.rs \
    && echo '' > crates/face-core/src/lib.rs \
    && echo '' > crates/db/src/lib.rs \
    && cargo build --release -p absensi-server ${FEATURES:+--features $FEATURES} || true

COPY . .
RUN touch crates/*/src/lib.rs crates/server/src/main.rs \
    && cargo build --release -p absensi-server ${FEATURES:+--features $FEATURES} \
    && cp target/release/absensi-server /app/absensi-server \
    && cp -r crates/db/migrations /app/migrations

FROM debian:bookworm-slim AS runtime
# libgomp1 + libstdc++6 are what the dynamically-loaded libonnxruntime.so needs
# (ORT's CPU execution provider uses OpenMP). Without them, loading the ORT
# dylib fails with a missing-symbol error at startup.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libssl3 libgomp1 libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

# ONNX Runtime shared library is loaded dynamically by the `ort` crate.
# When building with FEATURES=onnx, mount or copy libonnxruntime.so here and
# set ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so.
COPY --from=builder /app/absensi-server /usr/local/bin/absensi-server
COPY --from=builder /app/migrations /app/migrations

ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
WORKDIR /app

# Non-root.
RUN useradd -r -u 10001 absensi
USER absensi

ENTRYPOINT ["/usr/local/bin/absensi-server"]
