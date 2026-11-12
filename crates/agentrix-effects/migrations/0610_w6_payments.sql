-- The Effects Gateway's first slice (W6; 02 §4.3, ADR-004, ADR-006): customer payments through
-- PayPal, refunds as owner decisions, and plan subscriptions. One writer: agentrix-effects.
-- Money in minor units with its ISO 4217 code. Times are Unix seconds. No card data, ever:
-- payers pay on PayPal's own page; we keep PayPal's ids and the amounts.

-- One row per effect, keyed by its fingerprint: the same request never runs twice.
CREATE TABLE IF NOT EXISTS effects (
  fingerprint   TEXT PRIMARY KEY,           -- sha256:… of the request's canonical JSON
  site_id       TEXT NOT NULL,
  tool          TEXT NOT NULL,              -- payment.create | payment.capture | payment.refund | subscription.create | subscription.cancel
  subject       TEXT NOT NULL,              -- pay_…, sub_…
  state         TEXT NOT NULL,              -- started | executed | not_executed | unknown
  provider_ref  TEXT,
  detail        TEXT,                       -- JSON, never secrets
  receipt       TEXT,                       -- digest of the platform receipt
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS effects_site ON effects(site_id, created_at);

-- A customer's payment for an order on a site.
CREATE TABLE IF NOT EXISTS customer_payments (
  id               TEXT PRIMARY KEY,        -- pay_…
  site_id          TEXT NOT NULL,
  order_id         TEXT NOT NULL,
  customer         TEXT,                    -- vac_… when signed in
  fingerprint      TEXT NOT NULL UNIQUE,
  provider         TEXT NOT NULL,           -- paypal
  provider_ref     TEXT UNIQUE,             -- the PayPal order id
  approve_url      TEXT,
  currency         TEXT NOT NULL,
  amount_minor     INTEGER NOT NULL,
  state            TEXT NOT NULL,           -- created | captured | pending | declined | cancelled | refunded | partly_refunded
  capture_id       TEXT,
  captured_minor   INTEGER,
  refunded_minor   INTEGER NOT NULL DEFAULT 0,
  sandbox          INTEGER NOT NULL,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS customer_payments_order ON customer_payments(site_id, order_id);

-- Refunds: each one an owner decision, bound to the approval's fingerprint.
CREATE TABLE IF NOT EXISTS refunds (
  id             TEXT PRIMARY KEY,          -- rf_…
  payment_id     TEXT NOT NULL REFERENCES customer_payments(id),
  site_id        TEXT NOT NULL,
  fingerprint    TEXT NOT NULL UNIQUE,      -- the approved EffectRequest's fingerprint
  amount_minor   INTEGER NOT NULL,
  currency       TEXT NOT NULL,
  state          TEXT NOT NULL,             -- executed | not_executed | unknown
  provider_ref   TEXT,
  approved_by    TEXT,                      -- the approver's account id
  receipt        TEXT,
  created_at     INTEGER NOT NULL
);

-- PayPal catalogue: one product and plan per platform plan and version.
CREATE TABLE IF NOT EXISTS paypal_plans (
  plan_key         TEXT PRIMARY KEY,        -- "<plan_id>@<version>"
  product_id       TEXT NOT NULL,
  paypal_plan_id   TEXT NOT NULL,
  currency         TEXT NOT NULL,
  amount_minor     INTEGER NOT NULL,
  created_at       INTEGER NOT NULL
);

-- Plan subscriptions as PayPal reports them. W3's billing reads these (03 §4.3: the billing
-- row is W3's; this is the provider's side).
CREATE TABLE IF NOT EXISTS plan_subscriptions (
  id               TEXT PRIMARY KEY,        -- sub_…
  site_id          TEXT NOT NULL,
  plan_key         TEXT NOT NULL,
  provider_ref     TEXT UNIQUE,             -- PayPal I-…
  approve_url      TEXT,
  state            TEXT NOT NULL,           -- pending_approval | active | suspended | cancelled | expired | past_due
  currency         TEXT NOT NULL,
  amount_minor     INTEGER NOT NULL,
  period_end       INTEGER,
  last_event       TEXT,
  sandbox          INTEGER NOT NULL,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS plan_subscriptions_site ON plan_subscriptions(site_id);

-- Every verified PayPal webhook, once (PayPal retries; the event id dedupes).
CREATE TABLE IF NOT EXISTS paypal_events (
  id           TEXT PRIMARY KEY,            -- PayPal's event id
  kind         TEXT NOT NULL,
  resource     TEXT NOT NULL,               -- the resource id it is about
  received_at  INTEGER NOT NULL
);
