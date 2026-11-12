-- agentrix-records (W2): the Friday digest's own stop link and last send (additive).
-- stop_token: a stable per-reader token for the stop link every weekly email carries
-- (List-Unsubscribe). It can only stop that reader's digest, so it is kept as is.
-- last_sent_at: when this reader's last weekly digest was recorded, so a week sends once.
ALTER TABLE rec_digest_subs ADD COLUMN stop_token TEXT;
ALTER TABLE rec_digest_subs ADD COLUMN last_sent_at INTEGER;
