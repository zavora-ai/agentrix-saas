-- W9 · Messages (crates/agentrix-messages, its own migrator). Every message the platform sends or would send,
-- recorded before delivery. One writer: agentrix-messages (outbox.rs).
--
-- One row per message per channel. The bodies are the stored copy: one-time codes and other
-- secrets the catalogue names are replaced before the row is written, so this table never
-- holds a usable code. Times are RFC 3339 UTC text with microseconds, as in 0001.
CREATE TABLE IF NOT EXISTS message_outbox (
    id              TEXT PRIMARY KEY,
    site_id         TEXT,
    account_id      TEXT,
    message         TEXT NOT NULL,
    audience        TEXT NOT NULL,
    kind            TEXT NOT NULL,
    priority        TEXT NOT NULL,
    channel         TEXT NOT NULL CHECK (channel IN ('email', 'push', 'chat')),
    recipient       TEXT NOT NULL,
    recipient_label TEXT NOT NULL,
    reason          TEXT NOT NULL,
    from_name       TEXT NOT NULL,
    subject         TEXT NOT NULL,
    body_text       TEXT NOT NULL,
    body_html       TEXT,
    payload         TEXT,
    dedupe_key      TEXT,
    status          TEXT NOT NULL CHECK (status IN ('queued', 'held', 'sent', 'not_sent', 'failed', 'suppressed')),
    status_reason   TEXT,
    hold_until      TEXT,
    provider        TEXT,
    provider_ref    TEXT,
    attempts        INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    sent_at         TEXT
) STRICT;
CREATE UNIQUE INDEX IF NOT EXISTS message_outbox_dedupe ON message_outbox (dedupe_key, channel) WHERE dedupe_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS message_outbox_site ON message_outbox (site_id, created_at);
CREATE INDEX IF NOT EXISTS message_outbox_account ON message_outbox (account_id, created_at);
CREATE INDEX IF NOT EXISTS message_outbox_due ON message_outbox (status, hold_until);
