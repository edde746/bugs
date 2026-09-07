-- Receipts outlive uploaded chunks. A debug-ID binding is part of the key
-- because Dart maps do not carry their target debug ID in their contents.
CREATE TABLE dif_assemblies (
    id TEXT PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    checksum TEXT NOT NULL,
    debug_id TEXT NOT NULL DEFAULT '',
    request_json TEXT NOT NULL,
    staging_root TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'processing', 'ok', 'error')),
    result_json TEXT,
    lease_owner TEXT,
    lease_until INTEGER,
    updated_at INTEGER NOT NULL,
    UNIQUE(project_id, checksum, debug_id)
);
CREATE INDEX idx_dif_assemblies_pending ON dif_assemblies(state, lease_until, updated_at);
