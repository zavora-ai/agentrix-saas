-- W1 · typed signals from the public desks to the back office (ADR-011; awp_types::Signal).
-- Typed fields only; free text is awp_types::Bounded, marked untrusted and redacted before it
-- is stored. The back office reads these rows, never a visitor's raw thread. Writer:
-- agentrix-kernel only.
CREATE TABLE webagent_signals (
    signal_id  TEXT PRIMARY KEY,
    site_id    TEXT NOT NULL,
    desk       TEXT NOT NULL,          -- the desk that emitted it (concierge, desk-cakes, agent-desk)
    kind       TEXT NOT NULL,          -- SignalKind::name(): content_gap, visitor_intent, …
    body       TEXT NOT NULL,          -- awp_types::Signal JSON
    wakes      TEXT NOT NULL,          -- JSON list of the WebAgents whose wake set names this kind
    emitted_at TEXT NOT NULL,
    seen_at    TEXT                    -- when a back-office WebAgent took it off the agenda
) STRICT;
CREATE INDEX webagent_signals_site ON webagent_signals (site_id, emitted_at);
