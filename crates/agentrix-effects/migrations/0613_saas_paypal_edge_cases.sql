-- The PayPal edge cases the ecosystem run makes happen (specs/saas/paypal-edge-cases.md): a
-- webhook event resent to our own webhook, an unclaimed payout item cancelled, and every call
-- that carried PayPal's sandbox error simulation, labelled so. One writer: agentrix-effects.
-- Times are Unix seconds.
CREATE TABLE IF NOT EXISTS paypal_edge_cases (
  id          TEXT PRIMARY KEY,        -- edge_…
  kind        TEXT NOT NULL,           -- webhook.resend | payout_item.cancel | simulation
  site_id     TEXT,                    -- the company concerned, when there is one
  subject     TEXT NOT NULL,           -- the event id, the payout item id, or the call's path
  label       TEXT,                    -- 'sandbox error simulation' for simulated calls
  outcome     TEXT NOT NULL,           -- executed | not_executed | unknown
  detail      TEXT NOT NULL,           -- JSON: what was asked and what PayPal answered
  sandbox     INTEGER NOT NULL,
  created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS paypal_edge_cases_at ON paypal_edge_cases(created_at);

-- An event delivered again (PayPal's retry, or a resend) is counted here and changes nothing.
ALTER TABLE paypal_events ADD COLUMN duplicates INTEGER NOT NULL DEFAULT 0;
ALTER TABLE paypal_events ADD COLUMN last_duplicate_at INTEGER;
