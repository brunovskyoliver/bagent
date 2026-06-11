-- Embeddings store for semantic search (requires sqlite-vec extension loaded at runtime)
CREATE TABLE IF NOT EXISTS embeddings (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id   TEXT NOT NULL,
    namespace TEXT NOT NULL,
    model     TEXT NOT NULL,
    dim       INTEGER NOT NULL,
    vector    BLOB NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS embeddings_namespace ON embeddings(namespace, item_id);
CREATE UNIQUE INDEX IF NOT EXISTS embeddings_item_model ON embeddings(item_id, namespace, model);

-- sqlite-vec virtual table created at runtime (extension must be loaded first):
-- CREATE VIRTUAL TABLE vec_items USING vec0(
--     item_id TEXT,
--     namespace TEXT,
--     embedding FLOAT[1024]
-- );
--
-- The daemon creates this table dynamically after loading the extension.
-- This migration only creates the backing storage table above.
