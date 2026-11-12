# ADR-008: Vultr is the primary cloud

Status: Proposed, 7 Nov 2026

## Context

The platform needs inference with tool calling, vision, embeddings, safety and decision models; VMs that can run microVMs; S3-compatible storage; a CDN; DNS; managed databases; and a Marketplace to distribute through. Vultr provides all of these, and Agent Rush is the first proof.

## Decision

Run on Vultr. The first release in Atlanta, the MVP control plane in Amsterdam, and African serving from Johannesburg. Frankfurt holds standby compute and a database replica only, because it has no Object Storage (7 Nov 2026). Every model call uses Vultr's Inference API. Self-hosted models on GPUs are out of scope (James, 7 Nov 2026). Vultr does not publish where Inference runs, so owners are told the region is unconfirmed until Vultr confirms it in writing ([02 §6](../02-architecture.md#6-the-model-plane)).

## Consequences

One provider means one bill, one IAM and one partner relationship. It also brings its constraints: no egress filtering in firewall groups (we enforce it on hosts), Auto SSL only for Vultr DNS (we run ACME), and no VX1 or Object Storage in Johannesburg (releases replicate in).
