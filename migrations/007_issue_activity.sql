-- Issue activity: tracks state transitions and events
CREATE TABLE IF NOT EXISTS issue_activity (
    id          INTEGER PRIMARY KEY,
    issue_id    INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,  -- first_seen, resolved, unresolved, ignored, unignored, regression
    data        TEXT,           -- optional JSON metadata
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS idx_issue_activity_issue ON issue_activity(issue_id, created_at DESC);
