CREATE TABLE repos (
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    display_name TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE files (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL REFERENCES repos(id),
    relpath TEXT NOT NULL,
    xxh64 TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    UNIQUE (repo_id, relpath)
);

CREATE TABLE chunks (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL REFERENCES files(id),
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    symbol TEXT,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL
);
CREATE INDEX chunks_by_file ON chunks(file_id);
CREATE INDEX chunks_by_hash ON chunks(content_hash);

-- Contentless: rows are fed by the triggers so relpath can be copied in
-- from files and path words rank in BM25 alongside content and symbols.
CREATE VIRTUAL TABLE chunks_fts USING fts5(
    content, symbol, relpath,
    content='', contentless_delete=1
);
CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
    INSERT INTO chunks_fts(rowid, content, symbol, relpath)
    VALUES (new.id, new.content, new.symbol,
            (SELECT relpath FROM files WHERE id = new.file_id));
END;
CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
    DELETE FROM chunks_fts WHERE rowid = old.id;
END;

CREATE TABLE memories (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER REFERENCES repos(id),
    kind TEXT NOT NULL CHECK (kind IN ('lesson', 'decision', 'convention', 'skill', 'fact', 'episode')),
    content TEXT NOT NULL,
    salience REAL NOT NULL DEFAULT 0.5,
    surprise REAL NOT NULL DEFAULT 0,
    cost REAL NOT NULL DEFAULT 0,
    explicit_weight REAL NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'live' CHECK (status IN ('live', 'stale', 'archived')),
    superseded_by INTEGER REFERENCES memories(id),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    last_access INTEGER NOT NULL DEFAULT (unixepoch()),
    access_count INTEGER NOT NULL DEFAULT 0,
    helpful_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE memory_anchors (
    memory_id INTEGER NOT NULL REFERENCES memories(id),
    repo_id INTEGER NOT NULL REFERENCES repos(id),
    relpath TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    xxh64 TEXT NOT NULL
);
CREATE INDEX memory_anchors_by_file ON memory_anchors(repo_id, relpath);

CREATE TABLE memory_links (
    memory_id INTEGER NOT NULL REFERENCES memories(id),
    related_id INTEGER NOT NULL REFERENCES memories(id),
    relation TEXT NOT NULL,
    UNIQUE (memory_id, related_id, relation)
);

CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
