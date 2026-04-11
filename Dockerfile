# Stage 1: Build frontend
FROM node:22-alpine AS frontend-builder
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# Stage 2: Build Rust binary
FROM rust:1.85-alpine AS rust-builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
COPY migrations/ ./migrations/
COPY --from=frontend-builder /app/frontend/dist ./frontend/dist/
RUN cargo build --release

# Stage 3: Final image
FROM alpine:3.21
RUN apk add --no-cache ca-certificates
COPY --from=rust-builder /app/target/release/bugs /usr/local/bin/bugs
RUN mkdir -p /data/artifacts
VOLUME /data
ENV BUGS_DATABASE_PATH=/data/bugs.db
ENV BUGS_ARTIFACTS_DIR=/data/artifacts
ENV BUGS_BIND_ADDRESS=0.0.0.0:9000
EXPOSE 9000
ENTRYPOINT ["bugs"]
