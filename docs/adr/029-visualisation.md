# ADR-029: Typed analytics for owners, content-free BI for operators, documents as signed artifacts

Status: Proposed, 7 Nov 2026 (data, [09 §6](../09-data-architecture.md#6-visualisation))

## Context

Owners see numbers in the console, briefs and mission reports (07 §8). Operators see fleet health, money and cost (08 §9). WebAgents make reports, spreadsheets and decks for owners and visitors (06 §5.4). 08 principle 10 forbids invented numbers on any surface, and 07 requires briefs to cite ledger rows and receipts. Zavora already has document engines (zavora-xlsx, zavora-slide, zavora-docx, zavora-printpdf), and 08 §13 leaves the observability backend open.

## Decision

- **Owner analytics are typed queries, not free SQL.** An `agentrix-analytics` service answers a catalogue of named metrics with DataFusion, over two sources:
  - the site's own records (C0–C3, the owner's own data);
  - the site's slice of the lake (C0–C1).
  
  Every number carries its method, its window and links to the rows or receipts it came from. The console renders it as plain words, a number and a small server-rendered chart. When the Chief WebAgent answers "how did Easter go?", it calls the same catalogue as a tool, so the number it states is the one the console shows.
- **Operator BI reads gold tables only.** The operator console's Fleet and Money views use the same service over content-free gold tables. Analysts use DuckDB and Apache Superset (Apache-2.0) behind single sign-on. Grafana serves SLO and burn-rate dashboards over the observability backend, for Zavora's SRE and on-call only. Partners see the same views, filtered by tenant path.
- **WebAgent documents are artifacts.** Charts, reports, spreadsheets and decks are generated with Zavora's document engines from typed query results. Each is stored in the site's content-addressed store, signed, and carries a provenance block naming its queries and data window. Documents made from untrusted uploads are parsed only in sealed rooms.
- **No third-party analytics scripts on sites.** Page and Ask analytics come from the edge's own content-free events (09 §5.3).

## Consequences

Owners and the Chief WebAgent can never disagree about a number. Analysts cannot reach personal data through BI, because BI connects only to the content-free lake. The metric catalogue becomes a versioned artifact, so changes go through review like playbooks. Superset and Grafana are extra services to run in the global layer. Self-hosters get the console's analytics without them.
