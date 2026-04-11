CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
    title,
    message,
    exception_values,
    stacktrace_functions,
    content='events',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2',
    detail='none'
);

CREATE TRIGGER IF NOT EXISTS events_fts_insert AFTER INSERT ON events BEGIN
    INSERT INTO events_fts(rowid, title, message, exception_values, stacktrace_functions)
    VALUES (new.id, new.title, new.message, new.exception_values, new.stacktrace_functions);
END;

CREATE TRIGGER IF NOT EXISTS events_fts_delete BEFORE DELETE ON events BEGIN
    INSERT INTO events_fts(events_fts, rowid, title, message, exception_values, stacktrace_functions)
    VALUES ('delete', old.id, old.title, old.message, old.exception_values, old.stacktrace_functions);
END;
