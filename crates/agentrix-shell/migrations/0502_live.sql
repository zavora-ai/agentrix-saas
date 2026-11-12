-- Live (concept/console/live.html): Front of house conversations the owner can sit in on, take
-- over as a person and hand back. A site's Ask writes the turns through agentrix_shell::live;
-- the owner's console reads them. Visitors' words stay with Front of house: the back office never
-- reads these tables.
CREATE TABLE shell_live_threads (
    id            TEXT PRIMARY KEY,
    site_id       TEXT NOT NULL,
    page          TEXT NOT NULL,
    visitor       TEXT NOT NULL,
    desk          TEXT NOT NULL,
    -- 'desk' or 'owner'. An owner who stays quiet for 10 minutes hands it back by themselves.
    mode          TEXT NOT NULL DEFAULT 'desk',
    owner_name    TEXT,
    owner_last_at TEXT,
    started_at    TEXT NOT NULL,
    last_at       TEXT NOT NULL
);
CREATE INDEX shell_live_threads_site ON shell_live_threads (site_id, last_at);

CREATE TABLE shell_live_turns (
    thread_id TEXT NOT NULL REFERENCES shell_live_threads (id),
    seq       INTEGER NOT NULL,
    at        TEXT NOT NULL,
    -- 'visitor', 'desk', 'owner' or 'note' (joined, handed back).
    who       TEXT NOT NULL,
    name      TEXT NOT NULL,
    text      TEXT NOT NULL CHECK (length(text) BETWEEN 1 AND 2000),
    cites     TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (thread_id, seq)
);
