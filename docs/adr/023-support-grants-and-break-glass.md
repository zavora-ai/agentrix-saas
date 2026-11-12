# ADR-023: Support access is an owner-signed grant; break-glass needs two people

Status: Proposed, 7 Nov 2026 (control plane, [08 §2.5](../08-control-plane.md#25-delegated-administration); [05 §3.6](../05-webagents.md#36-autonomy-pauses-and-the-kill-switch))

## Context

Support and incident response sometimes need to look inside a site. Owners are promised that their site and data are theirs ([03 §1](../03-trust.md#1-promises)), and operator WebAgents never read site content ([ADR-017](017-zavora-runs-on-zavora.md)).

## Decision

- **Support access** is a grant the owner signs from a support ticket, with a passkey or the MFA step-up. It is time-boxed (1–72 hours, 4 by default), scoped to named areas (never payment data), read-only unless stated, revocable mid-session, and receipted for every access.
- It goes to named Zavora people by default. A Support WebAgent reads only typed diagnostics under a grant (ledger rows, configuration, receipts, job status); it reads conversations, leads or customer records only if the owner's grant names them.
- **Break-glass** needs two operator people (security plus a director), each approving with a passkey or the MFA step-up, never one person twice, for an active SEV1/SEV2 or a legal order. It covers one site, read-only by default, for 1 hour, renewable once. The owner is told at the time, and the access is reviewed in the postmortem.
- **The platform hold is separate from the owner's freeze.** Neither side can lift the other's. During a hold the operator may roll the site back only to its previous signed release; the owner rolls forward with the passkey or the step-up.

## Consequences

Zavora can support customers and handle incidents without a standing back door. Support is slower when an owner is unreachable, and break-glass needs named approvers and backups (08 §13).
