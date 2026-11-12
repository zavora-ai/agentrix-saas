-- Chat channels (W13, range 1300–1399). Writer: agentrix-channels.
-- Run by agentrix_channels::migrate with its own table, _sqlx_migrations_agentrix_channels.
-- No foreign keys across workstreams: account_id and site_id refer by id.

-- One-time codes for linking a chat (purpose 'telegram') or pairing a phone ('app'). Only the
-- code's SHA-256 is stored; a code is single use and expires.
CREATE TABLE IF NOT EXISTS channel_link_codes (
    code_hash  TEXT PRIMARY KEY,
    purpose    TEXT NOT NULL CHECK (purpose IN ('telegram', 'app')),
    account_id TEXT NOT NULL,
    site_id    TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_at    TEXT
);
CREATE INDEX IF NOT EXISTS channel_link_codes_owner ON channel_link_codes (account_id, site_id, purpose);

-- A chat linked to an owner's site. One active link per (channel, site, account).
CREATE TABLE IF NOT EXISTS channel_links (
    id           TEXT PRIMARY KEY,
    channel      TEXT NOT NULL CHECK (channel IN ('telegram', 'whatsapp')),
    bot          TEXT NOT NULL,
    chat_id      TEXT NOT NULL,
    account_id   TEXT NOT NULL,
    site_id      TEXT NOT NULL,
    label        TEXT NOT NULL,
    linked_at    TEXT NOT NULL,
    unlinked_at  TEXT,
    unlinked_how TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS channel_links_active ON channel_links (channel, site_id, account_id) WHERE unlinked_at IS NULL;
CREATE INDEX IF NOT EXISTS channel_links_chat ON channel_links (channel, bot, chat_id) WHERE unlinked_at IS NULL;

-- The getUpdates offset per bot, so each update is handled once across restarts.
CREATE TABLE IF NOT EXISTS channel_cursors (
    bot        TEXT PRIMARY KEY,
    next       INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);

-- What was announced to which link (a card, a brief day, a question, a freeze), so each goes once.
CREATE TABLE IF NOT EXISTS channel_alerts (
    link_id   TEXT NOT NULL,
    kind      TEXT NOT NULL,
    subject   TEXT NOT NULL,
    at        TEXT NOT NULL,
    outbox_id TEXT,
    PRIMARY KEY (link_id, kind, subject)
);
