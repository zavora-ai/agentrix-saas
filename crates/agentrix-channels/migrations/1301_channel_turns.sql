-- Chat turns (W13): what a customer or owner sent and what the bot answered, kept 30 days so the
-- owner's Live view and support can see what was said. The question is cut to 200 characters.
CREATE TABLE IF NOT EXISTS channel_turns (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    channel        TEXT NOT NULL,
    bot            TEXT NOT NULL,
    chat_id        TEXT NOT NULL,
    role           TEXT NOT NULL CHECK (role IN ('owner', 'customer')),
    desk           TEXT,
    asked          TEXT NOT NULL,
    answered       TEXT NOT NULL,
    in_message_id  INTEGER,
    out_message_id INTEGER,
    at             TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS channel_turns_chat ON channel_turns (channel, bot, chat_id, at);
