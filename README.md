# Bugs

A lightweight, self-hosted error tracking system compatible with Sentry SDKs.

Bugs receives, processes, and visualizes application errors with source map support, release management, and alerting — all backed by SQLite in a single binary.

## Quick start

### Docker

```bash
# Using the pre-built image from GitHub Container Registry
docker run -p 9000:9000 -v bugs-data:/data ghcr.io/edde746/bugs:latest

# Or build from source
docker build -t bugs .
docker run -p 9000:9000 -v bugs-data:/data bugs
```

### From source

```bash
# Build frontend
cd frontend && bun install --frozen-lockfile && bun run build && cd ..

# Build and run
cargo build --release
./target/release/bugs
```

The UI is available at `http://localhost:9000`.

## Configuration

Configuration is loaded from (highest precedence first):

1. Environment variables prefixed with `BUGS_` (nested keys split by `_`)
2. A `bugs.toml` file in the working directory
3. Built-in defaults

### Options

| Variable | TOML key | Default | Description |
|----------|----------|---------|-------------|
| `BUGS_BIND_ADDRESS` | `bind_address` | `0.0.0.0:9000` | Listen address |
| `BUGS_DATABASE_PATH` | `database_path` | `./data/bugs.db` | SQLite database path |
| `BUGS_ARTIFACTS_DIR` | `artifacts_dir` | `./data/artifacts` | Release file storage |
| `BUGS_RETENTION_DAYS` | `retention_days` | `90` | Days to keep events |
| `BUGS_ENVELOPE_RETENTION_HOURS` | `envelope_retention_hours` | `24` | Hours to keep raw envelopes |
| `BUGS_WORKER_THREADS` | `worker_threads` | `4` | Background processing threads |
| `BUGS_AUTH__ADMIN_TOKEN` | `auth.admin_token` | *(empty)* | Bearer token for management API |

### SQLite tuning

| Variable | Default | Description |
|----------|---------|-------------|
| `BUGS_SQLITE__SYNCHRONOUS` | `NORMAL` | SQLite synchronous mode |
| `BUGS_SQLITE__CACHE_SIZE_MB` | `64` | Page cache size |
| `BUGS_SQLITE__READER_CONNECTIONS` | `8` | Read connection pool size |
| `BUGS_SQLITE__MMAP_SIZE_MB` | `256` | Memory-mapped I/O size |
| `BUGS_SQLITE__CHECKPOINT_INTERVAL_BATCHES` | `10` | WAL checkpoint frequency |

### Ingest limits

| Variable | Default | Description |
|----------|---------|-------------|
| `BUGS_INGEST__MAX_RAW_REQUEST_BYTES` | `20 MB` | Max raw request body |
| `BUGS_INGEST__MAX_ENVELOPE_BYTES` | `10 MB` | Max decompressed envelope |
| `BUGS_INGEST__MAX_EVENT_ITEM_BYTES` | `1 MB` | Max single event item |
| `BUGS_INGEST__MAX_ATTACHMENT_BYTES` | `10 MB` | Max attachment size |
| `BUGS_INGEST__MAX_ITEMS_PER_ENVELOPE` | `100` | Max items per envelope |

## Authentication

### Admin API

Set `BUGS_AUTH__ADMIN_TOKEN` to secure the management API and web UI. Requests must include `Authorization: Bearer <token>`. The web UI will show a login page when auth is enabled. If no token is configured, the management API is open — a warning is logged at startup.

### Ingest

Ingest endpoints authenticate via Sentry DSN keys. Create a project in the UI to get a DSN, then configure your Sentry SDK with it.

## Sentry SDK integration

Point any Sentry SDK at your Bugs instance:

```javascript
Sentry.init({
  dsn: "http://<public_key>@<bugs-host>:9000/<project_id>",
});
```

The DSN is shown in project settings after creating a project.

## Development

```bash
# Backend (auto-reloads with cargo-watch)
cargo watch -x run

# Frontend dev server (proxies /api to localhost:9000)
cd frontend && bun run dev
```

## Production Docker Compose

```yaml
services:
  bugs:
    image: ghcr.io/edde746/bugs:latest
    restart: unless-stopped
    ports:
      - "9000:9000"
    volumes:
      - bugs-data:/data
    environment:
      BUGS_AUTH__ADMIN_TOKEN: "change-me-to-a-secret-token"
      BUGS_RETENTION_DAYS: "90"

volumes:
  bugs-data:
```
