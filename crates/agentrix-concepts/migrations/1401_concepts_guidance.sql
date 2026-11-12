-- The concept stage, second slice: agents write index.html in rooms kept until the owner
-- chooses, an automatic check-and-fix loop, the owner's own guidance, and the evidence the
-- page shows for each run.

-- A run's form token (choose and guide posts carry it), the planner as Vultr reported it,
-- the longest prompt sent, and when its rooms were deleted.
ALTER TABLE concept_runs ADD COLUMN csrf TEXT NOT NULL DEFAULT '';
ALTER TABLE concept_runs ADD COLUMN planner_seen TEXT NOT NULL DEFAULT '';
ALTER TABLE concept_runs ADD COLUMN prompt_chars_max INTEGER NOT NULL DEFAULT 0;
ALTER TABLE concept_runs ADD COLUMN rooms_deleted_at TEXT;

-- Per design: the provider Vultr reported, check-and-fix rounds, what the last check found
-- missing (JSON array), tools refused by name (JSON object), fs_write calls, files checked
-- for the key, the design's receipt and its model calls.
ALTER TABLE concepts ADD COLUMN provider TEXT NOT NULL DEFAULT '';
ALTER TABLE concepts ADD COLUMN rounds INTEGER NOT NULL DEFAULT 0;
ALTER TABLE concepts ADD COLUMN missing TEXT NOT NULL DEFAULT '[]';
ALTER TABLE concepts ADD COLUMN refused TEXT NOT NULL DEFAULT '{}';
ALTER TABLE concepts ADD COLUMN writes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE concepts ADD COLUMN key_checked_files INTEGER NOT NULL DEFAULT 0;
ALTER TABLE concepts ADD COLUMN page_receipt TEXT;
-- Every model call the designer made, recorded when it starts: [{provider, model, kind, ms}],
-- kind build, fix or owner; ms filled from Zavora's result (a call stopped by the cap has none).
ALTER TABLE concepts ADD COLUMN calls TEXT NOT NULL DEFAULT '[]';

-- The owner's own guidance to one designer ("Tell this designer what to change").
CREATE TABLE IF NOT EXISTS concept_changes (
    change_id    TEXT PRIMARY KEY,
    concept_id   TEXT NOT NULL REFERENCES concepts (concept_id),
    site_id      TEXT NOT NULL,
    account      TEXT NOT NULL,
    words        TEXT NOT NULL,
    state        TEXT NOT NULL CHECK (state IN ('running', 'done', 'failed')),
    note         TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL,
    finished_at  TEXT
);
CREATE INDEX IF NOT EXISTS concept_changes_site ON concept_changes (site_id, created_at);
