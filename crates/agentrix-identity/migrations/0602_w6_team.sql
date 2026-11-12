-- Team members and roles for an owner's staff (W6; 01b §4.2, 07 §5.6). The owner stays
-- Brick 1's sites.owner_account_id (read only here). account_id is auth/'s accounts id,
-- referenced by id with no foreign key across owners (03 §4.2). Times are Unix seconds.

CREATE TABLE IF NOT EXISTS site_members (
  id           TEXT PRIMARY KEY,            -- mem_…
  site_id      TEXT NOT NULL,
  email        TEXT NOT NULL COLLATE NOCASE,
  account_id   TEXT,                        -- set when the invite is accepted
  role         TEXT NOT NULL,               -- manager | viewer | billing
  invited_by   TEXT NOT NULL,               -- the owner's account id
  invite_hash  TEXT,                        -- SHA-256 of the invite token; cleared on acceptance
  invited_at   INTEGER NOT NULL,
  expires_at   INTEGER NOT NULL,            -- invites last 7 days
  accepted_at  INTEGER,
  removed_at   INTEGER,
  receipt      TEXT                         -- the last team.changed receipt
);
CREATE UNIQUE INDEX IF NOT EXISTS site_members_live ON site_members(site_id, email) WHERE removed_at IS NULL;
CREATE INDEX IF NOT EXISTS site_members_account ON site_members(account_id) WHERE removed_at IS NULL;
