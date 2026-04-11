-- Issue muting: allow ignoring issues with optional auto-unmute conditions
ALTER TABLE issues ADD COLUMN snooze_until TEXT;
ALTER TABLE issues ADD COLUMN snooze_event_count INTEGER;

CREATE INDEX IF NOT EXISTS idx_issues_snooze ON issues(status, snooze_until)
    WHERE status = 'ignored' AND snooze_until IS NOT NULL;
