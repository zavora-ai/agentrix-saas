-- W10 (agentrix-home, range 1100–1199): people who asked agentrix.website to keep their check
-- report, with the exact words they agreed to and when. A row is deleted 30 days after it was
-- made unless its owner later confirms the weekly fix (W10 slice d sets delete_after to NULL).
CREATE TABLE IF NOT EXISTS home_contacts (
  id TEXT PRIMARY KEY,
  channel TEXT NOT NULL CHECK (channel IN ('email', 'whatsapp')),
  contact TEXT NOT NULL,
  assessment_id TEXT NOT NULL,
  report_url TEXT NOT NULL,
  report_words TEXT NOT NULL,
  weekly INTEGER NOT NULL DEFAULT 0,
  weekly_words TEXT,
  weekly_confirmed_at TEXT,
  source_page TEXT NOT NULL,
  report_status TEXT NOT NULL DEFAULT 'saved' CHECK (report_status IN ('saved', 'sent')),
  created_at TEXT NOT NULL,
  delete_after TEXT
);
CREATE INDEX IF NOT EXISTS home_contacts_delete_after ON home_contacts (delete_after);
CREATE UNIQUE INDEX IF NOT EXISTS home_contacts_once ON home_contacts (assessment_id, contact);
