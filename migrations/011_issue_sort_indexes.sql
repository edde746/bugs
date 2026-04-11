-- Covering indexes for issue list sorting by event_count and first_seen with status filter
CREATE INDEX IF NOT EXISTS idx_issues_event_count ON issues(project_id, status, event_count DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_issues_first_seen_status ON issues(project_id, status, first_seen DESC, id DESC);
