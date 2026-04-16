-- Indexes that the original schema missed.
--
-- 1. idx_issue_activity_created_at: migration 007 only indexed (issue_id,
--    created_at). Any query that scans activity across all issues within
--    a time range (global audit view, recent-activity feeds) currently
--    does a full table scan.
--
-- 2. idx_event_tags_event_key: the existing idx_event_tags_event only
--    covers event_id alone. Queries on the issue detail page fetch all
--    tag values for a given event filtered by key (e.g. "environment"),
--    and benefit from a composite.

CREATE INDEX IF NOT EXISTS idx_issue_activity_created_at
    ON issue_activity(created_at DESC);

CREATE INDEX IF NOT EXISTS idx_event_tags_event_key
    ON event_tags(event_id, key);
