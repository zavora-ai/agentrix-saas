-- W1 · an owner's change to one job's dial. Lowering applies at once; raising waits for the
-- owner's passkey or authenticator code over this change's fingerprint (the frozen approve
-- contract, through agentrix-host's Approvals). Writer: agentrix-kernel only.
CREATE TABLE webagent_dial_changes (
    change_id    TEXT PRIMARY KEY,
    site_id      TEXT NOT NULL,
    webagent     TEXT NOT NULL,
    job          TEXT NOT NULL,
    from_level   TEXT NOT NULL,
    to_level     TEXT NOT NULL,
    fingerprint  TEXT NOT NULL,        -- hex SHA-256 of the change's canonical JSON
    state        TEXT NOT NULL CHECK (state IN ('waiting', 'applied', 'declined', 'expired')),
    requested_by TEXT NOT NULL,        -- the owner's account
    method       TEXT,                 -- how it was approved: signed-in (lowering), passkey, totp
    receipt      TEXT,                 -- the receipt's digest once applied
    created_at   TEXT NOT NULL,
    expires_at   TEXT NOT NULL,
    decided_at   TEXT
) STRICT;
CREATE INDEX webagent_dial_changes_site ON webagent_dial_changes (site_id, state);
