-- Partner payouts through PayPal Payouts (Agentrix SaaS). Money leaving the company: each one
-- bound to the approval of its exact fingerprint, paid once. One writer: agentrix-effects.
-- Money in minor units with its ISO 4217 code. Times are Unix seconds.
CREATE TABLE IF NOT EXISTS payouts (
  id               TEXT PRIMARY KEY,        -- po_… (also PayPal's sender_item_id)
  site_id          TEXT NOT NULL,           -- the company paying
  fingerprint      TEXT NOT NULL UNIQUE,    -- the approved EffectRequest's; its hex is the sender_batch_id
  receiver         TEXT NOT NULL,           -- the payee's PayPal email
  currency         TEXT NOT NULL,
  amount_minor     INTEGER NOT NULL,
  note             TEXT NOT NULL,
  state            TEXT NOT NULL,           -- started | executed | not_executed | unknown
  batch_id         TEXT UNIQUE,             -- PayPal's payout_batch_id
  batch_status     TEXT,                    -- PENDING | PROCESSING | SUCCESS | DENIED | CANCELED …
  item_id          TEXT,                    -- PayPal's payout_item_id
  item_status      TEXT,                    -- SUCCESS | UNCLAIMED | PENDING | FAILED | RETURNED …
  approved_by      TEXT,
  receipt          TEXT,
  sandbox          INTEGER NOT NULL,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS payouts_site ON payouts(site_id, created_at);
