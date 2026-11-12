# ADR-028: One erasure and export path across every store

Status: Proposed, 7 Nov 2026 (data, [09 §8](../09-data-architecture.md#8-lifecycle))

## Context

03 §5 promises one-click export and deletion that reaches every store, including retrieval indexes, and 06 §9 promises visitor memory erased within an hour. A site's data sits in up to eight places: records, vectors, objects, fast state, the site log, Kafka, the lake and backups. Some of it also sits outside the platform, at Vultr Inference (uploaded files) and in owners' connected tools.

adk-rust v3 already has a subject-rights framework in `adk-core/src/subject/`:
- `SubjectDataStore` (`rights.rs:143`), where each store declares `Hard`, `Eventual{within}`, `Unsupported` or `External` support and implements `export` and `erase`;
- `SubjectRights` (`rights.rs:372`), which marks the subject `Erasing`, revokes consent, erases each store, writes an `Erasure` audit record and then forgets the subject.

This framework came from the adk-rust #1283 epic, which #1334 closed. Adapters exist for session, memory, artifact, RAG, the audit log and the edge beacon. The following have no adapter or only a partial one:
- the kernel's ledger, charter, KPI, autonomy and signal stores;
- harness facts, which have a soft "forget" only;
- telemetry;
- adk-payments;
- adk-website, whose `SiteSubjectData` erases contacts but leaves leads, orders and bookings behind.

There is also no erasure or export for a whole tenant. The framework's answer is one `SubjectRights` per site, scoped with `with_app`; it declined a tenant field in adk-core.

## Decision

- **The platform builds on `SubjectRights`, not beside it.** An erasure orchestrator in `agentrix-control` takes an `ErasureRequest`:
  - scope `Subject` (a visitor or customer), `Record`, or `Site` (closure);
  - the site;
  - a subject reference.
  
  For `Subject`, it runs the site's own `SubjectRights` (one per site, scoped with `with_app`) over every registered `SubjectDataStore`, including the platform's own stores (leads, orders, artifacts, conversations). For `Site`, it drops the site's schema or directory and prefix and destroys its data key. Each store returns an **erasure receipt** with counts and digests. The request completes only when every store has answered, and its receipt is chained into the site log.
- **Deadlines:**
  - visitor memory within one hour;
  - other subject requests within 30 days;
  - closed sites 90 days after closure (08 §4.2).
- **Payment records are pseudonymised, not deleted**, because they carry legal retention duties.
- **Content-free stores** (Kafka, the lake, telemetry) hold no C3. Pseudonymous subject hashes in the lake are removed by Iceberg delete files on the subject's hash. Kafka topics age out within their retention.
- **Backups are never edited.** Every erasure is recorded in an erasure ledger, and any restore replays the ledger before the store serves again. Backups age out on their schedule. For a site, destroying its data key makes its encrypted copies unreadable at once.
- **Export uses the same registry in reverse.** `SubjectDataStore::export` serves a subject's export. A site export runs the site's own `SubjectRights` (scoped with `with_app`) and adds a dump of the site's schema or directory, so no new adk-rust API is needed. The export is a signed bundle:
  - records as JSON Lines and Parquet;
  - the release as a static site;
  - receipts with `agentrix audit verify`;
  - CSV for spreadsheets.
  
  A visitor's `memory.export` uses the same path with subject scope.
- **Outside stores:** files uploaded to Vultr Inference are deleted through its API. Owners' connected tools are not erased by Agentrix; the owner is told which tools may hold copies.

## Consequences

Every new store must register a `SubjectDataStore` and a site export before it can hold C2 or C3, and CI checks that the registry covers every store in the profile. The adk-rust stores without adapters need them upstream (09 §10). Restore drills (08 §3.9) gain one step: verify that the erasure ledger was replayed.
