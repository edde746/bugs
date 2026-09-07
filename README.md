# Bugs

A lightweight, self-hosted error tracking system compatible with Sentry SDKs.

Bugs receives, processes, and visualizes application errors with source map support, release management, and alerting — all backed by SQLite in a single binary.

## Comparison

Resource usage of Docker images, measured idle after startup.

| | Bugs | [Rustrak](https://github.com/AbianS/rustrak) | [Bugsink](https://github.com/bugsink/bugsink) | [GlitchTip](https://gitlab.com/glitchtip/glitchtip-backend) |
|---|---|---|---|---|
| **Language** | Rust | Rust + Node.js | Python | Python |
| **Database** | SQLite (embedded) | SQLite or PostgreSQL | SQLite or PostgreSQL | PostgreSQL (required) |
| **Docker image size** | 40 MB | 443 MB | 649 MB | 1.05 GB |
| **Idle RAM** | ~3 MB | ~68 MB | ~285 MB | ~145 MB |
| **Total RAM (with deps)** | ~3 MB | ~100 MB | ~285 MB | ~190 MB |
| **Containers needed** | 1 | 3 | 1 | 2+ |

### Features

| | Bugs | Rustrak | Bugsink | GlitchTip |
|---|---|---|---|---|
| **Sentry SDK compatible** | ✓ | ✓ | ✓ | ✓ |
| **Source maps** | ✓ | ✗ | ✓ | ✓ |
| **Releases & deploys** | ✓ | ✗ | ✓ | ✓ |
| **Performance monitoring** | ✓ | ✗ | ✗ | ✓ |
| **Alerts** | ✓ | ✓ | ✓ | ✓ |
| **User feedback** | ✓ | ✗ | ✓ | ✓ |
| **Full-text search** | ✓ | ✗ | ✓ | ✓ |
| **Environments** | ✓ | ✓ | ✓ | ✓ |
| **Multi-user / teams** | ✗ | ✗ | ✓ | ✓ |
| **Uptime monitoring** | ✗ | ✗ | ✗ | ✓ |
| **Retention policies** | ✓ | ✗ | ✓ | ✓ |

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

1. Environment variables prefixed with `BUGS_` (nested keys split by `__`)
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
| `BUGS_MCP__ENABLED` | `mcp.enabled` | `false` | Expose MCP server at `/mcp` for LLM agents |

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
| `BUGS_INGEST__MAX_ITEMS_PER_ENVELOPE` | `100` | Max items per envelope |

### Admin upload limits

| Variable | Default | Description |
|----------|---------|-------------|
| `BUGS_UPLOADS__MAX_BYTES` | `2 GB` | Max size for admin-authenticated uploads (dSYMs, release artifacts) |
| `BUGS_UPLOADS__CHUNK_RETENTION_HOURS` | `12` | Hours to keep chunk-upload blobs after last use; active uploads refresh this TTL |
| `BUGS_UPLOADS__ASSEMBLY_CONCURRENCY` | `1` | Maximum simultaneous DIF conversions per process; independent of event workers |
| `BUGS_UPLOADS__MAX_WAIT_SECS` | `600` | Advertised client polling budget for DIF processing; not an HTTP connection timeout |

### Native and Dart symbol uploads

The DIF assemble endpoint returns `not_found` with missing chunk hashes, `assembling`
while queued or processing, and `ok` or `error` on completion. Clients must wait for
processing (`sentry-cli debug-files upload --wait-for 600`) to detect conversion
failures. Expensive hashing and SymCache conversion do not run in the HTTP handler.

Assembly jobs and completed receipts are stored in SQLite. Admitted jobs pin their
chunks with hard links under the artifacts directory, so chunk retention cannot
remove active inputs. Workers renew 60-second leases; unfinished jobs are reclaimed
after a crashed worker's lease expires. Completed receipts survive chunk expiry and
check the current canonical artifact before returning success. Failed attempts stay
visible for 60 seconds before a new request may retry them.

Native artifacts are selected by project, debug ID, architecture and kind. Immutable
content-addressed files prevent concurrent replacement of mapped caches; measured
debug/line information takes precedence over stripped symbols. Keep assembly
concurrency at one on memory-constrained deployments: chunk-upload concurrency does
not limit conversion memory.

Before upgrading an existing deployment, back up both the database and artifacts.
Migrations 019–020 rebuild artifact identity and add durable assembly state;
schema changes and migration markers commit atomically. Rolling back to an older
binary requires the pre-upgrade backup, not just changing the image tag.

## Authentication

### Admin API

Set `BUGS_AUTH__ADMIN_TOKEN` to secure the management API and web UI. Requests must include `Authorization: Bearer <token>`. The web UI will show a login page when auth is enabled. If no token is configured, the management API is open — a warning is logged at startup.

### Ingest

Ingest endpoints authenticate via Sentry DSN keys. Create a project in the UI to get a DSN, then configure your Sentry SDK with it.

## MCP server

Set `BUGS_MCP__ENABLED=true` to expose an [MCP](https://modelcontextprotocol.io/) server (Streamable HTTP) at `/mcp`, letting LLM agents query and triage your instance: list projects and issues, read full event payloads (stack traces, breadcrumbs, variables), full-text search, resolve/ignore issues, and leave comments. Destructive operations (deletes, project/key management) are not exposed.

The endpoint shares the management API's bearer-token auth. Connect from Claude Code:

```bash
claude mcp add --transport http bugs http://localhost:9000/mcp \
  --header "Authorization: Bearer <admin_token>"
```

## Sentry SDK integration

Point any Sentry SDK at your Bugs instance:

```javascript
Sentry.init({
  dsn: "http://<public_key>@<bugs-host>:9000/<project_id>",
});
```

The DSN is shown in project settings after creating a project.

## Database migrations

Schema changes live in `migrations/` as numbered SQL files. They are applied once in order on startup and tracked in the `_migrations` table.

The migration runner is **forward-only** — there is no rollback mechanism and no down migrations. To change an existing schema object, add a new migration that performs the alteration (SQLite often requires the full table-rebuild pattern: new table, `INSERT SELECT`, drop, rename; see `014_event_tags_project_fk.sql`). Don't edit files that have already been released.

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
