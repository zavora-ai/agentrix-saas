-- A site's own store (records.db in data/sites/<site>/), W8's tables. Every row belongs to the
-- one site whose directory holds the file; site_id is kept anyway, as defence in depth
-- (03-data.md §4.2 check 3).

-- The blob index: which blobs this site holds, their class, and whether they are sealed.
-- The bytes live in data/sites/<site>/cas/sha256/<aa>/<rest>.
CREATE TABLE blobs (
    site_id      TEXT NOT NULL,
    digest       TEXT NOT NULL CHECK (length(digest) = 71),
    kind         TEXT NOT NULL,
    content_type TEXT NOT NULL DEFAULT 'application/octet-stream',
    size         INTEGER NOT NULL CHECK (size >= 0),
    class        TEXT NOT NULL CHECK (class IN ('C0', 'C1', 'C2', 'C3')),
    sealed       INTEGER NOT NULL DEFAULT 0 CHECK (sealed IN (0, 1)),
    created_at   TEXT NOT NULL,
    PRIMARY KEY (site_id, digest)
) STRICT;

-- The site log: every receipt this site's issuers sign, in order, hash-chained. chain is
-- sha256("agentrix.site-log.v1\n" || prev_chain || "\n" || receipt_digest) as hex, so
-- dropping, reordering or changing a row breaks every chain value after it. Erasure redacts
-- body (redacted_at set) and keeps both digests, so the chain still verifies.
CREATE TABLE site_log (
    seq            INTEGER PRIMARY KEY CHECK (seq >= 1),
    site_id        TEXT NOT NULL,
    receipt_digest TEXT NOT NULL UNIQUE,
    prev_chain     TEXT NOT NULL,
    chain          TEXT NOT NULL UNIQUE,
    kind           TEXT NOT NULL,
    issuer         TEXT NOT NULL,
    issued_at      TEXT NOT NULL,
    body           TEXT,
    redacted_at    TEXT,
    CHECK (body IS NOT NULL OR redacted_at IS NOT NULL)
) STRICT;
CREATE INDEX site_log_kind ON site_log (kind, issued_at);

-- The outbox (09 §4.4): content-free events written in the same transaction as the change they
-- describe, relayed later to the lake. Only C0 and C1 events cross the site's boundary.
CREATE TABLE outbox (
    seq        INTEGER PRIMARY KEY,
    site_id    TEXT NOT NULL,
    topic      TEXT NOT NULL CHECK (length(topic) BETWEEN 3 AND 80),
    key        TEXT NOT NULL DEFAULT '',
    class      TEXT NOT NULL CHECK (class IN ('C0', 'C1')),
    payload    TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

-- How far each relay has read the outbox.
CREATE TABLE relay_cursors (
    relay      TEXT PRIMARY KEY,
    seq        INTEGER NOT NULL CHECK (seq >= 0),
    updated_at TEXT NOT NULL
) STRICT;
