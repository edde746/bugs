CREATE TABLE IF NOT EXISTS user_reports (
    id          INTEGER PRIMARY KEY,
    project_id  INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    event_id    TEXT NOT NULL,
    name        TEXT NOT NULL DEFAULT '',
    email       TEXT NOT NULL DEFAULT '',
    comments    TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS idx_user_reports_event ON user_reports(event_id);
CREATE INDEX IF NOT EXISTS idx_user_reports_project ON user_reports(project_id, created_at DESC);
