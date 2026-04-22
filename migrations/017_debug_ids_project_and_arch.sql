-- Expand artifact_debug_ids to support native DIFs (dSYM / PDB / ELF /
-- Dart split-debug-info). Previously only sourcemaps were indexed here.
-- The table was defined in 001 but never populated; the columns added
-- below are NULL-safe for existing 'sourcemap' rows (currently none).
ALTER TABLE artifact_debug_ids ADD COLUMN project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE;
ALTER TABLE artifact_debug_ids ADD COLUMN arch TEXT;
ALTER TABLE artifact_debug_ids ADD COLUMN code_id TEXT;

CREATE INDEX IF NOT EXISTS idx_artifact_debug_ids_project
    ON artifact_debug_ids(project_id, debug_id, kind);
