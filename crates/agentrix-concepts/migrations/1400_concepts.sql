-- The concept stage (W0 integration): three looks designed for a claimed site, and the one the
-- owner chose. Refers to sites and accounts by id only. Times are RFC 3339 UTC text.

-- One row per "Design three looks" press.
CREATE TABLE IF NOT EXISTS concept_runs (
    run_id         TEXT PRIMARY KEY,
    site_id        TEXT NOT NULL,
    account        TEXT NOT NULL,
    state          TEXT NOT NULL CHECK (state IN ('planning', 'designing', 'done', 'failed')),
    planner_model  TEXT NOT NULL,
    worker_model   TEXT NOT NULL,
    note           TEXT NOT NULL DEFAULT '',
    started_at     TEXT NOT NULL,
    finished_at    TEXT,
    duration_ms    INTEGER
);
CREATE INDEX IF NOT EXISTS concept_runs_site ON concept_runs (site_id, started_at);

-- One row per design: n is 1, 2 or 3 within its run.
CREATE TABLE IF NOT EXISTS concepts (
    concept_id     TEXT PRIMARY KEY,
    run_id         TEXT NOT NULL REFERENCES concept_runs (run_id),
    site_id        TEXT NOT NULL,
    n              INTEGER NOT NULL CHECK (n BETWEEN 1 AND 3),
    name           TEXT NOT NULL,
    direction      TEXT NOT NULL,
    status         TEXT NOT NULL CHECK (status IN ('waiting', 'writing', 'done', 'failed')),
    note           TEXT NOT NULL DEFAULT '',
    html_digest    TEXT,
    html_chars     INTEGER NOT NULL DEFAULT 0,
    model          TEXT NOT NULL,
    started_at     TEXT,
    finished_at    TEXT,
    duration_ms    INTEGER,
    chosen         INTEGER NOT NULL DEFAULT 0 CHECK (chosen IN (0, 1)),
    chosen_at      TEXT,
    receipt        TEXT,
    UNIQUE (run_id, n)
);
CREATE INDEX IF NOT EXISTS concepts_site ON concepts (site_id, chosen);
