# ADR-021: Infrastructure changes are effects

Status: Proposed, 7 Nov 2026 (control plane, [08 §0](../08-control-plane.md#0-principles) principle 7 and [§5.4](../08-control-plane.md#54-how-operator-webagents-act-on-the-fleet))

## Context

Vultr resources (instances, VKE pools, Inference subscriptions, CDN, DNS) are the platform's foundation. Applying infrastructure changes with long-lived keys from laptops or CI would bypass the owner wall's machinery and [ADR-004](004-two-key-holders.md)'s rule that only two services hold secrets.

## Decision

- A Terraform plan is an **effect preview**. Applying it is an **effect**: fingerprinted by the plan digest, approved by the number of people its blast radius needs, each with a passkey or the MFA step-up ([ADR-006](006-passkey-signed-approvals.md)), and executed exactly once by the operator's Effects Gateway, which holds the Vultr credentials.
- `terraform plan` and other code runs happen in sealed rooms; the plan's room may reach only `api.vultr.com`.
- Fleet levers (draining a node, routing a purpose to its fallback) are thin clients of `agentrix-fleet`, with receipts in the operator ledger and in each affected site's receipts.
- **Out-of-band changes are detected.** The operator ledger is reconciled with Vultr Audit Logs, and any change without a receipt raises an alarm.

## Consequences

No person, script or agent can change production infrastructure without a signed, receipted effect. Emergency changes need approvers on call, each with a passkey or an authenticator. Terraform modules ride their own trains, cell by cell ([ADR-020](020-release-trains-and-rings.md)).
