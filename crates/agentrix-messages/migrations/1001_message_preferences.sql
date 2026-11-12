-- W9 · Messages: which channels an owner wants each family of operational messages on.
-- One writer: agentrix-messages (prefs.rs). A missing row means on. Security messages,
-- receipts, invoices and P1 alarms ignore this table: they always arrive.
CREATE TABLE IF NOT EXISTS message_preferences (
    account_id TEXT NOT NULL,
    family     TEXT NOT NULL,
    channel    TEXT NOT NULL CHECK (channel IN ('email', 'push', 'chat')),
    enabled    INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account_id, family, channel)
) STRICT;
