# flo — Kubernetes robot orchestration client
# SKELETON: builds successfully but is NOT yet published by CI.
# Base runtime is distroless/static so a normal gnu release binary and TLS
# CA certificates both work (flo may need TLS at runtime). A future, explicitly
# musl-static release image could use `scratch`.

# --- build stage ---
FROM rust:1.97-slim AS build
WORKDIR /src
COPY . .
# Default features only (no GStreamer). A `media` variant would need a GStreamer
# dev layer here and `--features media`.
RUN cargo build --release --bin flo

# --- runtime stage ---
FROM gcr.io/distroless/static-debian12
COPY --from=build /src/target/release/flo /usr/local/bin/flo
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/flo"]
