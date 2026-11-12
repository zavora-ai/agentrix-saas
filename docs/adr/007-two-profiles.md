# ADR-007: One codebase, two deployment profiles

Status: Proposed, 7 Nov 2026

## Context

The same platform must run as a hackathon entry, as a Vultr Marketplace self-host, and as Zavora's managed fleet.

## Decision

The single-node profile uses one control VM plus one sandbox VM, SQLite and a local CAS mirrored to Object Storage. The fleet profile uses VKE, Managed PostgreSQL, Valkey, Object Storage, the CDN, and an autoscaled VX1 runner pool. Store and queue traits make the profiles a configuration choice.

## Consequences

The first release is real product code, not a demo. Every store trait needs two implementations and CI that runs both.
