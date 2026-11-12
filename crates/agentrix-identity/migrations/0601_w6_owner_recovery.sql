-- Owner recovery (W6; 01b §4.1, plan decision 8): recovery codes at sign-up, sign-in by email
-- code for email-only accounts, and recovery by email held for 24 hours with a notice.
-- account_id is the owner's id in auth/'s `accounts` (Brick 1's table): referenced by id, no
-- foreign key across owners (03 §4.2). Times are Unix seconds.

-- Recovery codes: only SHA-256(account id ‖ code) is stored. Each works once.
CREATE TABLE IF NOT EXISTS owner_recovery_codes (
  account_id  TEXT NOT NULL,
  code_hash   TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  used_at     INTEGER,
  PRIMARY KEY (account_id, code_hash)
);

-- Recovery by email: nothing happens until release_at; the owner can cancel before then.
CREATE TABLE IF NOT EXISTS owner_recovery_holds (
  id            TEXT PRIMARY KEY,           -- rh_…
  account_id    TEXT NOT NULL,
  requested_at  INTEGER NOT NULL,
  release_at    INTEGER NOT NULL,
  device        TEXT NOT NULL,
  state         TEXT NOT NULL,              -- held | cancelled | completed
  ended_at      INTEGER
);
CREATE INDEX IF NOT EXISTS owner_recovery_holds_account ON owner_recovery_holds(account_id, state);

-- Every owner sign-in by email code or recovery code, with its platform receipt.
CREATE TABLE IF NOT EXISTS owner_code_sign_ins (
  id          TEXT PRIMARY KEY,             -- ocs_…
  account_id  TEXT NOT NULL,
  at          INTEGER NOT NULL,
  method      TEXT NOT NULL,                -- email-code | recovery-code | recovery-hold
  device      TEXT NOT NULL,
  ip_hash     TEXT NOT NULL,
  receipt     TEXT
);
CREATE INDEX IF NOT EXISTS owner_code_sign_ins_account ON owner_code_sign_ins(account_id, at);
