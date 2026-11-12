-- Agentrix SaaS settlement (saas-settle), migrations 1700–1709, own table.

-- Each cart mandate moves money once. A row is claimed before any money moves; a settlement
-- that PayPal refused releases it (state 'failed'), one whose outcome is unknown keeps it.
CREATE TABLE IF NOT EXISTS settle_carts (
    cart_mandate_id   TEXT PRIMARY KEY,
    cart_digest       TEXT NOT NULL UNIQUE,
    intent_mandate_id TEXT NOT NULL,
    invoice_id        TEXT NOT NULL,
    payer             TEXT NOT NULL,
    currency          TEXT NOT NULL,
    amount_minor      INTEGER NOT NULL,
    month             TEXT NOT NULL,
    route             TEXT NOT NULL,
    state             TEXT NOT NULL,          -- claimed | settled | failed | unknown
    paypal_key        TEXT,
    receipt_json      TEXT,
    error             TEXT,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS settle_carts_month ON settle_carts (intent_mandate_id, month);
CREATE INDEX IF NOT EXISTS settle_carts_invoice ON settle_carts (invoice_id);

-- Agreed instalment plans and their scheduled collections.
CREATE TABLE IF NOT EXISTS settle_plans (
    plan_id        TEXT PRIMARY KEY,
    invoice_id     TEXT NOT NULL,
    payer          TEXT NOT NULL,
    intent_json    TEXT NOT NULL,
    plan_json      TEXT NOT NULL,
    agreement_json TEXT NOT NULL,
    state          TEXT NOT NULL,             -- active | paid
    created_at     INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS settle_schedule (
    plan_id         TEXT NOT NULL,
    idx             INTEGER NOT NULL,
    due_on          TEXT NOT NULL,
    currency        TEXT NOT NULL,
    amount_minor    INTEGER NOT NULL,
    cart_json       TEXT NOT NULL,
    cart_mandate_id TEXT NOT NULL,
    state           TEXT NOT NULL,            -- scheduled | settled | failed
    receipt_json    TEXT,
    PRIMARY KEY (plan_id, idx)
);
CREATE INDEX IF NOT EXISTS settle_schedule_due ON settle_schedule (state, due_on);

-- The payer's saved PayPal wallet (Vault), held by the payee that captures from it.
CREATE TABLE IF NOT EXISTS settle_vault (
    payer         TEXT PRIMARY KEY,
    vault_id      TEXT,
    customer_id   TEXT,
    setup_token   TEXT,
    approve_url   TEXT,
    state         TEXT NOT NULL,              -- pending | saved
    updated_at    INTEGER NOT NULL
);

-- Request nonces seen, so a captured signed request can't be replayed inside its lifetime.
CREATE TABLE IF NOT EXISTS settle_nonces (
    origin     TEXT NOT NULL,
    nonce      TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (origin, nonce)
);

-- The payer's side: bills offered to it and what it did.
CREATE TABLE IF NOT EXISTS settle_offers (
    invoice_id   TEXT NOT NULL,
    payee        TEXT NOT NULL,
    invoice_json TEXT NOT NULL,
    decision     TEXT NOT NULL,
    receipt_json TEXT,
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (payee, invoice_id)
);
