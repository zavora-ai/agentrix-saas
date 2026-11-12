-- saas-badge: PayPal bindings, from PayPal's Vault answer to the owner's signed record.
CREATE TABLE IF NOT EXISTS badge_bindings (
    binding_id    TEXT PRIMARY KEY,
    site          TEXT NOT NULL,
    setup_token   TEXT,
    state         TEXT NOT NULL,      -- awaiting_paypal | awaiting_passkey | bound | revoked
    draft_json    TEXT,
    fingerprint   TEXT,
    signed_json   TEXT,
    audit_ref     TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS badge_bindings_site ON badge_bindings (site, state);
CREATE INDEX IF NOT EXISTS badge_bindings_token ON badge_bindings (setup_token);
