# ADR-004: Only the Model Gateway and the Effects Gateway hold secrets

Status: Proposed, 7 Nov 2026

## Context

Agents read untrusted text all day, so any agent may be steered by injection. If agents hold credentials, steering becomes theft.

## Decision

Vultr Inference keys live only in the Model Gateway, ideally one subscription per site or agency. Payment, messaging, DNS and social credentials live only in the Effects Gateway. Agents receive short-lived capability tokens scoped to site, tool and effect class.

## Consequences

Injection can at worst waste a bounded budget or propose an effect that a gate refuses. Every model call and effect is metered and receipted in one place. The two gateways become critical services and need high availability in the fleet profile.
