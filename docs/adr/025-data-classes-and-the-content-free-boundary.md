# ADR-025: Data classes and the content-free boundary

Status: Proposed, 7 Nov 2026 (data, [09 §2](../09-data-architecture.md#2-data-domains-and-classes) and [§7](../09-data-architecture.md#7-tenancy-and-isolation-in-every-store))

## Context

The platform stores eleven kinds of data, from signed public pages to visitors' conversations, orders and remembered preferences. Several documents already promise that cross-site views are content-free (05 §6.3, 08 principle 5, K-I19), that `SiteId` is in every row (02 tenet 6), and that deletion reaches every store (03 §5). None of them says which data may go where, so each new store, topic or dashboard would have to decide again.

## Decision

- Every field the platform stores carries one of five classes:
  - **C0 public:** signed releases and published facts;
  - **C1 content-free:** ids, kinds, durations, counts, digests and money;
  - **C2 business-confidential:** owner facts, drafts, playbooks, decisions, evidence;
  - **C3 personal:** conversations, leads, orders, bookings, visitor memory, visitor artifacts, contact details;
  - **C4 secret:** keys, tokens and credentials.
- **C2 and C3 never leave their site's stores.** On a single node those are the site's directory. In the fleet they are the site's Postgres schema, reached only through the site's own database role, plus its Object Storage prefix. Anything that spans sites (Kafka topics, the lakehouse, telemetry, fleet learning, operator and partner views) receives only C0 and C1. The owner, and people the owner grants, read C2 and C3 inside the site.
- **C4 is never stored in a data store in plaintext.** Keys live only in the key holders: the Model and Effects Gateways ([ADR-004](004-two-key-holders.md)), and `agentrix-keys`, which wraps each site's data key ([ADR-030](030-data-keys-have-their-own-holder.md)).
- **Per-person sign-in secrets are the one exception, and they are stored sealed** (amended 7 Nov 2026 for d-auth-passkey-first; [09 §2.3](../09-data-architecture.md#23-sign-in-credentials-m11)). Password hashes and authenticator (TOTP) seeds are C4:
  - The data store keeps them only as sealed blobs, bound to their person and site.
  - The blobs open only inside `agentrix-keys`, which verifies a password or code and never returns the plaintext.
  - Passkey public keys and linked social sign-ins are C3. Provider tokens are not kept.
- **Events cross the boundary only as typed, versioned, content-free records**, written to an outbox in the same transaction as the row they describe. There is no table-level change capture of C2 or C3 tables.
- Every store keys by `SiteId` and isolates by site, in one of these ways:
  - a directory;
  - a schema with a per-site role (adk-rust's tables have no tenant column, so row-level security alone cannot isolate them);
  - row-level security on shared control tables;
  - an object prefix;
  - a key prefix;
  - a partition.
  
  The cross-tenant suite ([03 §8](../03-trust.md#8-how-trust-is-tested)) tests each store's isolation.
- C3 objects (visitor artifacts, exports, legacy archives, backups) are encrypted with a per-site data key, wrapped by `agentrix-keys` ([ADR-030](030-data-keys-have-their-own-holder.md)). Destroying the key makes every copy unreadable.

## Consequences

Erasure and residency get simpler: the lake, Kafka and telemetry hold no personal content, so a visitor's erasure touches only the site's own stores, plus tombstones for pseudonymous subject hashes (ADR-028). Owner analytics that need detail must run inside the site's own stores (ADR-029). Event types need review, because a careless field could carry content across; the event crate's types are the gate, and a lint rejects free-text fields in boundary events.
