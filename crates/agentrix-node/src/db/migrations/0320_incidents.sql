-- Incidents (W3, agentrix-operator; docs/08-control-plane.md §3.7). Range 0300–0399.
-- Additive only; no foreign keys to other workstreams' tables. One writer: agentrix-operator.
-- Declared by an on-call person today; the SRE WebAgent opens them from burn-rate alerts once
-- edge probes run. Every change is also an operator_ledger entry.

CREATE TABLE incidents (
    incident_id   TEXT PRIMARY KEY,
    severity      TEXT NOT NULL CHECK (severity IN ('sev1', 'sev2', 'sev3', 'sev4')),
    title         TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 140),
    summary       TEXT NOT NULL DEFAULT '',
    cell          TEXT NOT NULL,
    site_id       TEXT,
    commander     TEXT NOT NULL,
    state         TEXT NOT NULL CHECK (state IN ('open', 'mitigated', 'resolved')),
    opened_by     TEXT NOT NULL,
    opened_at     TEXT NOT NULL,
    mitigated_at  TEXT,
    resolved_at   TEXT,
    postmortem    TEXT
) STRICT;
CREATE INDEX incidents_state ON incidents (state, opened_at);

-- The timeline: notes, evidence, status changes, drafts for people to send.
CREATE TABLE incident_events (
    incident_id TEXT NOT NULL,
    seq         INTEGER NOT NULL CHECK (seq >= 1),
    at          TEXT NOT NULL,
    actor       TEXT NOT NULL,
    actor_name  TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('note', 'evidence', 'status', 'comms')),
    words       TEXT NOT NULL CHECK (length(words) BETWEEN 1 AND 2000),
    PRIMARY KEY (incident_id, seq)
) STRICT;
