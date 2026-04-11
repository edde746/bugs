CREATE INDEX IF NOT EXISTS idx_events_user_id ON events(
    project_id, json_extract(data, '$.user.id')
) WHERE json_extract(data, '$.user.id') IS NOT NULL;
