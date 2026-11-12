-- Customer accounts on each site's own host (W6; 01a §2.4, 06 §4.10, 03 §1.5).
-- One writer: agentrix-identity's `visitor` module. Every row carries site_id: an account,
-- passkey or session never works on another site. Times are Unix seconds.
-- Additive only; ids are typed text (`vac_…`, `cr_…`, `vse_…`).

CREATE TABLE IF NOT EXISTS visitor_accounts (
  id                 TEXT PRIMARY KEY,
  site_id            TEXT NOT NULL,
  email              TEXT NOT NULL COLLATE NOCASE,
  name               TEXT NOT NULL,
  phone              TEXT,
  -- The WebAuthn user handle (a UUID), fixed for the life of the account.
  user_handle        TEXT NOT NULL,
  email_verified_at  INTEGER,
  created_at         INTEGER NOT NULL,
  UNIQUE (site_id, email)
);

-- Passkeys: webauthn-rs's Passkey as JSON (credential id, public key, counter). Public data.
CREATE TABLE IF NOT EXISTS visitor_credentials (
  id            TEXT PRIMARY KEY,           -- the credential id, base64url
  site_id       TEXT NOT NULL,
  account_id    TEXT NOT NULL REFERENCES visitor_accounts(id) ON DELETE CASCADE,
  passkey       TEXT NOT NULL,
  label         TEXT NOT NULL,
  created_at    INTEGER NOT NULL,
  last_used_at  INTEGER
);
CREATE INDEX IF NOT EXISTS visitor_credentials_account ON visitor_credentials(account_id);

-- Recovery codes: only SHA-256(account id ‖ code) is stored. Each works once.
CREATE TABLE IF NOT EXISTS visitor_recovery_codes (
  account_id  TEXT NOT NULL REFERENCES visitor_accounts(id) ON DELETE CASCADE,
  code_hash   TEXT NOT NULL,
  site_id     TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  used_at     INTEGER,
  PRIMARY KEY (account_id, code_hash)
);

-- Sessions: only the SHA-256 of the cookie's token is stored.
CREATE TABLE IF NOT EXISTS visitor_sessions (
  token_hash        TEXT PRIMARY KEY,
  id                TEXT NOT NULL UNIQUE,   -- vse_…, what the security page names
  site_id           TEXT NOT NULL,
  account_id        TEXT NOT NULL REFERENCES visitor_accounts(id) ON DELETE CASCADE,
  method            TEXT NOT NULL,          -- passkey | email-code | recovery-code
  device            TEXT NOT NULL,          -- "Chrome on a Mac", from the User-Agent
  created_at        INTEGER NOT NULL,
  last_seen_at      INTEGER NOT NULL,
  authenticated_at  INTEGER NOT NULL,       -- for "sign in again" before big changes
  expires_at        INTEGER NOT NULL,
  ended_at          INTEGER
);
CREATE INDEX IF NOT EXISTS visitor_sessions_account ON visitor_sessions(account_id);

-- Sign-in history (13 months, 03 §1.5) and the "New sign-in. Not you?" alert.
CREATE TABLE IF NOT EXISTS visitor_sign_in_events (
  id            TEXT PRIMARY KEY,
  site_id       TEXT NOT NULL,
  account_id    TEXT NOT NULL REFERENCES visitor_accounts(id) ON DELETE CASCADE,
  at            INTEGER NOT NULL,
  method        TEXT NOT NULL,
  device        TEXT NOT NULL,
  new_device    INTEGER NOT NULL DEFAULT 0,
  dismissed_at  INTEGER
);
CREATE INDEX IF NOT EXISTS visitor_sign_in_events_account ON visitor_sign_in_events(account_id, at);

-- Consent receipts (A-08): the body is the receipt's RFC 8785 JSON; `receipt` is the digest of
-- the platform receipt that signs it (control's key, `consent.given`). Withdrawal keeps the row
-- with both times.
CREATE TABLE IF NOT EXISTS consent_receipts (
  id            TEXT PRIMARY KEY,           -- cr_…
  site_id       TEXT NOT NULL,
  account_id    TEXT NOT NULL REFERENCES visitor_accounts(id) ON DELETE CASCADE,
  purpose       TEXT NOT NULL,              -- order-updates | account-memory | offers
  body          TEXT NOT NULL,              -- RFC 8785 JSON of the receipt
  digest        TEXT NOT NULL,              -- sha256:… of body
  receipt       TEXT,                       -- digest of the signed platform receipt
  withdrawn_receipt TEXT,                   -- digest of the `consent.withdrawn` receipt
  given_at      INTEGER NOT NULL,
  withdrawn_at  INTEGER
);
CREATE INDEX IF NOT EXISTS consent_receipts_account ON consent_receipts(account_id, purpose);
