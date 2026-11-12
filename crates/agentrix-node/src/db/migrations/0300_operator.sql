-- The control plane's tables (W3, agentrix-operator; docs/08-control-plane.md). Range 0300–0399.
--
-- Additive only: new tables and indexes. No foreign keys to other workstreams' tables; ids are
-- referred to by value (specs/platform/how-we-work.md, "Migrations"). Times are RFC 3339 UTC text
-- with microseconds, like 0001. One writer per table: agentrix-operator.
--
-- In Horizon 2, W6 takes operator_roles, support_grants and break_glass into agentrix-identity's
-- memberships and grants (agreed with W6, Sun 8 Nov 2026), and this crate reads them through
-- W6's API.

-- Platform roles for Zavora's people (08 §2.3). 'director' is a charter Owner on the operator
-- charter (08 §5.3). Granted from the roles file at startup or by an op.admin; every grant and
-- revocation is also an operator_ledger entry.
CREATE TABLE operator_roles (
    account_id TEXT NOT NULL,
    role       TEXT NOT NULL CHECK (role IN ('op.admin', 'op.sre', 'op.release', 'op.finance',
                   'op.support', 'op.partners', 'op.security', 'op.auditor', 'director')),
    granted_by TEXT NOT NULL,
    granted_at TEXT NOT NULL,
    revoked_at TEXT,
    PRIMARY KEY (account_id, role)
) STRICT;

-- Operator sessions on the ops host, separate from owners' sessions: only the SHA-256 of the
-- cookie's token is stored. method names how the person proved presence at sign-in.
CREATE TABLE operator_sessions (
    token_hash TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    method     TEXT NOT NULL CHECK (method IN ('passkey', 'totp')),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    ended_at   TEXT
) STRICT;
CREATE INDEX operator_sessions_account ON operator_sessions (account_id);

-- The operator ledger (08 §4.6): append-only and hash-chained. hash = SHA-256 over the entry's
-- canonical JSON, which includes prev_hash, so editing or dropping a row breaks every later hash.
-- actor is 'account:<id>', 'agent:op:zavora/<role>' or 'platform'.
CREATE TABLE operator_ledger (
    seq        INTEGER PRIMARY KEY CHECK (seq >= 1),
    entry_id   TEXT NOT NULL UNIQUE,
    at         TEXT NOT NULL,
    actor      TEXT NOT NULL,
    actor_name TEXT NOT NULL,
    verb       TEXT NOT NULL,
    target     TEXT NOT NULL,
    summary    TEXT NOT NULL,
    approval   TEXT,
    detail     TEXT,
    prev_hash  TEXT NOT NULL,
    hash       TEXT NOT NULL UNIQUE
) STRICT;
CREATE INDEX operator_ledger_target ON operator_ledger (target, seq);

-- Support grants (08 §2.5): the owner grants time-boxed, scoped access to named Zavora people.
-- Requested by an operator; granted by the owner with a passkey or an authenticator code bound to
-- the fingerprint. Every access under a grant is a row in support_access and a receipt.
CREATE TABLE support_grants (
    grant_id        TEXT PRIMARY KEY,
    site_id         TEXT NOT NULL,
    requested_by    TEXT NOT NULL,
    grantees        TEXT NOT NULL,
    areas           TEXT NOT NULL,
    access          TEXT NOT NULL CHECK (access IN ('read', 'read-write')),
    hours           INTEGER NOT NULL CHECK (hours BETWEEN 1 AND 72),
    reason          TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
    ticket          TEXT,
    fingerprint     TEXT NOT NULL,
    state           TEXT NOT NULL CHECK (state IN ('requested', 'active', 'declined',
                        'expired', 'revoked', 'withdrawn')),
    decided_by      TEXT,
    decided_method  TEXT CHECK (decided_method IN ('passkey', 'totp')),
    decided_at      TEXT,
    starts_at       TEXT,
    ends_at         TEXT,
    revoked_by      TEXT,
    revoked_at      TEXT,
    receipt         TEXT,
    created_at      TEXT NOT NULL,
    CHECK (state <> 'active' OR (decided_by IS NOT NULL AND starts_at IS NOT NULL
        AND ends_at IS NOT NULL))
) STRICT;
CREATE INDEX support_grants_site ON support_grants (site_id, state);

CREATE TABLE support_access (
    access_id  INTEGER PRIMARY KEY,
    grant_id   TEXT NOT NULL,
    site_id    TEXT NOT NULL,
    account_id TEXT NOT NULL,
    area       TEXT NOT NULL,
    at         TEXT NOT NULL,
    receipt    TEXT
) STRICT;
CREATE INDEX support_access_grant ON support_access (grant_id);

-- Break-glass (08 §2.5): emergency read access to one site without the owner's grant. Two
-- operator people (op.security and a director), a recorded reason, one hour, renewable once;
-- the owner is told at the time.
CREATE TABLE break_glass (
    bg_id          TEXT PRIMARY KEY,
    site_id        TEXT NOT NULL,
    reason         TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
    incident       TEXT,
    fingerprint    TEXT NOT NULL,
    state          TEXT NOT NULL CHECK (state IN ('waiting', 'open', 'closed', 'declined',
                       'expired')),
    first_by       TEXT NOT NULL,
    first_role     TEXT NOT NULL,
    first_method   TEXT NOT NULL CHECK (first_method IN ('passkey', 'totp')),
    first_at       TEXT NOT NULL,
    second_by      TEXT,
    second_role    TEXT,
    second_method  TEXT CHECK (second_method IN ('passkey', 'totp')),
    second_at      TEXT,
    starts_at      TEXT,
    ends_at        TEXT,
    renewed        INTEGER NOT NULL DEFAULT 0 CHECK (renewed IN (0, 1)),
    owner_told_at  TEXT,
    closed_by      TEXT,
    closed_at      TEXT,
    receipt        TEXT,
    created_at     TEXT NOT NULL,
    CHECK (second_by IS NULL OR second_by <> first_by)
) STRICT;
CREATE INDEX break_glass_site ON break_glass (site_id, state);

-- Platform holds (08 §2.4, §3.8): the operator's switch, separate from the owner's freeze.
-- mode 'read-only' serves the last signed release with answers and forms paused; 'notice'
-- serves the neutral notice page. reason is shown to the owner, never to visitors.
CREATE TABLE platform_holds (
    hold_id        TEXT PRIMARY KEY,
    scope          TEXT NOT NULL CHECK (scope IN ('site', 'partner', 'cell', 'fleet')),
    target         TEXT NOT NULL,
    mode           TEXT NOT NULL CHECK (mode IN ('read-only', 'notice')),
    reason         TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
    fingerprint    TEXT NOT NULL,
    placed_by      TEXT NOT NULL,
    placed_method  TEXT NOT NULL CHECK (placed_method IN ('passkey', 'totp')),
    placed_at      TEXT NOT NULL,
    second_by      TEXT,
    second_method  TEXT CHECK (second_method IN ('passkey', 'totp')),
    second_at      TEXT,
    lifted_by      TEXT,
    lifted_at      TEXT,
    lift_reason    TEXT,
    receipt        TEXT
) STRICT;
CREATE INDEX platform_holds_target ON platform_holds (scope, target, lifted_at);
