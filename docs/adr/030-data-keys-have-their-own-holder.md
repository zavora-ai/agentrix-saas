# ADR-030: Data keys have their own holder

Status: Proposed, 7 Nov 2026 (architecture, answering the data workstream's A7; [09 §7](../09-data-architecture.md#7-tenancy-and-isolation-in-every-store))

## Context

Each site has a data key that seals its personal (C3) objects, exports and backups. Destroying the key at closure makes every copy unreadable ([ADR-028](028-one-erasure-and-export-path.md)). The data key is wrapped by a key-encryption key (KEK), and the question is who holds the KEK.

[ADR-004](004-two-key-holders.md) names two key holders. The Model Gateway holds inference keys. The Effects Gateway holds the credentials that act on the world: payments, messaging, DNS, social and, since [ADR-021](021-infrastructure-changes-are-effects.md), infrastructure. Putting the KEK in the Effects Gateway's vault would be the least new machinery, but:

- One compromise would give an attacker both the power to act and the power to read every site's personal data.
- Unwrapping is on the read path of the edge, the kernel and control. The Effects Gateway would become a dependency of answering a visitor, and its outage would stop reads that have nothing to do with effects.
- Its access pattern (rare, approved, fingerprinted effects) differs from a key service's (frequent, token-scoped unwraps). One set of rate limits and alarms cannot fit both.

## Decision

- **A third key holder, `agentrix-keys`, holds the KEKs.** It only wraps and unwraps data keys. KEKs never leave it, and it holds no other credential.
- **There is one KEK per cell, under two-person control.** Rotating a KEK re-wraps data keys and never re-encrypts data.
- **An unwrap requires a capability token for that site** minted by `agentrix-control` ([05 §3.1](../05-webagents.md#31-identity)). The unwrapped key lives in the caller's memory for that use only. Every unwrap is counted per site and principal. Anomalies raise an alarm, and the counts are content-free (C1).
- **Closure destroys the site's wrapped data key in `agentrix-keys`** and records a receipt in the erasure ledger.
- **Implementation:** a small Rust service, or OpenBao's Transit engine behind the same API. Either way the platform depends only on the wrap and unwrap API.
- **Single node:** `agentrix-keys` runs as its own process with the KEK sealed in a host secret file. This separation is weaker than the fleet's, as with the two gateways on one host, and the self-hosting guide says so.
- **ADR-004 is amended:** three holders, each with one kind of secret. Online HTTP-signature keys (Web Bot Auth) may move to `agentrix-keys` as a signing service later. That is not decided here.

## Consequences

Reading a site's personal data and acting for it need two different compromises. The Effects Gateway stays off the read path. One more service needs high availability in the fleet, because an `agentrix-keys` outage stops reads of sealed data, but not of public pages or content-free state. Per-site keys for adk-rust's own encrypted payloads still need a key resolver upstream (09 §10, D6).
