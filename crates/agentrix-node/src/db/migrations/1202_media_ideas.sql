-- W11 images (range 1200–1299). Writer: agentrix-media (`ideas`). Readers: the sample sites
-- (W2's cake studio and the owner's held requests).
--
-- An idea is a picture of what one visitor is designing ("Show me an idea of it" in the cake
-- studio, ADR-032). Its brief is composed by the server from the owner's options only. Ideas
-- are kept apart from `media_images` on purpose: they never wait for the owner's approval,
-- never become a site's own picture and never appear in the Pictures library. `key` is a hash
-- of the brief, so the same design is drawn once per site. The bytes live in the content store
-- under `digest`. A picture the safety check stopped is not kept here. No foreign keys to other
-- workstreams' tables: `site_id` names a site by id.
CREATE TABLE media_ideas (
    idea_id        TEXT PRIMARY KEY,
    site_id        TEXT NOT NULL,
    key            TEXT NOT NULL,
    kind           TEXT NOT NULL,
    digest         TEXT NOT NULL,
    media_type     TEXT NOT NULL,
    width          INTEGER,
    height         INTEGER,
    bytes          INTEGER NOT NULL CHECK (bytes >= 0),
    alt            TEXT NOT NULL,
    label          TEXT NOT NULL,
    shows_text     INTEGER NOT NULL CHECK (shows_text IN (0, 1)),
    brief          TEXT NOT NULL,
    prompt         TEXT NOT NULL,
    model          TEXT NOT NULL DEFAULT '',
    seed           INTEGER,
    screen         TEXT NOT NULL DEFAULT '{}',
    asked_by       TEXT NOT NULL,
    cost_micro_usd INTEGER NOT NULL DEFAULT 0 CHECK (cost_micro_usd >= 0),
    created_at     TEXT NOT NULL,
    UNIQUE (site_id, key)
) STRICT;
CREATE INDEX media_ideas_digest ON media_ideas (digest);
CREATE INDEX media_ideas_made ON media_ideas (site_id, created_at);
