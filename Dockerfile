# syntax=docker/dockerfile:1
# flo — multi-stage multi-target Dockerfile
#
# Build a specific image:
#   docker build --target server  -t ghcr.io/OWNER/flo-server:TAG .
#   docker build --target client  -t ghcr.io/OWNER/flo-client:TAG .
#   docker build --target server-media  -t ghcr.io/OWNER/flo-server-media:TAG .
#   docker build --target client-media  -t ghcr.io/OWNER/flo-client-media:TAG .

# Base images pinned by digest (see .github/dependabot.yml `docker` ecosystem).
# Update via Dependabot; verify with `docker buildx imagetools inspect <image>`.
# === Shared builder with cargo-chef ===
FROM rust:1.97-slim-bookworm@sha256:2775a09d208ff0d7c1f50490c45b62db929e87ba1dcbc3f2132ac71a704bcdd3 AS chef
WORKDIR /app
RUN cargo install cargo-chef --version 0.1.71 --locked

# === Planner: compute dependency recipes ===
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/bin && touch src/lib.rs src/bin/flo-server.rs src/bin/flo-client.rs
RUN cargo chef prepare --recipe-path recipe.json

# === Builder: default features ===
FROM chef AS build-default
ARG CARGO_TARGET_DIR=/app/target
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    cargo build --release --bin flo-server && \
    cargo build --release --bin flo

# === Builder: media features ===
FROM chef AS build-media
ARG CARGO_TARGET_DIR=/app/target
RUN apt-get update && apt-get install -y --no-install-recommends \
    libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev \
    libx264-dev \
    && rm -rf /var/lib/apt/lists/*
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    cargo chef cook --release --features media --recipe-path recipe.json
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    cargo build --release --features media --bin flo-server && \
    cargo build --release --features media --bin flo

# === Runtime: flo-server (default) ===
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f AS server
COPY --from=build-default /app/target/release/flo-server /flo-server
ENV FLO_HEALTH_ADDR=0.0.0.0:8080
EXPOSE 8080
HEALTHCHECK CMD ["/flo-server", "--healthcheck"]
ENTRYPOINT ["/flo-server"]

# === Runtime: flo-client (default) ===
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f AS client
COPY --from=build-default /app/target/release/flo /flo
ENV FLO_HEALTH_ADDR=0.0.0.0:8080
EXPOSE 8080
HEALTHCHECK CMD ["/flo", "--healthcheck"]
ENTRYPOINT ["/flo"]

# === Runtime base: media (shared) ===
# GStreamer is runtime-loaded by the binary, so media images keep a slim
# debian base (plugins + loader env) instead of distroless; posture below
# mirrors the distroless non-root UID used by the default images.
FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241 AS media-runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad \
    gstreamer1.0-plugins-ugly \
    libgstreamer1.0-0 \
    && rm -rf /var/lib/apt/lists/*

# === Runtime: flo-server (media) ===
FROM media-runtime AS server-media
COPY --from=build-media /app/target/release/flo-server /flo-server
ENV FLO_HEALTH_ADDR=0.0.0.0:8080
EXPOSE 8080
USER 65532:65532
HEALTHCHECK CMD ["/flo-server", "--healthcheck"]
ENTRYPOINT ["/flo-server"]

# === Runtime: flo-client (media) ===
FROM media-runtime AS client-media
COPY --from=build-media /app/target/release/flo /flo
ENV FLO_HEALTH_ADDR=0.0.0.0:8080
EXPOSE 8080
USER 65532:65532
HEALTHCHECK CMD ["/flo", "--healthcheck"]
ENTRYPOINT ["/flo"]
