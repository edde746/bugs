-- Store Sentry envelope attachments separately from the event JSON so event
-- detail responses stay lightweight while downloads preserve original bytes.
CREATE TABLE IF NOT EXISTS event_attachments (
    id              INTEGER PRIMARY KEY,
    event_id        INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    content_type    TEXT,
    attachment_type TEXT,
    size            INTEGER NOT NULL,
    body            BLOB NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_event_attachments_event
    ON event_attachments(event_id, id);
