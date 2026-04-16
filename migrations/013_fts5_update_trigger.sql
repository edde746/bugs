-- FTS5 UPDATE trigger.
--
-- Migration 002 wired only INSERT and DELETE triggers, so any UPDATE to an
-- events row left the events_fts index stale. In practice updates are rare
-- today, but retention/backfill paths or future enrichment jobs would
-- silently desync search. This adds the missing UPDATE trigger following
-- the standard FTS5 external content pattern: delete-old, then insert-new.

CREATE TRIGGER IF NOT EXISTS events_fts_update AFTER UPDATE ON events BEGIN
    INSERT INTO events_fts(events_fts, rowid, title, message, exception_values, stacktrace_functions)
    VALUES ('delete', old.id, old.title, old.message, old.exception_values, old.stacktrace_functions);
    INSERT INTO events_fts(rowid, title, message, exception_values, stacktrace_functions)
    VALUES (new.id, new.title, new.message, new.exception_values, new.stacktrace_functions);
END;
