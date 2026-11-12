-- Claims (design §5, R3). One writer: the control-plane lane's claim module.
-- The data lane's migrator includes this file. Times are Unix seconds.

CREATE TABLE IF NOT EXISTS claims (
  id           TEXT PRIMARY KEY,           -- UUID v7
  site_id      TEXT NOT NULL,
  account_id   TEXT NOT NULL,
  domain       TEXT NOT NULL,              -- lower-case ASCII (IDNA)
  code         TEXT NOT NULL,              -- shown to the owner: xxxx-xxxx-xxxx-xxxx
  state        TEXT NOT NULL,              -- pending | verified
  method       TEXT,                       -- dns | file, once verified
  evidence     TEXT,                       -- JSON: what the sandbox saw when it last checked
  created_at   INTEGER NOT NULL,
  checked_at   INTEGER,
  verified_at  INTEGER
);
CREATE INDEX IF NOT EXISTS claims_site ON claims(site_id, state);
CREATE UNIQUE INDEX IF NOT EXISTS claims_one_open ON claims(site_id, account_id) WHERE state = 'pending';
