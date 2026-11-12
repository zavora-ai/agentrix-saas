# ADR-017: Zavora runs on Zavora

Status: Proposed, 7 Nov 2026 (control plane, [08 §0](../08-control-plane.md#0-principles) and [§5](../08-control-plane.md#5-operator-webagents))

## Context

Zavora Cloud must operate a growing fleet of sites, agencies and Vultr infrastructure with a small team. The platform already has governed WebAgents, signed approvals, receipts and release gates. A separate, ungoverned operations stack of scripts, consoles and credentials would be the weakest link, and would never exercise the product the way customers do.

## Decision

- Operations are done by **operator WebAgents** in an **operator tenant**: ordinary WebAgents with principals `urn:adk:agent:op:{operator}/{role}`, their own kernel, charter (an `awp_types::Charter` of kind Business with empty public terms), ladder, budget and receipts.
- They follow the same governed path as site WebAgents ([05 §3.4](../05-webagents.md#34-the-governed-tool-path)), plus an operator capability check in the platform policy layer. Both partitions apply: support, abuse and growth intake are public desks.
- They propose, investigate, draft and pull reversible levers. People decide everything consequential, with a passkey or the MFA step-up ([ADR-006](006-passkey-signed-approvals.md)).
- They never run in a customer's kernel, never read site content without an owner-signed grant ([ADR-023](023-support-grants-and-break-glass.md)), never approve a site's card, change a dial or touch payment credentials. An operator token is never valid at a customer's Effects Gateway connections.
- Zavora is the first customer of every operator feature, and its own sites are ring 0 of every release train ([ADR-020](020-release-trains-and-rings.md)).

## Consequences

Operations get the same audit, budgets and kill switches as customers. Every gap in the product shows up in Zavora's own operations first. The operator tenant is one more tenant to run and secure, and its charter's red lines must be tested like any site's.
