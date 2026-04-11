-- Single-tenant org (simplified, but org-scoped for correct release model)
CREATE TABLE IF NOT EXISTS organizations (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL DEFAULT 'Default',
    slug        TEXT NOT NULL UNIQUE DEFAULT 'default',
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
INSERT OR IGNORE INTO organizations (id, name, slug) VALUES (1, 'Default', 'default');

CREATE TABLE IF NOT EXISTS projects (
    id          INTEGER PRIMARY KEY,
    org_id      INTEGER NOT NULL DEFAULT 1 REFERENCES organizations(id),
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL,
    platform    TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    UNIQUE(org_id, slug)
);

CREATE TABLE IF NOT EXISTS project_settings (
    project_id         INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    allowed_origins    TEXT,
    max_event_size     INTEGER DEFAULT 1048576,
    retention_days     INTEGER DEFAULT NULL,
    rate_limit_per_min INTEGER DEFAULT NULL
);

CREATE TABLE IF NOT EXISTS project_keys (
    id          INTEGER PRIMARY KEY,
    project_id  INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    public_key  TEXT NOT NULL UNIQUE,
    label       TEXT NOT NULL DEFAULT 'Default',
    is_active   INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    rate_limit  INTEGER DEFAULT NULL
);
CREATE INDEX IF NOT EXISTS idx_project_keys_public ON project_keys(public_key) WHERE is_active = 1;

-- Releases are ORG-SCOPED
CREATE TABLE IF NOT EXISTS releases (
    id          INTEGER PRIMARY KEY,
    org_id      INTEGER NOT NULL DEFAULT 1 REFERENCES organizations(id),
    version     TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    data        TEXT,
    UNIQUE(org_id, version)
);

CREATE TABLE IF NOT EXISTS release_projects (
    release_id  INTEGER NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
    project_id  INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    PRIMARY KEY (release_id, project_id)
);

CREATE TABLE IF NOT EXISTS release_files (
    id          INTEGER PRIMARY KEY,
    release_id  INTEGER NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    file_path   TEXT NOT NULL,
    file_size   INTEGER NOT NULL DEFAULT 0,
    sha256      TEXT,
    dist        TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    UNIQUE(release_id, name, dist)
);

CREATE TABLE IF NOT EXISTS artifact_debug_ids (
    id          INTEGER PRIMARY KEY,
    debug_id    TEXT NOT NULL,
    release_id  INTEGER REFERENCES releases(id) ON DELETE CASCADE,
    file_path   TEXT NOT NULL,
    source_name TEXT,
    kind        TEXT NOT NULL DEFAULT 'sourcemap',
    UNIQUE(debug_id, kind)
);
CREATE INDEX IF NOT EXISTS idx_artifact_debug_ids_lookup ON artifact_debug_ids(debug_id);

CREATE TABLE IF NOT EXISTS issues (
    id              INTEGER PRIMARY KEY,
    project_id      INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    fingerprint     TEXT NOT NULL,
    title           TEXT NOT NULL,
    culprit         TEXT,
    level           TEXT NOT NULL DEFAULT 'error',
    status          TEXT NOT NULL DEFAULT 'unresolved',
    first_seen      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    last_seen       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    event_count     INTEGER NOT NULL DEFAULT 0,
    metadata        TEXT,
    UNIQUE(project_id, fingerprint)
);
CREATE INDEX IF NOT EXISTS idx_issues_project_status ON issues(project_id, status, last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_issues_first_seen ON issues(project_id, first_seen DESC);

CREATE TABLE IF NOT EXISTS event_envelopes (
    id                    INTEGER PRIMARY KEY,
    project_id            INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    event_id              TEXT NOT NULL,
    received_at           TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    content_encoding      TEXT,
    body                  BLOB NOT NULL,
    state                 TEXT NOT NULL DEFAULT 'pending',
    attempts              INTEGER NOT NULL DEFAULT 0,
    last_error            TEXT,
    next_attempt_at       TEXT,
    processing_started_at TEXT,
    UNIQUE(project_id, event_id)
);
CREATE INDEX IF NOT EXISTS idx_envelopes_pending ON event_envelopes(state, next_attempt_at)
    WHERE state IN ('pending', 'failed');
CREATE INDEX IF NOT EXISTS idx_envelopes_processing ON event_envelopes(state, processing_started_at)
    WHERE state = 'processing';

CREATE TABLE IF NOT EXISTS events (
    id                    INTEGER PRIMARY KEY,
    event_id              TEXT NOT NULL,
    project_id            INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    issue_id              INTEGER REFERENCES issues(id) ON DELETE SET NULL,
    timestamp             TEXT NOT NULL,
    received_at           TEXT NOT NULL,
    level                 TEXT NOT NULL DEFAULT 'error',
    platform              TEXT,
    release               TEXT,
    environment           TEXT,
    transaction_name      TEXT,
    trace_id              TEXT,
    message               TEXT,
    title                 TEXT,
    exception_values      TEXT,
    stacktrace_functions  TEXT,
    data                  TEXT NOT NULL,
    UNIQUE(project_id, event_id)
);
CREATE INDEX IF NOT EXISTS idx_events_issue ON events(issue_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_events_project_time ON events(project_id, received_at DESC);
CREATE INDEX IF NOT EXISTS idx_events_trace ON events(trace_id) WHERE trace_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS event_tags (
    id          INTEGER PRIMARY KEY,
    event_id    INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    project_id  INTEGER NOT NULL,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_event_tags_lookup ON event_tags(project_id, key, value);
CREATE INDEX IF NOT EXISTS idx_event_tags_event ON event_tags(event_id);

CREATE TABLE IF NOT EXISTS tag_keys (
    id          INTEGER PRIMARY KEY,
    project_id  INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key         TEXT NOT NULL,
    values_seen INTEGER NOT NULL DEFAULT 0,
    UNIQUE(project_id, key)
);

CREATE TABLE IF NOT EXISTS tag_values (
    id          INTEGER PRIMARY KEY,
    project_id  INTEGER NOT NULL,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL,
    times_seen  INTEGER NOT NULL DEFAULT 1,
    last_seen   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    UNIQUE(project_id, key, value)
);
CREATE INDEX IF NOT EXISTS idx_tag_values_freq ON tag_values(project_id, key, times_seen DESC);

CREATE TABLE IF NOT EXISTS issue_stats_hourly (
    id          INTEGER PRIMARY KEY,
    issue_id    INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    project_id  INTEGER NOT NULL,
    bucket      TEXT NOT NULL,
    count       INTEGER NOT NULL DEFAULT 0,
    UNIQUE(issue_id, bucket)
);
CREATE INDEX IF NOT EXISTS idx_issue_stats_bucket ON issue_stats_hourly(project_id, bucket DESC);

CREATE TABLE IF NOT EXISTS alert_rules (
    id          INTEGER PRIMARY KEY,
    project_id  INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    conditions  TEXT NOT NULL,
    actions     TEXT NOT NULL,
    frequency   INTEGER NOT NULL DEFAULT 1800,
    last_fired  TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
