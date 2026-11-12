# ADR-024: Billing lives in the open core; Zavora's books live in zavora-era

Status: Proposed, 7 Nov 2026 (control plane, [08 §4.2](../08-control-plane.md#42-money) and [§11](../08-control-plane.md#11-what-the-architecture-must-add))

## Context

Self-hosters, agencies and Zavora Cloud all need rating, invoices and partner statements. The platform is MIT ([ADR-002](002-open-core-mit.md)), and Zavora already keeps its accounts in zavora-era.

## Decision

- An `agentrix-billing` crate in the MIT repository does rating, invoices, dunning, partner statements and payouts.
- It posts to an accounting sink: zavora-era for Zavora Cloud, an export for everyone else.
- Plans and prices are data (a plan catalogue), and every price is a labelled hypothesis until validated.
- Usage is metered by the Model Gateway and reconciled with Vultr's `/v1/usage` token counts. The price comes from the account's bill, with forecasts at the flat rate as the ceiling.
- Owners pay AI usage above their included budget only after raising their cap on a decision card.

## Consequences

Self-hosters and agencies can bill without Zavora, which keeps the open core honest. Zavora's own books stay in its ERP. Three price lists exist in Zavora's systems today, and James must settle which customers see (08 §13).
