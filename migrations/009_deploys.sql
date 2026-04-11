CREATE TABLE IF NOT EXISTS deploys (
    id              INTEGER PRIMARY KEY,
    release_id      INTEGER NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
    environment     TEXT NOT NULL DEFAULT '',
    name            TEXT NOT NULL DEFAULT '',
    url             TEXT NOT NULL DEFAULT '',
    date_started    TEXT,
    date_finished   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS idx_deploys_release ON deploys(release_id, date_finished DESC);
