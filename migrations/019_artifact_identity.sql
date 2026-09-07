-- Preserve historical rows and release ownership while replacing global identity.
-- NULL quality means unmeasured, not stripped: existing caches are inspected lazily.
CREATE TABLE artifact_debug_ids_new (
    id          INTEGER PRIMARY KEY,
    debug_id    TEXT NOT NULL,
    project_id  INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    release_id  INTEGER REFERENCES releases(id) ON DELETE CASCADE,
    file_path   TEXT NOT NULL,
    source_name TEXT,
    arch        TEXT NOT NULL DEFAULT '',
    code_id     TEXT,
    kind        TEXT NOT NULL DEFAULT 'sourcemap',
    quality_lines INTEGER,
    quality_files INTEGER,
    quality_functions INTEGER,
    content_hash TEXT,
    UNIQUE(project_id, debug_id, arch, kind)
);

INSERT INTO artifact_debug_ids_new
    (id, debug_id, project_id, release_id, file_path, source_name, arch, code_id, kind)
SELECT a.id, a.debug_id,
       COALESCE(a.project_id, (SELECT MIN(rp.project_id) FROM release_projects rp
                              WHERE rp.release_id = a.release_id)),
       a.release_id, a.file_path, a.source_name, COALESCE(a.arch, ''), a.code_id, a.kind
FROM artifact_debug_ids a;

-- A formerly unscoped release artifact belongs to every project on its release.
INSERT INTO artifact_debug_ids_new
    (debug_id, project_id, release_id, file_path, source_name, arch, code_id, kind)
SELECT a.debug_id, rp.project_id, a.release_id, a.file_path, a.source_name,
       COALESCE(a.arch, ''), a.code_id, a.kind
FROM artifact_debug_ids a
JOIN release_projects rp ON rp.release_id = a.release_id
WHERE a.project_id IS NULL
  AND rp.project_id != (SELECT MIN(p.project_id) FROM release_projects p
                       WHERE p.release_id = a.release_id);

DROP TABLE artifact_debug_ids;
ALTER TABLE artifact_debug_ids_new RENAME TO artifact_debug_ids;
CREATE INDEX idx_artifact_debug_ids_lookup ON artifact_debug_ids(debug_id);
CREATE INDEX idx_artifact_debug_ids_project ON artifact_debug_ids(project_id, debug_id, kind);
-- Retain unowned historical artifacts without giving them cross-project visibility.
CREATE UNIQUE INDEX idx_artifact_debug_ids_unowned
    ON artifact_debug_ids(debug_id, arch, kind) WHERE project_id IS NULL;
