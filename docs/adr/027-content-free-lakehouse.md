# ADR-027: A content-free lakehouse on Object Storage

Status: Proposed, 7 Nov 2026 (data, [09 §5](../09-data-architecture.md#5-the-lakehouse))

## Context

Owners need trends (answers handled, orders, spend). Operators need fleet health, money and cost to serve. Fleet learning needs aggregates behind a privacy gate (05 §6.3). Billing needs metered usage. These all read history across many runs and, for operators, across sites. Running such queries against the transactional stores would load them and blur the site wall.

## Decision

- **One lakehouse per cell, on Vultr Object Storage, in open formats:** Parquet data files under Apache Iceberg table metadata, with the catalog in the cell's Postgres (single node: SQLite).
- **It is content-free:** it holds only C0 and C1 ([ADR-025](025-data-classes-and-the-content-free-boundary.md)). Subjects appear only as keyed hashes.
- **Ingestion is one path.** Producers write typed events to an outbox in the same transaction as their change. A relay publishes them to Managed Kafka (fleet) or an in-process channel (single node). The Rust ingester `agentrix-lake` turns them into Arrow batches, writes Parquet and commits Iceberg snapshots, recording source offsets and code version in each snapshot.
- **Layers:**
  - bronze: events as received;
  - silver: conformed facts per domain;
  - gold: aggregates per site, partner and fleet.
- **Engines:** DataFusion inside platform services (the owner analytics API, the operator console, billing). DuckDB for analysts, BI tools and exports. No query engine is on the page-load or Ask path.
- **Phasing:**
  - The first release has no lake: the site log and SQLite answer everything.
  - The pilot writes date-partitioned Parquet that DuckDB reads directly.
  - The fleet adds Iceberg metadata over the same files, plus erasure deletes and compaction.
  - If iceberg-rust's writer lacks a feature the platform needs when it is built, the ingester writes Delta Lake through delta-rs behind the same interface.
- Telemetry (metrics, traces, logs) is not the lake. It goes to the observability backend with short windows (09 §5.6).

## Consequences

Open formats keep exports and an owner's own analytics tools possible without Zavora. A content-free lake needs no per-visitor erasure beyond hash tombstones, and needs no support grant to query. The cost is a second write path (outbox and relay) and a small Rust service to run. Kafka is a cell dependency only in the fleet profile.
