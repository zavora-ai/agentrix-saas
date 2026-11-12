# ADR-019: Cells are the unit of deployment and blast radius

Status: Proposed, 7 Nov 2026 (control plane, [08 §3.2](../08-control-plane.md#32-cells-and-regions))

## Context

The fleet profile ([ADR-007](007-two-profiles.md)) must grow past one cluster and one region, serve African sites, keep residency options open and limit how far a bad change or an outage can spread.

## Decision

- A **cell** is one region's complete stack: a VKE cluster (edge, control, kernel and gateway pools), Managed PostgreSQL with a replica, Managed Valkey, Object Storage, CDN pull zones and a VX1 runner pool outside Kubernetes.
- Each site has one **home cell** for its kernel and stores. Inside a cell, kernels are sharded by a consistent hash of `SiteId` with leases and mirrored governance ([ADR-015](015-tenant-kernel-per-site.md)).
- A thin **global layer** (fleet registry, rollout controller, billing, the operator tenant, console and partner APIs) runs in the primary cell with a standby in a second.
- Placement follows residency, latency and headroom. A cell has a site cap (a hypothesis to measure); beyond it a new cell opens.
- Moving a site between cells is a scripted migration with a seconds-long hold, shown to the owner as a platform notice.
- Planned cells: `atl-0` (the first release), `ams-1` (first full cell, global primary), `fra-1` (standby and DR), `jnb-1` (edge only until Johannesburg offers VX1 and Object Storage).

## Consequences

An incident or a bad wave is contained to a cell. Release trains advance cell by cell. Each cell carries fixed costs, so cells open on evidence of demand, not in advance.
