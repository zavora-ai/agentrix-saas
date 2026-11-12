# ADR-011: Two partitions, typed signals and a hub-and-spoke back office

Status: Proposed, 7 Nov 2026

## Context

Some WebAgents read untrusted text all day (visitors, email, other agents). Others publish, spend and message on the owner's behalf. Several WebAgents must also cooperate on owner requests that span their jobs. adk-rust offers partitions that deny crossings by default, typed signals, `TeamSpec` teams with `Delegate` and `Handoff` edges, and A2A.

## Decision

- **Public partition:** the Concierge's three desks. They hold only declared capabilities, `search_site`, `reply_in_thread` and `emit_signal` (A-I2).
- **Back office:** every other WebAgent. It never reads a requester's raw text (A-I3); it learns from typed, bounded, screened `SignalKind`s on the agenda.
- **Hub and spoke:** the back office is one `TeamSpec` with the Chief WebAgent as coordinator and `Delegate` edges to each specialist, at delegation depth 1 with isolated contexts (`harness_team_policy`). No `Handoff` edges. Specialists wake on their own signals and schedules, so the Chief WebAgent is not on every path.
- **No A2A between a site's own WebAgents.** In-process delegation and signals share one policy stack, one action ledger and one audit trail. A2A is for outside agents only, and an outside agent is called as a tool, never made a team member, because a remote member is governed by the remote side.

## Consequences

Prompt injection in any inbound text can at most file a bounded signal (scenario S7). Delegation never lends authority, because each delegate runs under its own jobs' levels. The Chief WebAgent stays accountable for every owner request it accepted. See [05-webagents.md §4–5](../05-webagents.md#4-a-sites-webagents).
