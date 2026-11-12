-- Sign-in tables (design §5, R2). One writer: the control-plane lane's auth module.
-- The data lane's migrator includes this file, so the schema has one source.
-- Times are Unix seconds. Account ids are UUID v7 strings.

CREATE TABLE IF NOT EXISTS accounts (
  id             TEXT PRIMARY KEY,
  email          TEXT NOT NULL UNIQUE COLLATE NOCASE,
  display_name   TEXT NOT NULL,
  -- argon2id in PHC form; NULL for an account that signs in with passkeys only.
  password_hash  TEXT,
  created_at     INTEGER NOT NULL
);

-- Passkeys: webauthn-rs's Passkey as JSON (credential id, public key, counter). Public data only.
CREATE TABLE IF NOT EXISTS credentials (
  id            TEXT PRIMARY KEY,           -- the credential id, base64url
  account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  passkey       TEXT NOT NULL,
  label         TEXT NOT NULL,
  created_at    INTEGER NOT NULL,
  last_used_at  INTEGER
);
CREATE INDEX IF NOT EXISTS credentials_account ON credentials(account_id);

-- TOTP seeds, sealed (R2.3): ChaCha20-Poly1305 with the node's sealing key, nonce ‖ ciphertext,
-- with the account id as associated data. last_step makes every code good once.
CREATE TABLE IF NOT EXISTS totp_seeds (
  account_id   TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  sealed_seed  BLOB NOT NULL,
  last_step    INTEGER NOT NULL DEFAULT 0,
  created_at   INTEGER NOT NULL
);

-- Sessions: only the SHA-256 of the cookie's token is stored.
CREATE TABLE IF NOT EXISTS sessions (
  token_hash  TEXT PRIMARY KEY,
  account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  method      TEXT NOT NULL,                -- passkey | password+totp
  created_at  INTEGER NOT NULL,
  expires_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_account ON sessions(account_id);
