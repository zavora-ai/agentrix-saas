# ADR-015: A kernel per site, started on wake

Status: Proposed, 7 Nov 2026

## Context

Each site's WebAgents need their own processes, agenda, charter, ladder, budget and kill switch. `adk-website`'s agency mode (`serve_agency`) runs every site in one kernel, one database and one kill switch, isolated by partition. adk-rust's managed `TenantKernel` gives each tenant its own kernel and stores, but it cannot host a website yet (no `CharterKit`, KPI tracker or per-agent targets on its builder; no website kits in `HarnessDef`). A started kernel polls its stores on three queues even when every process sleeps, and neither `Kernel` nor `ManagedKernels` has lazy activation or eviction.

## Decision

- Every site gets its own kernel (the platform's **TenantKernel**) over its own stores: an SQLite file per site on a single node, a Postgres schema per site with row-level security in the fleet.
- Until the managed form can host a website (U16), each site's kernel is built with `KernelBuilder`, `KernelStores::open`, `require_durable()`, one target per WebAgent and `CharterKit::install`. It moves to the managed `TenantKernel` when U16 lands.
- A node-wide wake front owns cron, webhook, queue and signal ingress and starts a site's kernel on its first wake; idle kernels are stopped, parking their processes. Missed cron ticks replay under each wake's `MissedFirePolicy`. Lazy activation and a shared claim loop are upstream enhancement U2.
- The platform does not use `adk-website`'s agency mode.

## Consequences

One site's freeze, store fault or runaway job cannot touch another's. A dormant site costs storage only. The wake front is new platform code with its own tests (no missed or duplicated wake across start and stop). See [05-webagents.md §8](../05-webagents.md#8-deployment).
