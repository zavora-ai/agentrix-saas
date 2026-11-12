-- W11 images (range 1200–1299). Writer: agentrix-media. Readers: the console's media library
-- (W5), the sample sites (W2), the home page (W10).
--
-- One row per picture made, uploaded or carried over for a site. The bytes live in the content
-- store under `digest`; a picture the safety check stopped keeps no bytes (`digest` is '').
-- A picture reaches a site only once the owner approves it (`status` 'approved'); approving
-- one marks the slot's earlier picture 'replaced'. `label` is what a visitor sees on it, set
-- wherever a visitor could take a generated picture for a photograph. No foreign keys to other
-- workstreams' tables: `site_id` names a site by id.
CREATE TABLE media_images (
    image_id       TEXT PRIMARY KEY,
    site_id        TEXT NOT NULL,
    slot           TEXT NOT NULL,
    kind           TEXT NOT NULL,
    status         TEXT NOT NULL CHECK (status IN ('draft', 'approved', 'replaced', 'declined', 'blocked')),
    origin         TEXT NOT NULL CHECK (origin IN ('generated', 'edited', 'uploaded', 'moved')),
    digest         TEXT NOT NULL,
    media_type     TEXT NOT NULL,
    width          INTEGER,
    height         INTEGER,
    bytes          INTEGER NOT NULL CHECK (bytes >= 0),
    alt            TEXT NOT NULL,
    label          TEXT,
    photographic   INTEGER NOT NULL CHECK (photographic IN (0, 1)),
    shows_person   INTEGER NOT NULL CHECK (shows_person IN (0, 1)),
    shows_text     INTEGER NOT NULL CHECK (shows_text IN (0, 1)),
    owner_check    TEXT NOT NULL DEFAULT '',
    brief          TEXT NOT NULL DEFAULT '',
    prompt         TEXT NOT NULL DEFAULT '',
    model          TEXT NOT NULL DEFAULT '',
    seed           INTEGER,
    screen         TEXT NOT NULL DEFAULT '{}',
    asked_by       TEXT NOT NULL,
    replaces       TEXT,
    cost_micro_usd INTEGER NOT NULL DEFAULT 0 CHECK (cost_micro_usd >= 0),
    created_at     TEXT NOT NULL,
    decided_at     TEXT,
    decided_by     TEXT,
    receipt        TEXT
) STRICT;
CREATE INDEX media_images_site ON media_images (site_id, status, slot);
CREATE INDEX media_images_digest ON media_images (digest);
