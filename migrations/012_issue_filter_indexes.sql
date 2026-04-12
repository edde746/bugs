CREATE INDEX IF NOT EXISTS idx_events_project_release ON events(project_id, release) WHERE release IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_events_project_environment ON events(project_id, environment) WHERE environment IS NOT NULL;
