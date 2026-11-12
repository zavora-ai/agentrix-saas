-- Decision card v2 (contract C4; W5, range 0500–0599). Writer: agentrix-decisions.
-- Run by agentrix_decisions::migrate with its own table, _sqlx_migrations_agentrix_decisions.
-- `card` is the DecisionCardV2 as JSON; `fingerprint` repeats card.fingerprint so a card changed
-- after it was raised is refused. Brick 1's own cards stay in `decisions`.
CREATE TABLE IF NOT EXISTS decision_cards_v2 (
    id          TEXT PRIMARY KEY,
    site_id     TEXT NOT NULL,
    kind        TEXT NOT NULL,
    card        TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    dedupe_key  TEXT,
    state       TEXT NOT NULL CHECK (state IN ('waiting', 'approved', 'declined', 'expired')),
    method      TEXT,
    decided_by  TEXT,
    decided_at  TEXT,
    receipt     TEXT,
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS decision_cards_v2_site ON decision_cards_v2 (site_id, state, created_at);
CREATE UNIQUE INDEX IF NOT EXISTS decision_cards_v2_dedupe ON decision_cards_v2 (site_id, dedupe_key) WHERE dedupe_key IS NOT NULL;
