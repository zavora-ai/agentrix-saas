# ADR-013: No swarm inside a site; fleets learn through signed artefacts and private aggregates

Status: Proposed, 7 Nov 2026

## Context

Swarm architectures (peers that negotiate, vote, bid or gossip without a controller) are attractive for scale and resilience. A WebAgent site must stay accountable to one owner, inside a charter and a budget, with receipts. The platform also wants each site to benefit from what other sites learn, without leaking one business's data to another (A-I22). adk-rust has no swarm runtime and no cross-tenant learning; agency mode in `adk-website` refuses every cross-site request.

## Decision

- **Inside a site:** coordinator and specialists only. No self-organising swarm, no debate loops, no task markets, no agents creating agents.
- **Fan-out and fan-in** is used for bulk, stateless, sandboxed work: the Move's crawl, render and parity per page, translations, fleet re-renders.
- **Ensembles** are used narrowly for high-stakes classification (quarantine verdicts, injection screening, parity disagreements); disagreement goes to a person.
- **Between sites:** no peer-to-peer agent traffic. Agency fleets are supervised: a kernel per client site, authority through charter roles, release trains for updates, a fleet freeze.
- **Fleet learning:** only signed, reviewed artefacts cross sites (playbook revisions, detector updates, packs), derived from content-free ledger rows and aggregates that pass a privacy gate (minimum cohorts, k-anonymity, differential privacy). Every revision passes the evaluation suite and a Zavora review, ships as a signed pack, and applies to a site only by its owner's decision. Conversations, leads, owner facts, prompts and customer content never leave their site.

## Consequences

Learning is slower than pooling data, but it keeps the site wall and owners' trust, and every improvement is auditable code. Contribution of aggregates is opt-in per site (to be confirmed by James). Cohort statistics and pack channels are upstream enhancement U11. See [05-webagents.md §6](../05-webagents.md#6-swarms-judged-honestly).
