-- Phones paired with an owner's site for the owner app (W13). A token is shown once to the app;
-- only its SHA-256 is kept. A device removed in Settings → Channels, or unused for 30 days, stops
-- working. A token never approves on its own: approvals still need a passkey or an
-- authenticator code bound to the card's fingerprint.
CREATE TABLE IF NOT EXISTS channel_devices (
    id           TEXT PRIMARY KEY,
    token_hash   TEXT NOT NULL UNIQUE,
    account_id   TEXT NOT NULL,
    site_id      TEXT NOT NULL,
    name         TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    last_used_at TEXT NOT NULL,
    removed_at   TEXT
);
CREATE INDEX IF NOT EXISTS channel_devices_owner ON channel_devices (account_id, site_id) WHERE removed_at IS NULL;
