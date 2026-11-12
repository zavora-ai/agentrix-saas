-- W1 · The individual WebAgent (agentrix-kernel). Writer: agentrix-kernel only.
-- Its own migrator (_sqlx_migrations_agentrix_kernel), range 0100-0199. No foreign keys to
-- other workstreams' tables; sites are referred to by id.

-- A site's team as the kernel knows it: the look its marks wear, who is on duty and the
-- Concierge's section desks. Written when a sample is seeded or the owner changes the team.
-- A moved Brick 1 site without a row here gets its look from its signed pages' tokens.
CREATE TABLE webagent_sites (
    site_id    TEXT PRIMARY KEY,
    look       TEXT NOT NULL,          -- SiteLook JSON
    on_duty    TEXT NOT NULL,          -- JSON list of WebAgent ids
    desks      TEXT NOT NULL,          -- JSON list of DeskDraft
    updated_at TEXT NOT NULL
) STRICT;

-- Owner-approved versions of one WebAgent's spec (a new mark from the Workshop, a raised or
-- lowered dial). The newest version wins over the built-in template; old ones stay as history.
CREATE TABLE webagent_specs (
    site_id     TEXT NOT NULL,
    webagent    TEXT NOT NULL,
    version     INTEGER NOT NULL CHECK (version >= 1),
    body        TEXT NOT NULL,         -- WebAgentSpec JSON
    reason      TEXT NOT NULL,         -- what changed, in plain words
    decision_id TEXT,                  -- the owner's decision that approved it, when one did
    created_at  TEXT NOT NULL,
    PRIMARY KEY (site_id, webagent, version)
) STRICT;
