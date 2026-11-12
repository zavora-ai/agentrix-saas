-- "Allow, and tell me" (04-ui-parity §3 item 7). Writer: agentrix-decisions. Additive only.
--
-- A standing grant the owner signed while approving a card: later cards of the same kind from
-- the same WebAgent (and, for a person grant, for the same person) are approved under it, up to
-- the card's own amount, until it ends or the owner stops it. `terms` is standing::GrantTerms as
-- JSON; the approval that made it signed those terms with the card. Stopping one keeps the row.
CREATE TABLE IF NOT EXISTS decision_grants (
    id              TEXT PRIMARY KEY,
    site_id         TEXT NOT NULL,
    webagent        TEXT NOT NULL,
    kind            TEXT NOT NULL,
    terms           TEXT NOT NULL,
    from_card       TEXT NOT NULL,
    created_by      TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    expires_at      TEXT NOT NULL,
    receipt         TEXT NOT NULL,
    uses            INTEGER NOT NULL DEFAULT 0,
    revoked_at      TEXT,
    revoked_receipt TEXT
);
CREATE INDEX IF NOT EXISTS decision_grants_site ON decision_grants (site_id, webagent, kind, revoked_at);

-- The grant a card made (when the owner approved it with "Allow, and tell me") or was approved
-- under (method 'grant').
ALTER TABLE decision_cards_v2 ADD COLUMN grant_id TEXT;
