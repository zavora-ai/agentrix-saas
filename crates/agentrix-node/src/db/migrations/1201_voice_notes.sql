-- W11 voice (range 1200–1299). Writer: agentrix-media. The note of each spoken answer to the
-- Chief WebAgent: what was heard and what it did. Never the recording (adk-surface's voice
-- rule: "We keep a note of your answer, never the recording"). No foreign keys to other
-- workstreams' tables.
CREATE TABLE voice_notes (
    note_id  INTEGER PRIMARY KEY,
    site_id  TEXT NOT NULL,
    heard    TEXT NOT NULL,
    intent   TEXT,
    card_id  TEXT,
    did      TEXT NOT NULL,
    at       TEXT NOT NULL
) STRICT;
CREATE INDEX voice_notes_site ON voice_notes (site_id, at);
