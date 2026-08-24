-- V18: mirror of macOS Notification Center entries.
--
-- Every source-derived column is nullable and `raw_json` keeps the decoded
-- record whole. The usernoted schema is private and still unverified, so a
-- surprise on Apple's side must cost a code change, not another migration.
CREATE TABLE IF NOT EXISTS notifications (
    id            INTEGER PRIMARY KEY,
    source_id     TEXT    NOT NULL UNIQUE,  -- usernoted row identity; dedupes re-polls
    app_bundle_id TEXT,
    app_name      TEXT,
    title         TEXT,
    subtitle      TEXT,
    body          TEXT,
    thread_id     TEXT,
    delivered_at  INTEGER,                  -- unix seconds
    -- Lowercased haystack, folded in Rust. SQLite's own lower()/LIKE are
    -- ASCII-only, so 'ČAU' LIKE '%čau%' is false — unusable for Slovak.
    search_text   TEXT,
    app_text      TEXT,                     -- folded app name + bundle id
    ingested_at   INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS notifications_delivered
    ON notifications(delivered_at DESC);

CREATE INDEX IF NOT EXISTS notifications_thread
    ON notifications(app_bundle_id, thread_id);

-- Collector progress, kept apart from the mirror on purpose: "forget all"
-- empties `notifications` but must NOT rewind the watermark, or the next poll
-- would re-ingest everything Apple still holds and undo the erasure.
CREATE TABLE IF NOT EXISTS notifications_state (
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    watermark INTEGER   -- newest delivered_at ever ingested
);

INSERT OR IGNORE INTO notifications_state (id, watermark) VALUES (1, NULL);

-- Feed state lives in `connectors`: enabled=0 until the user opts in, and
-- last_sync_at NULL means the collector has never run (feed unavailable).
INSERT OR IGNORE INTO connectors (kind, config_json, enabled, last_sync_at)
VALUES ('notifications', '{}', 0, NULL);
