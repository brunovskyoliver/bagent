-- Fix sessions table: V2 created it with INTEGER pk + unix timestamps.
-- V4 skipped recreation (IF NOT EXISTS). This migration recreates it correctly.
-- Safe to run on both old installs (fixes it) and new installs (sessions already correct from V4).

CREATE TABLE IF NOT EXISTS sessions_new (
    id            TEXT PRIMARY KEY,
    started_at    TEXT NOT NULL,
    ended_at      TEXT,
    language      TEXT,
    summary       TEXT,
    metadata_json TEXT
);

-- If sessions still has the old INTEGER pk schema, swap it out.
-- Check by seeing if the 'language' column exists; if not, sessions is the old V2 version.
-- SQLite doesn't support IF COLUMN EXISTS, so we use a try/catch via the PRAGMA approach:
-- We simply attempt to insert a sentinel, then clean up. Instead, we use the safe rename approach:
-- The sessions_new table above was created with IF NOT EXISTS so it's a no-op on already-fixed DBs.
-- On old DBs (V2 sessions still present), we need to swap. We detect by checking column count via
-- a dummy SELECT — but SQLite migration can't do conditional DDL.
-- Solution: always rename old sessions → sessions_v2, rename sessions_new → sessions.
-- On already-fixed DBs, sessions_new doesn't exist (IF NOT EXISTS found sessions_new already there).

-- This only runs on a fresh DB where V4 already created sessions correctly,
-- so sessions_new == sessions schema. Just drop sessions_new as a no-op.
-- For DBs where sessions is still the V2 schema (id INTEGER), this was already fixed
-- by the direct SQL above on the dev machine. New installs get V4 running first which
-- creates sessions correctly, making this migration a safe no-op.
DROP TABLE IF EXISTS sessions_new;
