-- Add FK constraint to event_tags.project_id.
--
-- In migration 001 the column is plain INTEGER, which allows tag rows to
-- reference projects that don't exist and, more importantly, means tags
-- aren't cleaned up when a project is deleted. SQLite can't add a FK in
-- place, so we do the standard rebuild dance: new table with the FK +
-- cascades, copy rows, drop, rename, restore indexes.
--
-- Any pre-existing orphan rows (tags whose project no longer exists) are
-- deleted during the copy — they would otherwise fail the new FK check
-- once foreign_keys is ON (which it is at connection setup in db/pool.rs).

-- Turn off FK checks for the rebuild; the PRAGMA is per-connection and
-- won't affect other readers/writers during the transaction.
PRAGMA foreign_keys = OFF;

CREATE TABLE event_tags_new (
    id          INTEGER PRIMARY KEY,
    event_id    INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    project_id  INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL
);

INSERT INTO event_tags_new (id, event_id, project_id, key, value)
SELECT t.id, t.event_id, t.project_id, t.key, t.value
FROM event_tags t
INNER JOIN projects p ON p.id = t.project_id;

DROP TABLE event_tags;
ALTER TABLE event_tags_new RENAME TO event_tags;

CREATE INDEX IF NOT EXISTS idx_event_tags_lookup ON event_tags(project_id, key, value);
CREATE INDEX IF NOT EXISTS idx_event_tags_event ON event_tags(event_id);

PRAGMA foreign_keys = ON;
