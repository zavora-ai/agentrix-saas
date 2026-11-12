# ADR-018: The tenant path is operator → partner → organisation → site

Status: Proposed, 7 Nov 2026 (control plane, [08 §2](../08-control-plane.md#2-tenancy-and-roles))

## Context

`SiteId` threads tenancy through every API, row and log line ([02 §0](../02-architecture.md#0-tenets) tenet 6). Agencies, resellers and Zavora itself sit above sites, and each needs scoped authority without weakening the site wall (A-I22).

## Decision

- `SiteId` extends to a **tenant path**: operator → partner → organisation → site. Every token, row, metric, receipt and log line carries the part of the path it needs, and a cross-tenant action cannot be expressed.
- **The organisation always owns its sites.** A partner's access is a grant from the organisation, revocable at any time; a site can leave a partner or the platform without the partner's consent.
- **Authority flows down only by signed grant**: a charter object with a grantee, a scope, an expiry and the grantor's passkey signature (or control's attestation of the grantor's MFA step-up), verified by the gateways like a standing grant. Being higher in the path gives platform levers, never an owner's decisions.
- Partners do not nest in v1.
- Anything that spans more than one site sees ids, kinds, durations, counts and money only (K-I19, A-I22).

## Consequences

Agencies get real, revocable authority without a back door into client data. adk-rust has per-site charter roles but no partner-wide roles or time-boxed grants (08 §12, CP9), so the platform stores and enforces grants in `agentrix-control` until it does.
