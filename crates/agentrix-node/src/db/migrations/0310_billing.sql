-- Platform billing (W3, agentrix-billing; docs/08-control-plane.md §4.2). Range 0300–0399.
-- Additive only; no foreign keys to other workstreams' tables. Money in USD minor units (cents).
-- One writer: agentrix-billing. Collection happens in W6's Effects Gateway (PayPal
-- Subscriptions in sandbox); each provider event it verifies arrives through Billing::on_event.

-- One subscription per site. state follows W6's PayPal lifecycle. starts_at is go-live day: the
-- first charge is then (08 §4.2), so an owner who never goes live is never charged.
CREATE TABLE subscriptions (
    subscription_id TEXT PRIMARY KEY,
    site_id         TEXT NOT NULL,
    plan_id         TEXT NOT NULL,
    plan_version    INTEGER NOT NULL CHECK (plan_version >= 1),
    price_cents     INTEGER NOT NULL CHECK (price_cents >= 0),
    rail            TEXT NOT NULL DEFAULT 'paypal' CHECK (rail IN ('paypal')),
    provider_ref    TEXT UNIQUE,
    state           TEXT NOT NULL CHECK (state IN ('pending_approval', 'active', 'past_due',
                        'suspended', 'cancelled', 'expired')),
    period_end      TEXT,
    past_due_since  TEXT,
    starts_at       TEXT,
    cancel_at       TEXT,
    receipt         TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
) STRICT;
CREATE INDEX subscriptions_site ON subscriptions (site_id, state);

-- Every provider event, as the Effects Gateway reported it, with its receipt.
CREATE TABLE subscription_events (
    event_id            INTEGER PRIMARY KEY,
    subscription_id     TEXT NOT NULL,
    provider_ref        TEXT NOT NULL,
    kind                TEXT NOT NULL CHECK (kind IN ('activated', 'payment_completed',
                            'payment_failed', 'suspended', 'cancelled', 'expired')),
    amount_cents        INTEGER,
    provider_payment_id TEXT,
    at                  TEXT NOT NULL,
    period_end          TEXT,
    receipt             TEXT,
    UNIQUE (provider_ref, kind, provider_payment_id, at)
) STRICT;
CREATE INDEX subscription_events_sub ON subscription_events (subscription_id, at);

-- Invoices: issued by an approved invoice run, paid by a provider payment. number is
-- INV-YYYY-MM-NNNN. Never deleted; a mistake is a credit note (a negative invoice).
CREATE TABLE invoices (
    invoice_id      TEXT PRIMARY KEY,
    number          TEXT NOT NULL UNIQUE,
    site_id         TEXT NOT NULL,
    subscription_id TEXT,
    period          TEXT NOT NULL,
    currency        TEXT NOT NULL DEFAULT 'USD' CHECK (currency = 'USD'),
    total_cents     INTEGER NOT NULL,
    state           TEXT NOT NULL CHECK (state IN ('issued', 'paid', 'void')),
    run_id          TEXT,
    issued_at       TEXT NOT NULL,
    paid_at         TEXT,
    UNIQUE (subscription_id, period)
) STRICT;
CREATE INDEX invoices_period ON invoices (period, state);

CREATE TABLE invoice_lines (
    invoice_id   TEXT NOT NULL,
    line         INTEGER NOT NULL CHECK (line >= 1),
    kind         TEXT NOT NULL CHECK (kind IN ('plan', 'ai-overage', 'one-off', 'credit')),
    description  TEXT NOT NULL,
    amount_cents INTEGER NOT NULL,
    PRIMARY KEY (invoice_id, line)
) STRICT;

-- Money received, from verified provider events only.
CREATE TABLE payments (
    payment_id          TEXT PRIMARY KEY,
    subscription_id     TEXT NOT NULL,
    invoice_id          TEXT,
    rail                TEXT NOT NULL DEFAULT 'paypal',
    provider_payment_id TEXT NOT NULL UNIQUE,
    amount_cents        INTEGER NOT NULL,
    attempts            INTEGER NOT NULL DEFAULT 1 CHECK (attempts >= 1),
    receipt             TEXT,
    at                  TEXT NOT NULL
) STRICT;
CREATE INDEX payments_at ON payments (at);

-- Invoice runs: drafted from active subscriptions, issued only after op.finance approves the
-- exact run (its digest) with a passkey or an authenticator code.
CREATE TABLE invoice_runs (
    run_id       TEXT PRIMARY KEY,
    period       TEXT NOT NULL UNIQUE,
    digest       TEXT NOT NULL,
    invoices     INTEGER NOT NULL,
    total_cents  INTEGER NOT NULL,
    approved_by  TEXT NOT NULL,
    method       TEXT NOT NULL CHECK (method IN ('passkey', 'totp')),
    approved_at  TEXT NOT NULL
) STRICT;
