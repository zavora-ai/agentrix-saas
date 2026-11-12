-- PayPal's own word on a site's sale: each verified webhook event about a capture or a refund
-- the site took through Agentrix's checkout, kept once by PayPal's event id on the order it
-- confirms. A confirmation posts nothing; the sale is booked when it is captured.
CREATE TABLE IF NOT EXISTS rec_paypal_confirmations (
    site TEXT NOT NULL,
    event_id TEXT NOT NULL,           -- PayPal's WH-…
    order_id TEXT NOT NULL,
    event_type TEXT NOT NULL,         -- PAYMENT.CAPTURE.COMPLETED, PAYMENT.CAPTURE.REFUNDED
    kind TEXT NOT NULL,               -- capture | refund
    object_id TEXT NOT NULL,          -- PayPal's capture or refund id
    status TEXT NOT NULL,             -- PayPal's status on the resource
    confirmed_at INTEGER NOT NULL,
    PRIMARY KEY (site, event_id)
);
CREATE INDEX IF NOT EXISTS rec_paypal_confirmations_order ON rec_paypal_confirmations (site, order_id);
