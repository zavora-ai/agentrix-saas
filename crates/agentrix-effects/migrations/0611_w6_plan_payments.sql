-- What PayPal collected for plan subscriptions (W6, asked by W3 for Money): one row per
-- verified PAYMENT.SALE.COMPLETED. provider_ref is the subscription's I-… id. W3's billing
-- reads it and marks invoices paid. Unix seconds.
CREATE TABLE IF NOT EXISTS plan_payments (
  provider_ref         TEXT NOT NULL,
  provider_payment_id  TEXT PRIMARY KEY,
  amount_minor         INTEGER NOT NULL,
  paid_at              INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS plan_payments_subscription ON plan_payments(provider_ref, paid_at);
