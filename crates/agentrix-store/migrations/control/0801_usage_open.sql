-- Model usage, opened for the product's own purposes (specs/platform/03-data.md §2.4). Owner: W8.
--
-- 0001 limited `purpose` to Brick 1's nine and `provider` to two, so image and voice calls
-- (W11) and WebAgent purposes (docs/05 §3.7) could not be metered. SQLite cannot drop a CHECK
-- in place, so the table is rebuilt with the same columns, in the same order, minus those two
-- lists; the gateway still checks purposes in code. Binaries built before this keep working:
-- every column they write is still here.
--
-- New nullable columns:
-- - units, unit: the quantity for calls not counted in tokens: 'image' (images made),
--   'audio_second' (speech in or out), 'character' (text spoken).
-- - webagent, job, process_id: which WebAgent, job and process made the call (W1), so the
--   console's "today per process" has a source.
-- - reserved_micro_usd: the amount reserved before the call, for reserve-then-settle.
CREATE TABLE usage_purposes (
    purpose TEXT PRIMARY KEY CHECK (length(purpose) BETWEEN 1 AND 40),
    family  TEXT NOT NULL CHECK (length(family) BETWEEN 1 AND 20),
    words   TEXT NOT NULL
) STRICT;

INSERT OR IGNORE INTO usage_purposes (purpose, family, words) VALUES
    ('build', 'chat', 'Rebuilding pages'),
    ('concierge', 'chat', 'Answering visitors'),
    ('brief', 'chat', 'Writing the brief'),
    ('vision-parity', 'chat', 'Comparing pages by eye'),
    ('screen', 'chat', 'Screening for hidden instructions'),
    ('decide', 'chat', 'Weighing decisions'),
    ('embed', 'search', 'Indexing pages for search'),
    ('rerank', 'search', 'Ranking search results'),
    ('router', 'chat', 'Choosing a model'),
    ('webagent.act', 'chat', 'WebAgents at work'),
    ('webagent.write', 'chat', 'WebAgents writing'),
    ('webagent.summarise', 'chat', 'WebAgents summarising'),
    ('plan', 'chat', 'Planning'),
    ('image', 'image', 'Making images'),
    ('voice.speak', 'voice', 'Speaking'),
    ('voice.listen', 'voice', 'Listening'),
    ('voice.live', 'voice', 'Live voice');

CREATE TABLE usage_open (
    usage_id           INTEGER PRIMARY KEY,
    site_id            TEXT NOT NULL REFERENCES sites (site_id),
    purpose            TEXT NOT NULL CHECK (length(purpose) BETWEEN 1 AND 40),
    provider           TEXT NOT NULL DEFAULT 'vultr' CHECK (length(provider) BETWEEN 1 AND 40),
    model              TEXT NOT NULL,
    tier               TEXT CHECK (tier IN ('fast', 'standard', 'deep')),
    tier_reason        TEXT NOT NULL DEFAULT '',
    move_id            TEXT REFERENCES moves (move_id),
    tokens_in          INTEGER NOT NULL CHECK (tokens_in >= 0),
    tokens_out         INTEGER NOT NULL CHECK (tokens_out >= 0),
    reasoning_tokens   INTEGER NOT NULL DEFAULT 0 CHECK (reasoning_tokens >= 0),
    cost_micro_usd     INTEGER NOT NULL CHECK (cost_micro_usd >= 0),
    outcome            TEXT NOT NULL CHECK (outcome IN ('ok', 'error', 'over-budget', 'replayed')),
    at                 TEXT NOT NULL,
    units              INTEGER CHECK (units IS NULL OR units >= 0),
    unit               TEXT CHECK (unit IS NULL OR unit IN ('image', 'audio_second', 'character')),
    webagent           TEXT,
    job                TEXT,
    process_id         TEXT,
    reserved_micro_usd INTEGER CHECK (reserved_micro_usd IS NULL OR reserved_micro_usd >= 0)
) STRICT;

INSERT INTO usage_open (usage_id, site_id, purpose, provider, model, tier, tier_reason, move_id,
    tokens_in, tokens_out, reasoning_tokens, cost_micro_usd, outcome, at)
SELECT usage_id, site_id, purpose, provider, model, tier, tier_reason, move_id,
    tokens_in, tokens_out, reasoning_tokens, cost_micro_usd, outcome, at
FROM usage;

-- Every row copied, or the migration fails and nothing changes.
CREATE TEMP TABLE w8_usage_copied (ok INTEGER NOT NULL CHECK (ok = 1));
INSERT INTO w8_usage_copied
SELECT (SELECT COUNT(*) FROM usage_open) = (SELECT COUNT(*) FROM usage)
   AND (SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage_open)
     = (SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage);
DROP TABLE w8_usage_copied;

DROP TABLE usage;
ALTER TABLE usage_open RENAME TO usage;
CREATE INDEX usage_site_at ON usage (site_id, at);
CREATE INDEX usage_move ON usage (move_id);
CREATE INDEX usage_webagent ON usage (site_id, webagent, at) WHERE webagent IS NOT NULL;
