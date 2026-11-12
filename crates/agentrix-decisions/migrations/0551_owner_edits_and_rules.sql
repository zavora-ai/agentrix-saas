-- Edit before approving, and decline and teach (04-ui-parity §3 item 7). Writer: agentrix-decisions.
-- Additive only: three nullable columns on the v2 cards and one new table, so a binary from
-- before this migration still runs on the result.
--
-- `editable` is the JSON list of fields the raiser lets the owner change (edit::EditableField).
-- `owner_edit` is the owner's edit an approval signed (edit::OwnerEdit: before, after, changes).
-- `decline_reason` is the owner's reason for a decline, in their words.
ALTER TABLE decision_cards_v2 ADD COLUMN editable TEXT;
ALTER TABLE decision_cards_v2 ADD COLUMN owner_edit TEXT;
ALTER TABLE decision_cards_v2 ADD COLUMN decline_reason TEXT;

-- Standing rules the owner made while declining a card: an instruction for the WebAgent and job
-- that raised it. Removing one keeps the row (removed_at, removed_receipt) for the record.
CREATE TABLE IF NOT EXISTS decision_rules (
    id              TEXT PRIMARY KEY,
    site_id         TEXT NOT NULL,
    webagent        TEXT NOT NULL,
    job             TEXT,
    kind            TEXT NOT NULL,
    words           TEXT NOT NULL,
    from_card       TEXT NOT NULL,
    created_by      TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    receipt         TEXT NOT NULL,
    removed_at      TEXT,
    removed_receipt TEXT
);
CREATE INDEX IF NOT EXISTS decision_rules_site ON decision_rules (site_id, webagent, removed_at);
