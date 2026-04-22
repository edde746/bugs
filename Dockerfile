# Stage 1: Build frontend
FROM oven/bun:1-alpine AS frontend-builder
WORKDIR /app/frontend
COPY frontend/package.json frontend/bun.lock ./
RUN bun install --frozen-lockfile
COPY frontend/ ./
RUN bun run build

# Stage 2: Build Rust binary
FROM rust:1.94-alpine AS rust-builder
# g++ is required for symbolic-demangle's C++ demangler build script (cc-rs).
RUN apk add --no-cache musl-dev sqlite-dev sqlite-static g++
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
COPY migrations/ ./migrations/
COPY --from=frontend-builder /app/frontend/dist ./frontend/dist/
ENV LIBSQLITE3_SYS_USE_PKG_CONFIG=1
RUN cargo build --release

# Stage 3: Final image
FROM alpine:3.21
# wget is already part of busybox in alpine; ca-certificates for outgoing TLS.
RUN apk add --no-cache ca-certificates
COPY --from=rust-builder /app/target/release/bugs /usr/local/bin/bugs

# Run as an unprivileged user. The container only needs to read its own
# binary, write to /data, and bind 9000 (unprivileged port).
RUN addgroup -S bugs && adduser -S -G bugs -u 1001 bugs \
    && mkdir -p /data/artifacts \
    && chown -R bugs:bugs /data
VOLUME /data
ENV BUGS_DATABASE_PATH=/data/bugs.db
ENV BUGS_ARTIFACTS_DIR=/data/artifacts
EXPOSE 9000

# Minimal healthcheck hitting the unauthenticated /api/health endpoint.
# -q keeps logs quiet; -O- discards the body; --spider would skip the
# body entirely but isn't reliable across busybox versions.
HEALTHCHECK --interval=30s --timeout=3s --retries=3 \
    CMD wget -qO- http://127.0.0.1:9000/api/health || exit 1

USER bugs
ENTRYPOINT ["bugs"]
