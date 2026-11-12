-- Every refusal a door or desk made, as the sealed refusal receipt it answered with.
CREATE TABLE IF NOT EXISTS settle_refusals (
    refusal_id   TEXT PRIMARY KEY,
    at           INTEGER NOT NULL,
    code         TEXT NOT NULL,
    receipt_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS settle_refusals_at ON settle_refusals (at);
