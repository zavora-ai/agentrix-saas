-- Customer erasures (W6; A-13, ADR-028): the report of one customer's erasure across every
-- registered store, kept after the account is gone so the receipt page still opens. Holds no
-- personal data: a keyed hash of the subject, store names, counts and times.
CREATE TABLE IF NOT EXISTS visitor_erasures (
  id            TEXT PRIMARY KEY,           -- er_…
  site_id       TEXT NOT NULL,
  subject_hash  TEXT NOT NULL,
  report        TEXT NOT NULL,              -- JSON: requested_at, stores[{store, state, ok, at}]
  digest        TEXT NOT NULL,              -- sha256:… of the report
  completed_at  INTEGER NOT NULL
);
