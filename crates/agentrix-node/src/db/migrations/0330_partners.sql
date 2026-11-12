-- Partners and their clients (W3, agentrix-operator; docs/08-control-plane.md §2.2, §2.5, §4.4).
-- Range 0300–0399. Additive only; no foreign keys to other workstreams' tables. One writer:
-- agentrix-operator. In Horizon 2, W6's agency grants (03 §4.3) take over partner_clients' grant
-- columns and this crate reads them.

-- An agency, reseller or referral partner.
CREATE TABLE partners (
    partner_id    TEXT PRIMARY KEY,
    kind          TEXT NOT NULL CHECK (kind IN ('agency', 'reseller', 'referral')),
    name          TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    console_host  TEXT,
    email_sender  TEXT,
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL
) STRICT;

-- The partner's people and their platform roles (08 §2.3).
CREATE TABLE partner_members (
    partner_id  TEXT NOT NULL,
    account_id  TEXT NOT NULL,
    role        TEXT NOT NULL CHECK (role IN ('partner.admin', 'partner.manager', 'partner.operator',
                    'partner.billing', 'partner.viewer')),
    added_by    TEXT NOT NULL,
    added_at    TEXT NOT NULL,
    removed_at  TEXT,
    PRIMARY KEY (partner_id, account_id, role)
) STRICT;

-- A client site and the charter role its owner granted the partner (08 §2.5: the owner grants,
-- revocable at any time). invited: asked, not yet granted. The owner grants with a passkey or an
-- authenticator code bound to the fingerprint of exactly this partner, site and role.
CREATE TABLE partner_clients (
    partner_id   TEXT NOT NULL,
    site_id      TEXT NOT NULL,
    charter_role TEXT NOT NULL CHECK (charter_role IN ('editor', 'approver', 'publisher', 'viewer')),
    ring         TEXT NOT NULL DEFAULT 'stable' CHECK (ring IN ('stable', 'canary')),
    state        TEXT NOT NULL CHECK (state IN ('invited', 'granted', 'declined', 'revoked')),
    fingerprint  TEXT NOT NULL,
    invited_by   TEXT NOT NULL,
    invited_at   TEXT NOT NULL,
    decided_by   TEXT,
    decided_method TEXT CHECK (decided_method IN ('passkey', 'totp')),
    decided_at   TEXT,
    revoked_at   TEXT,
    receipt      TEXT,
    PRIMARY KEY (partner_id, site_id)
) STRICT;
CREATE INDEX partner_clients_site ON partner_clients (site_id, state);
