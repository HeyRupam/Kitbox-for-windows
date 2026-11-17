pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS files (
    hash        TEXT PRIMARY KEY,
    size        INTEGER NOT NULL,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL,
    width       INTEGER,
    height      INTEGER,
    pages       INTEGER,
    source_path TEXT,
    thumb_hash  TEXT,
    created_at  INTEGER NOT NULL,
    pinned      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS jobs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    tool        TEXT NOT NULL,
    recipe      TEXT NOT NULL,
    input_hash  TEXT NOT NULL,
    output_hash TEXT NOT NULL,
    summary     TEXT NOT NULL,
    duration_ms INTEGER NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tags (
    hash TEXT NOT NULL,
    tag  TEXT NOT NULL,
    PRIMARY KEY (hash, tag)
);

CREATE INDEX IF NOT EXISTS idx_files_created ON files (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_files_name    ON files (name);
CREATE INDEX IF NOT EXISTS idx_jobs_input    ON jobs (input_hash);
CREATE INDEX IF NOT EXISTS idx_jobs_output   ON jobs (output_hash);
"#;
