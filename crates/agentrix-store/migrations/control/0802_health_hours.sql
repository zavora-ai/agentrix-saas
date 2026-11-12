-- Health checks behind the status page (W8, agentrix-store::health). One row per check and UTC
-- hour, counting the minutes the check found working, slow or down, so availability over any
-- window is (ok + slow) / all minutes counted. Kept 90 days; content-free.
CREATE TABLE health_hours (
    check_id     TEXT NOT NULL CHECK (length(check_id) BETWEEN 1 AND 40),
    hour         TEXT NOT NULL CHECK (length(hour) = 13),
    ok_minutes   INTEGER NOT NULL DEFAULT 0 CHECK (ok_minutes >= 0),
    slow_minutes INTEGER NOT NULL DEFAULT 0 CHECK (slow_minutes >= 0),
    down_minutes INTEGER NOT NULL DEFAULT 0 CHECK (down_minutes >= 0),
    last_state   TEXT NOT NULL CHECK (last_state IN ('ok', 'slow', 'down')),
    last_detail  TEXT NOT NULL DEFAULT '',
    last_at      TEXT NOT NULL,
    PRIMARY KEY (check_id, hour)
) STRICT;
