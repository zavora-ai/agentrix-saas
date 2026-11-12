-- Questions visitors passed to the owner from Ask (R8.3): when Ask could not answer, a visitor
-- may pass the question on, with an optional way to reply. Stored as data only; never sent to a
-- model. Writer: site/ (POST /ask/pass) through db::questions. Reader: the console's Today.
CREATE TABLE passed_questions (
    question_id TEXT PRIMARY KEY,
    site_id     TEXT NOT NULL REFERENCES sites (site_id),
    release_seq INTEGER,
    question    TEXT NOT NULL CHECK (length(question) BETWEEN 1 AND 500),
    contact     TEXT CHECK (contact IS NULL OR length(contact) BETWEEN 1 AND 120),
    passed_at   TEXT NOT NULL
) STRICT;
CREATE INDEX passed_questions_site ON passed_questions (site_id, passed_at);
