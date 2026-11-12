# ADR-005: Visitors are served signed static releases

Status: Proposed, 7 Nov 2026

## Context

Pages must be fast everywhere, including on African mobile networks, and must never change because an agent was tricked.

## Decision

Agents produce candidate releases. The LaunchGate (approvals, parity, policy, scans) signs them with Ed25519 and admits them by switching an atomic pointer. The edge serves only signed releases, from local cache, Object Storage and the CDN. Rollback switches the pointer back.

## Consequences

Page loads involve no model and no database. Tampering is detectable by anyone (`awp verify`). Dynamic features (Ask, orders, bookings) go through the edge API, not page rendering.
