-- control.db, schema 1 (specs/brick-1/design.md §5; docs/09-data-architecture.md §3.3).
--
-- Conventions:
-- - Times are RFC 3339 UTC text with microseconds, so text order is time order (the auth
--   tables use Unix seconds). Digests are 'sha256:<64 hex>'. Ids are prefixed UUIDv7 text
--   from agentrix-types. JSON columns hold agentrix-types values.
-- - Every table that holds site data carries site_id (design §7, the site wall).
-- - One writer per table; the owning lane is named on each table (design §3.0).
-- - STRICT tables, so a wrong type is an error rather than a silent conversion.

-- Control-plane's tables are defined once, in its own files: accounts, credentials, totp_seeds
-- and sessions in src/auth/schema.sql, and claims in src/claim/schema.sql. db::open applies
-- them before these migrations, so the foreign keys below can name accounts (id).

-- Sites. Assess creates the row with no owner; a verified claim sets it. Writer: data
-- (db::sites), called by assess/ to create a site and by claim/ to set its owner.
CREATE TABLE sites (
    site_id            TEXT PRIMARY KEY,
    domain             TEXT NOT NULL UNIQUE COLLATE NOCASE,
    name               TEXT NOT NULL DEFAULT '',
    old_origin         TEXT NOT NULL,
    utc_offset_minutes INTEGER NOT NULL DEFAULT 0,
    owner_account_id TEXT REFERENCES accounts (id),
    fixture          INTEGER NOT NULL DEFAULT 0 CHECK (fixture IN (0, 1)),
    created_at       TEXT NOT NULL
) STRICT;

-- Free assessments (R1). Writer: coordinator (assess/).
CREATE TABLE assessments (
    assessment_id TEXT PRIMARY KEY,
    site_id       TEXT NOT NULL REFERENCES sites (site_id),
    url           TEXT NOT NULL,
    state         TEXT NOT NULL CHECK (state IN ('running', 'report-ready', 'unreachable', 'failed')),
    report        TEXT,
    created_at    TEXT NOT NULL,
    finished_at   TEXT
) STRICT;
CREATE INDEX assessments_site ON assessments (site_id);

-- Moves (design §6). Writer: move lane.
CREATE TABLE moves (
    move_id    TEXT PRIMARY KEY,
    site_id    TEXT NOT NULL REFERENCES sites (site_id),
    state      TEXT NOT NULL CHECK (state IN ('assessed', 'claimed', 'replayed', 'cleaned',
                   'rebuilt', 'proved', 'awaiting-owner', 'live', 'rolled-back', 'failed',
                   'stopped')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX moves_site ON moves (site_id);

-- One row per step transition, so a crash resumes from the last completed step (N9).
-- resume holds the step's JSON resume data. Writer: move lane.
CREATE TABLE move_steps (
    move_id    TEXT NOT NULL REFERENCES moves (move_id),
    seq        INTEGER NOT NULL,
    site_id    TEXT NOT NULL REFERENCES sites (site_id),
    step       TEXT NOT NULL CHECK (step IN ('assess', 'claim', 'replay', 'clean', 'rebuild',
                   'prove', 'golive')),
    state      TEXT NOT NULL CHECK (state IN ('pending', 'running', 'waiting', 'done', 'failed')),
    summary    TEXT NOT NULL DEFAULT '',
    receipt    TEXT,
    resume     TEXT,
    started_at TEXT NOT NULL,
    ended_at   TEXT,
    PRIMARY KEY (move_id, seq)
) STRICT;

-- Sealed-room jobs (design §4.2). spec is the SignedJobSpec JSON; result the JobResult JSON.
-- Writer: coordinator (broker/).
CREATE TABLE jobs (
    job_id        TEXT PRIMARY KEY,
    site_id       TEXT NOT NULL REFERENCES sites (site_id),
    move_id       TEXT REFERENCES moves (move_id),
    assessment_id TEXT REFERENCES assessments (assessment_id),
    kind          TEXT NOT NULL CHECK (kind IN ('fetch', 'replay', 'render', 'build')),
    spec          TEXT NOT NULL,
    state         TEXT NOT NULL CHECK (state IN ('queued', 'running', 'done', 'failed', 'killed')),
    result        TEXT,
    receipt       TEXT,
    created_at    TEXT NOT NULL,
    started_at    TEXT,
    ended_at      TEXT
) STRICT;
CREATE INDEX jobs_site ON jobs (site_id);
CREATE INDEX jobs_move ON jobs (move_id);

-- The CAS index: which site holds which blob, and how to serve it. Writer: data (db::cas).
-- location: 'local', or 'local+object' once mirrored to Object Storage.
CREATE TABLE artifacts (
    site_id      TEXT NOT NULL REFERENCES sites (site_id),
    digest       TEXT NOT NULL,
    kind         TEXT NOT NULL,
    content_type TEXT NOT NULL DEFAULT 'application/octet-stream',
    size       INTEGER NOT NULL CHECK (size >= 0),
    location   TEXT NOT NULL CHECK (location IN ('local', 'local+object')),
    created_at TEXT NOT NULL,
    PRIMARY KEY (site_id, digest)
) STRICT;

-- Detector findings (R1, R5). evidence is a JSON array of digests. Writers: assess/ for rows with
-- assessment_id set, and the move lane for rows with move_id set.
CREATE TABLE findings (
    finding_id    TEXT PRIMARY KEY,
    site_id       TEXT NOT NULL REFERENCES sites (site_id),
    assessment_id TEXT REFERENCES assessments (assessment_id),
    move_id       TEXT REFERENCES moves (move_id),
    detector      TEXT NOT NULL,
    severity      TEXT NOT NULL CHECK (severity IN ('urgent', 'warn', 'info')),
    category      TEXT NOT NULL CHECK (category IN ('security', 'speed', 'links', 'ai-readiness')),
    title         TEXT NOT NULL,
    explanation   TEXT NOT NULL DEFAULT '',
    location      TEXT NOT NULL,
    evidence      TEXT NOT NULL DEFAULT '[]',
    at            TEXT NOT NULL
) STRICT;
CREATE INDEX findings_site ON findings (site_id);

-- Decision cards (R7). card is the DecisionCard JSON. A decided row records the method
-- ('passkey' or 'totp', design §4.1), the signer and the assertion or step-up evidence.
-- Writer: move lane (decisions/).
CREATE TABLE decisions (
    decision_id TEXT PRIMARY KEY,
    site_id     TEXT NOT NULL REFERENCES sites (site_id),
    kind        TEXT NOT NULL CHECK (kind IN ('go-live', 'quarantine-restore', 'order-held')),
    card        TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    nonce       TEXT,
    state       TEXT NOT NULL CHECK (state IN ('waiting', 'approved', 'declined', 'expired')),
    method      TEXT CHECK (method IN ('passkey', 'totp')),
    signer      TEXT REFERENCES accounts (id),
    assertion   TEXT,
    receipt     TEXT,
    created_at  TEXT NOT NULL,
    decided_at  TEXT,
    CHECK (state = 'waiting' OR state = 'expired' OR decided_at IS NOT NULL),
    CHECK (state <> 'approved' OR (method IS NOT NULL AND signer IS NOT NULL
        AND assertion IS NOT NULL))
) STRICT;
CREATE INDEX decisions_site ON decisions (site_id, state);

-- Quarantined items (R5). title and reason are plain words for the owner. The console shows
-- "restore asked" while decision_id names a waiting card. Writer: move lane.
CREATE TABLE quarantine (
    item_id     TEXT PRIMARY KEY,
    site_id     TEXT NOT NULL REFERENCES sites (site_id),
    move_id     TEXT REFERENCES moves (move_id),
    title       TEXT NOT NULL,
    reason      TEXT NOT NULL,
    location    TEXT NOT NULL,
    evidence    TEXT NOT NULL DEFAULT '[]',
    state       TEXT NOT NULL CHECK (state IN ('held', 'restored', 'deleted')),
    decision_id TEXT REFERENCES decisions (decision_id),
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
) STRICT;
CREATE INDEX quarantine_site ON quarantine (site_id);

-- Releases (design §4.5): the manifest digest and the signed checkpoint. Writer: move lane
-- (launch/).
CREATE TABLE releases (
    site_id         TEXT NOT NULL REFERENCES sites (site_id),
    seq             INTEGER NOT NULL CHECK (seq >= 1),
    manifest_digest TEXT NOT NULL,
    checkpoint      TEXT,
    state           TEXT NOT NULL CHECK (state IN ('candidate', 'live', 'superseded',
                        'rolled-back')),
    decision_id     TEXT REFERENCES decisions (decision_id),
    created_at      TEXT NOT NULL,
    signed_at       TEXT,
    PRIMARY KEY (site_id, seq),
    CHECK (state = 'candidate' OR checkpoint IS NOT NULL)
) STRICT;

-- The live pointer: which signed release the edge serves. Writer: move lane (launch/).
CREATE TABLE site_pointer (
    site_id         TEXT PRIMARY KEY REFERENCES sites (site_id),
    seq             INTEGER NOT NULL,
    manifest_digest TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    FOREIGN KEY (site_id, seq) REFERENCES releases (site_id, seq)
) STRICT;

-- Charters (v1, starting levels), signed by the owner. Writer: move lane.
CREATE TABLE charters (
    site_id    TEXT NOT NULL REFERENCES sites (site_id),
    version    INTEGER NOT NULL CHECK (version >= 1),
    body       TEXT NOT NULL,
    digest     TEXT NOT NULL,
    signed_by  TEXT REFERENCES accounts (id),
    signature  TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (site_id, version)
) STRICT;

-- Receipts (design §4.4): the signed receipt's canonical JSON, addressed by its digest.
-- Every lane writes through the receipts store (db::receipts), so this has one writer: data.
CREATE TABLE receipts (
    digest       TEXT PRIMARY KEY,
    receipt_id   TEXT NOT NULL UNIQUE,
    site_id      TEXT NOT NULL REFERENCES sites (site_id),
    kind         TEXT NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id   TEXT NOT NULL,
    issuer       TEXT NOT NULL,
    started_at   TEXT NOT NULL,
    ended_at     TEXT NOT NULL,
    issued_at    TEXT NOT NULL,
    body         TEXT NOT NULL
) STRICT;
CREATE INDEX receipts_site_kind ON receipts (site_id, kind, issued_at);
CREATE INDEX receipts_subject ON receipts (subject_type, subject_id);

-- Model usage: one row per gateway call, per site, purpose and provider (R11.3).
-- cost_micro_usd is an estimate from the catalogue prices (design §4.3); replayed calls (D10a)
-- cost 0. tier and tier_reason record how the model was chosen (design §4.3); specialists
-- have no tier. move_id is set when the caller names one. Writer: data (agentrix-models).
CREATE TABLE usage (
    usage_id         INTEGER PRIMARY KEY,
    site_id          TEXT NOT NULL REFERENCES sites (site_id),
    purpose          TEXT NOT NULL CHECK (purpose IN ('build', 'concierge', 'brief',
                         'vision-parity', 'screen', 'decide', 'embed', 'rerank', 'router')),
    provider         TEXT NOT NULL DEFAULT 'vultr' CHECK (provider IN ('vultr', 'anthropic')),
    model            TEXT NOT NULL,
    tier             TEXT CHECK (tier IN ('fast', 'standard', 'deep')),
    tier_reason      TEXT NOT NULL DEFAULT '',
    move_id          TEXT REFERENCES moves (move_id),
    tokens_in        INTEGER NOT NULL CHECK (tokens_in >= 0),
    tokens_out       INTEGER NOT NULL CHECK (tokens_out >= 0),
    reasoning_tokens INTEGER NOT NULL DEFAULT 0 CHECK (reasoning_tokens >= 0),
    cost_micro_usd   INTEGER NOT NULL CHECK (cost_micro_usd >= 0),
    outcome          TEXT NOT NULL CHECK (outcome IN ('ok', 'error', 'over-budget', 'replayed')),
    at               TEXT NOT NULL
) STRICT;
CREATE INDEX usage_site_at ON usage (site_id, at);
CREATE INDEX usage_move ON usage (move_id);

-- Budgets per site and purpose ('*' for every purpose) (R11.3). Writer: data.
CREATE TABLE budgets (
    site_id         TEXT NOT NULL REFERENCES sites (site_id),
    purpose         TEXT NOT NULL,
    period          TEXT NOT NULL CHECK (period IN ('day', 'month', 'total')),
    limit_micro_usd INTEGER NOT NULL CHECK (limit_micro_usd >= 0),
    PRIMARY KEY (site_id, purpose)
) STRICT;

-- The Chief WebAgent's briefs for Today (R10.1, S2-04). body is the brief as JSON: lines in
-- plain words, each with the receipt digest it rests on. Writer: data (brief/).
CREATE TABLE briefs (
    site_id    TEXT NOT NULL REFERENCES sites (site_id),
    written_at TEXT NOT NULL,
    body       TEXT NOT NULL,
    PRIMARY KEY (site_id, written_at)
) STRICT;

-- Gauntlet runs (R13). Writer: gauntlet lane.
CREATE TABLE gauntlet_runs (
    run_id     TEXT PRIMARY KEY,
    site_id    TEXT NOT NULL REFERENCES sites (site_id),
    score      INTEGER NOT NULL CHECK (score >= 0),
    out_of     INTEGER NOT NULL CHECK (out_of >= score),
    receipt    TEXT,
    detail     TEXT,
    started_at TEXT NOT NULL,
    ended_at   TEXT
) STRICT;

-- UI events for SSE replay from Last-Event-ID (design §4.3a). stream is 'assess:<id>' or
-- 'move:<id>'. Writer: coordinator (emitted by agentrix-node).
CREATE TABLE ui_events (
    stream  TEXT NOT NULL,
    seq     INTEGER NOT NULL CHECK (seq >= 1),
    site_id TEXT NOT NULL REFERENCES sites (site_id),
    name    TEXT NOT NULL,
    data    TEXT NOT NULL,
    at      TEXT NOT NULL,
    PRIMARY KEY (stream, seq)
) STRICT;
