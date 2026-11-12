-- The sealed cart mandate as sent, so a retried call resends it byte for byte: the same
-- digest, the same PayPal request id, never a second payment.
ALTER TABLE settle_carts ADD COLUMN cart_json TEXT;
