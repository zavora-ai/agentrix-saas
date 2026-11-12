-- Missions (07 §3.5, 01b C16-C18; W5). Writer: agentrix-shell. A mission is a contract the owner
-- approves: a goal, what counts, who works on it, an end date. It never raises any job's level.
-- `state`: proposed (its card waits), active, paused, ended. `measure`: orders, questions or none.
CREATE TABLE IF NOT EXISTS shell_missions (
    id          TEXT PRIMARY KEY,
    site_id     TEXT NOT NULL,
    goal        TEXT NOT NULL,
    measure     TEXT NOT NULL,
    target      INTEGER NOT NULL,
    members     TEXT NOT NULL,
    lead        TEXT NOT NULL,
    ends_at     TEXT NOT NULL,
    state       TEXT NOT NULL,
    card_id     TEXT,
    receipt     TEXT,
    created_at  TEXT NOT NULL,
    started_at  TEXT,
    ended_at    TEXT
);
CREATE INDEX IF NOT EXISTS shell_missions_site ON shell_missions (site_id, state);
