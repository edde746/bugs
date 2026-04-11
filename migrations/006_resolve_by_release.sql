-- Resolve by release: track which release fixed an issue
-- Value is either a version string or '__next__' for "resolve in next release"
ALTER TABLE issues ADD COLUMN resolved_in_release TEXT;
