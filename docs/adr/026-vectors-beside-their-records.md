# ADR-026: Vectors live beside their records

Status: Proposed, 7 Nov 2026 (data, [09 §4](../09-data-architecture.md#4-vector-strategy))

## Context

Ask, agentic site search, "teach your site" and owner facts need retrieval. 02 §7 listed Vultr's Serverless Inference vector store for the single node and pgvector for the fleet (since aligned with this ADR in `4a41c30`). The first release lists "Concierge on Vultr vector store" as a Should (04). The options are Vultr's managed vector store, pgvector in Managed PostgreSQL, an embedded Rust index, and Qdrant (a Vultr Cloud Alliance partner). They differ on isolation, export, erasure, residency, latency and cost.

## Decision

- **Fleet: pgvector in the site's own Postgres schema**, beside the rows the vectors come from, isolated by the same schema and per-site role ([ADR-025](025-data-classes-and-the-content-free-boundary.md)). A chunk and its source row are written and erased in one transaction.
- **Single node: an embedded LanceDB index in the site's own directory** (`data/sites/<site>/vectors/`, adk-rag's `LanceDBVectorStore`), so a site is still one directory to back up, export or delete. LanceDB deletes are logical until `purge_deleted` runs, so erasure always runs it. Zavora's SQLRight (embedded SQLite with HNSW) is evaluated in the pilot as a `VectorStore` backend. If it passes on recall, latency and erasure, it replaces LanceDB and a site's vectors move into its own SQLite.
- **Both sit behind adk-rag's `VectorStore` trait** (`adk-rag/src/vectorstore.rs:151`) with one `Scope` per site. The platform records index metadata on every collection: the embedding model, dimensions, the release or source version, and the index version. A model change re-embeds into a new index version, and reads switch only when the new index is complete.
- **Vultr's vector store is an optional adapter for C0 corpora only** (the signed release's public text). The first release may use it for the Concierge demo. It never holds owner facts, conversations or visitor data:
  - its location is not published;
  - export and erasure depend on its API;
  - its search returns a fixed 10 results with no scores or filters;
  - its RAG route searches on the first user message only;
  - collection ids are cut to 14 characters (its API docs, 7 Nov 2026).
  
  Those last three also rule it out for scoped, cited, reranked search beyond the demo.
- **Qdrant is the escape hatch, not the default.** If a site's corpus outgrows pgvector (a large publisher, measured by recall and latency against 02 §9's budgets), that site's collections move to Qdrant behind the same trait, one collection per site. On Vultr that means Qdrant Hybrid Cloud on VKE, which is a second stateful system, and its backups need volume snapshots that Vultr does not support today.
- **Visitor memory is not embedded.** It stays typed items read by declared capabilities (05 §4.5).

## Consequences

Erasure and export of vectors come with the records (ADR-028). There is no second copy of a site's corpus in another system to keep in step. pgvector shares the cell's database, so index memory is watched per cell, and HNSW parameters are set per site size. Embedding calls still send text to Vultr Inference in the US: `GET /v1/models` lists `qwen3-embedding-4b` in San Jose (7 Nov 2026). 03 §5 must tell EU owners so (09 §11, A9).

Three upstream changes to adk-rag are needed before the pilot (09 §10):
- `PgVectorStore` creates no ANN index today;
- the OpenAI embedder has a fixed endpoint, so it cannot call Vultr through the Model Gateway;
- collections do not record their embedding model.
