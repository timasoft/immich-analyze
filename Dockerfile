# Stage 1: Build application
FROM rust:1.91-alpine AS builder

RUN apk add --no-cache \
    ca-certificates \
    pkgconfig \
    openssl-dev \
    build-base

WORKDIR /app

COPY Cargo.toml Cargo.lock ./

RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs

RUN cargo fetch --locked

COPY src/ ./src/
COPY locales/ ./locales/

RUN cargo build --release --locked

# Stage 2: Final runtime image
FROM alpine:3.20

RUN apk add --no-cache \
    ca-certificates \
    openssl \
    curl \
    bash

RUN addgroup -S appuser && adduser -S appuser -G appuser

COPY --from=builder --chown=appuser:appuser /app/target/release/immich-analyze /usr/local/bin/

COPY --chown=appuser:appuser entrypoint.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/entrypoint.sh

RUN mkdir -p /data && \
    chown -R appuser:appuser /data

WORKDIR /app
RUN chown appuser:appuser /app

HEALTHCHECK --interval=30s --timeout=10s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

LABEL org.opencontainers.image.source="https://github.com/timasoft/immich-analyze"
LABEL org.opencontainers.image.description="Immich image analysis service with AI-powered descriptions"
LABEL org.opencontainers.image.version="0.3.1"
LABEL org.opencontainers.image.authors="timasoft"

USER appuser

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
