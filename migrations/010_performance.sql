-- Performance monitoring: transaction groups and individual transactions
CREATE TABLE IF NOT EXISTS transaction_groups (
    id              INTEGER PRIMARY KEY,
    project_id      INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    transaction_name TEXT NOT NULL,
    op              TEXT NOT NULL DEFAULT '',
    method          TEXT NOT NULL DEFAULT '',
    count           INTEGER NOT NULL DEFAULT 0,
    error_count     INTEGER NOT NULL DEFAULT 0,
    sum_duration_ms REAL NOT NULL DEFAULT 0,
    min_duration_ms REAL,
    max_duration_ms REAL,
    p50_duration_ms REAL,
    p95_duration_ms REAL,
    last_seen       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    UNIQUE(project_id, transaction_name, op, method)
);
CREATE INDEX IF NOT EXISTS idx_txn_groups_project ON transaction_groups(project_id, last_seen DESC);

CREATE TABLE IF NOT EXISTS transactions (
    id              INTEGER PRIMARY KEY,
    project_id      INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    group_id        INTEGER REFERENCES transaction_groups(id) ON DELETE SET NULL,
    trace_id        TEXT,
    transaction_name TEXT NOT NULL,
    op              TEXT NOT NULL DEFAULT '',
    method          TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL DEFAULT 'ok',
    duration_ms     REAL NOT NULL DEFAULT 0,
    timestamp       TEXT NOT NULL,
    environment     TEXT,
    release         TEXT,
    data            TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS idx_transactions_group ON transactions(group_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_transactions_project ON transactions(project_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_transactions_group_duration ON transactions(group_id, duration_ms);
