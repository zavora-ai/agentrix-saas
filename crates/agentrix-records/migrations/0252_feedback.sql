-- agentrix-records (W2): desk feedback and taught answers (additive).
-- rec_questions.answered_at: when the owner taught the answer, so a desk can say
-- "from the owner, Sun 8 Nov". Older answered rows keep NULL and fall back to created_at.
ALTER TABLE rec_questions ADD COLUMN answered_at INTEGER;

-- One row per answer a visitor marked with "Was this right?". Pressing the other button, or
-- adding a note, updates the same row. answer is what the desk said, when the site still had
-- it in memory; basis is ai, instant, rules or owner.
CREATE TABLE rec_feedback (
    site TEXT NOT NULL,
    answer_id TEXT NOT NULL,
    desk TEXT NOT NULL,
    question TEXT NOT NULL,
    answer TEXT,
    basis TEXT,
    verdict TEXT NOT NULL,
    note TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (site, answer_id)
);
CREATE INDEX rec_feedback_by_time ON rec_feedback (site, updated_at);
