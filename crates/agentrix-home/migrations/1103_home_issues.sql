-- W10: the weekly fix's issues. The Editor drafts one a week; it waits for James's OK as a
-- decision card, and goes out only if he approves.
CREATE TABLE IF NOT EXISTS home_issues (
  id TEXT PRIMARY KEY,
  week TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL,
  lines TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('waiting', 'sent', 'declined')),
  card_id TEXT,
  readers INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  sent_at TEXT
);
