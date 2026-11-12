# 03 · Data: entities, stores, migrations and sample seeds for production

Status: draft v0.1, Sat 7 Nov 2026, 23:59 EAT (from `date`). Line numbers are at `main` f280031.
Read with [how-we-work.md](how-we-work.md) and [02-architecture.md](02-architecture.md).
Sources: `docs/09-data-architecture.md` (all), ADR-025 to ADR-030, the data and billing parts of
`docs/08-control-plane.md`, `docs/05-webagents.md` §2–4, `docs/06-website-experience.md`,
`docs/07-owner-console.md`, `crates/agentrix-types/src/*`, `crates/agentrix-node/src/db/`
and both migrations, `auth/schema.sql`, `claim/schema.sql`, `crates/agentrix-models/src/meter.rs`,
`crates/agentrix-move/src/store.rs`, `specs/first-release/design.md` §5, and every concept page.

This page says what data the whole product holds, where each kind lives, what exists today,
how parallel sessions add tables without colliding, what the three sample sites are seeded with,
and which numbers on the concept screens have no source yet. Doc 09 stays the design. This page
turns it into a build list.

---

## 0. The short version

1. **139 entities in 11 domains** (§1). The first release stores 24 of them in 24 tables, all in **one
   shared `control.db`** (§2.1). Nothing in the first release uses an adk-rust store: no crate calls
   `adk_kernel`, `adk_session`, `adk_harness` or `adk_website` stores, so 09 §3.3's per-site
   directory (`docs/09-data-architecture.md:264-283`) does not exist yet.
2. **Whole domains are missing:** commerce (catalogue, stock, carts, orders, payments, refunds,
   bookings), content (posts, corrections, digests), customers (profiles, visitor accounts,
   consents, addresses), conversations (threads and turns), WebAgents (specs, dials, missions,
   facts), billing (plans, subscriptions, invoices), tenancy above the site (operators,
   partners, orgs, memberships), and the trust ledgers (grants, erasure ledger, exports).
3. **Existing tables need changes** before production. The worst: personal data in the shared
   database (`decisions.card` holds a customer's name and email,
   `crates/agentrix-node/src/decisions/mod.rs:797-799`); password hashes stored unsealed
   (`crates/agentrix-node/src/auth/schema.sql:10`); CHECK lists that refuse the product's own
   kinds (`0001_control.sql:125`, `:221`); no hash chain on receipts; no site lifecycle,
   tenant path, time zone or currency on `sites` (§2.4).
4. **Production v1 should run one cell:** Managed PostgreSQL with a schema and role per site,
   pgvector beside the records, Object Storage with the platform's copier to Milan, an outbox
   relayed straight to Parquet, and no Kafka or Iceberg until a second cell (§3.3).
5. **Migrations:** the range rule in how-we-work.md stays for the shared `control.db` until the
   store layer lands. Production uses **one migration set per crate and store**, each with its
   own migrations table (sqlx 0.9 supports this), timestamp versions, both SQL dialects, and a
   catalogue entry per table naming its one owning workstream (§4).
6. **Samples:** Kilele Bakery (local services, KES), a perfume shop (WooCommerce, KES) and The
   Kestrel Post (national news, KES). Not zavora.ai or classyperfumes.shop, which are real
   (§5). Seeds are export bundles, labelled Sample on every surface, and can never cause a real
   effect.
7. **56 numbers and lists on the concept screens have no source** (§7). Most need
   threads, a site log, missions, catalogue and stock, per-WebAgent spend, or the lake.

---

## 1. Entity model for the whole product

### 1.0 Legend

**Class** (ADR-025, `docs/09-data-architecture.md:97-103`): C0 public, C1 content-free,
C2 business-confidential, C3 personal, C4 secret. A record takes its most sensitive field's
class.

**Store** (the system of record):

| Code | Single node | Fleet |
|---|---|---|
| CTL | `data/control.db` | Control database, global layer, RLS on the tenant path |
| REC | `data/sites/<site>/data/records.db` (platform tables) | Schema `site_<id>`, role `site_<id>` |
| KRN | `kernel.db`, `kernel/sessions.db` (adk-kernel, adk-session, adk-harness) | Same site schema |
| EDGE | `edge.db` (adk-server `adk_awp_*`, adk-website `website_*`) | Same site schema |
| LOG | `log.db`, the site log on `AuditLogStore`, hash-chained | Same site schema |
| CAS | `data/cas/sha256/…`, C3 sealed with the site key | `agx-<cell>-cas`, prefix `sites/<site>/` |
| REL | The signed release, in CAS | `agx-<cell>-releases` |
| VEC | LanceDB `vectors/` | pgvector in the site schema |
| KEYS | `agentrix-keys` process | `agentrix-keys` service (ADR-030) |
| LAKE | `data/lake/` Parquet (pilot) | Iceberg per cell |
| EXT | Outside the platform: payment providers, the owner's connected tools | Same |

**Erasure** (ADR-028): **H** hard delete through a `SubjectDataStore` adapter; **P**
pseudonymise (legal retention, payment records); **R** redact the body, keep the hash so the
chain verifies; **S** goes only with the site's closure (no subject data); **T** a retention
job; **K** crypto-shredded with the site's data key; **X** outside, the owner is told.

**Export** (`docs/09-data-architecture.md:910-932`): **B** the site bundle (JSON Lines and
Parquet); **B+csv** also CSV; **Sx** the person's own export (`memory.export`, "Download my
data"); **St** the static site; **N** not exported (rebuildable or secret).

Retention marked *h* is a hypothesis to measure in the pilot.

### 1.1 Platform (21)

| Entity | Key fields | Class | Store | Retention | Erase · Export |
|---|---|---|---|---|---|
| Operator | `operator_id`, name, published key ids | C1 | CTL | Life of the platform | — · N |
| Partner | `partner_id`, kind (agency, reseller, referral, dedicated), name, brand tokens, wholesale price, payout ref (held by the Effects Gateway) | C2; contacts C3 | CTL | Life of the partnership, then tax law | P contacts · B (partner's own) |
| Organisation | `org_id`, name, country, tax PIN, billing contact, `partner_id`, tenant path | C2; contacts C3 | CTL | Until 90 days after its last site closes; invoices per tax law | S, P on invoices · B |
| Account (person) | `account_id`, email, display name, locale, created | C3 | CTL | Life of the account (`docs/03-trust.md:82`) | H; signer refs P · Sx |
| Membership | `account_id`, scope (operator, partner, org), platform role (`op.*`, `partner.*`, owner, manager, viewer, billing; 08 §2.3), granted by, at | C3 | CTL | Life; 13 months after removal for audit *h* | H with account · Sx |
| Passkey | credential id, `account_id`, public key, sign count, authenticator model, device name, last used | C3 | CTL | Until removed, then deleted at once | H · Sx (names and dates) |
| Password secret | `account_id`, sealed Argon2id blob | C4 sealed | CTL blob; opens in KEYS | Until removed | H · N |
| Authenticator secret | `account_id`, sealed TOTP seed, last accepted time step (C1) | C4 sealed | CTL blob; opens in KEYS | Until removed | H · N |
| Recovery code | `account_id`, code hash, used at | C4 (hash) | CTL | Until regenerated | H · N |
| Linked sign-in | provider (Google, LinkedIn, GitHub), provider subject, verified email, name, linked at | C3 | CTL | Until unlinked | H · Sx |
| Session | token hash, `account_id`, method, device, created, last seen, expires | C3 | CTL | Until expiry plus 30 days *h* | H · Sx |
| Sign-in event | `account_id`, at, method, outcome, device class, rough place | C3; counts C1 via `agx.auth.v1` | CTL | 13 months *h* (`docs/09-data-architecture.md:860`) | H, T · Sx |
| Device | `device_id`, `account_id`, kind (PWA push, WhatsApp number, authenticator, voice), label, paired, revoked | C3 | CTL (+ kernel device attachments) | Until revoked plus 13 months *h* | H · Sx |
| Plan | `plan_id`, version, name, prices per currency (USD, KES, NGN, ZAR, minor units), included AI budget, WebAgents and packs, limits | C0 | `plans.toml` in zavora-fleet, copied to CTL (`docs/08-control-plane.md:692`) | Every version kept | — · N |
| Subscription | `subscription_id`, org, site, plan@version, rail (PayPal, Paystack, M-Pesa), provider ref, state, period, cancel at, starts at go-live | C2 | CTL (`agentrix-billing`) | Life plus tax law | P · B |
| Invoice and line | number (`INV-2026-11-0143`), org or partner, period, lines (plan, approved AI overage, one-off), currency, tax, eTIMS state, journal ref | C2; billing contact C3 | CTL; books in zavora-era | Tax law (KRA period to confirm) | P, never deleted · B+csv |
| Platform payment | `payment_id`, invoice, rail, provider ref, amount, fee, attempts, state | C2 | CTL; credentials in the Effects Gateway | Tax law | P · B |
| Model usage | site, purpose, provider, model, tier, WebAgent, job, process, tokens in, out and reasoning, reserved and settled micro-USD, outcome, at | C1 | REC (fleet: site schema plus `agx.usage.v1`); today CTL `usage` | 25 months (`docs/09-data-architecture.md:128`) | S · B (analytics) |
| AI budget | site, scope (site, partition, WebAgent, job, mission), period, cap in micro-USD, display currency, the decision that raised it | C2 | CTL for the site cap; KRN quotas below it | Life of the site | S · B |
| Exchange rate | pair, rate, source (the site's posted rate; CBK for the books), valid from and to | C0 posted; C1 | REC for a site's posted rate; CTL for the platform's | Every rate kept (orders pin it) | — · B |
| Support ticket | `ticket_id`, org, site, channel, subject, messages, severity, state, linked incident | C2; message text C3 | CTL (`docs/08-control-plane.md:841`) | 2 years *h* | H text on request · B |

### 1.2 Site (12)

| Entity | Key fields | Class | Store | Retention | Erase · Export |
|---|---|---|---|---|---|
| Site | `site_id`, org, primary domain, name, archetype, tier (T0–T4), home cell, residency tag, IANA time zone, currency, languages, lifecycle state (lead, preview, kept, moving, live, at risk, suspended, leaving, closed), origin (real, sample, fixture), plan | C2; name and domain C0 | CTL registry plus the site's own store set | Life plus 90 days after closure (`docs/08-control-plane.md` §4.2) | S, K · manifest |
| Domain | site, hostname, role (primary, alias, preview), DNS state, certificate expiry | C0, C1 | CTL | Life | S · B |
| Domain claim | `claim_id`, site, account, domain, code, method (dns, file), evidence, state, verified at | C1; account C3 | CTL | Life of the site | S · B |
| Release | site, seq, manifest digest, signed checkpoint, state, one-line summary ("Sunday hours"), author (WebAgent or person), approving decision, signed at | C0 signed; C2 candidate | REC row; files in REL | Rollback set, then until the owner deletes | S · St |
| Live pointer | site, seq, manifest digest, updated | C0 | REC | Current | S · B |
| Page | site, release seq, route, title, kind, HTML and Markdown-twin digests, anchors, language | C0 | REL manifest; index row in REC | With its release | S · St |
| Blob index | site, digest, kind, content type, size, location (local, local+object), class, sealed flag | By content | REC (today CTL `artifacts`) | By class | Unlink then GC · with its owner record |
| Draft | `draft_id`, site, target route, version, author WebAgent, state, digest | C2 | KRN (`SiteDraftStore`) | Until merged or abandoned plus 30 days (`docs/09-data-architecture.md:121`) | S · B |
| Desk manifest | site, desk slug, name, mandate, capability subset, starters, "may promise" list, corpus scope, mark seed | C0 | REL (signed with the release; `docs/05-webagents.md:602-608`) | With its release | S · St |
| Redirect | site, from path, to path, status, source (Move or owner) | C0 | REL | With its release | S · St |
| Brand voice | tone, banned phrases, languages, approved translations | C2 | REC | Life | S · B |
| Presentation script | chapters, must-say, must-not-say, allowed sources, maximum length | C0 | REL (`docs/06-website-experience.md:848-874`) | With its release | S · St |

### 1.3 Commerce (17)

All in REC unless noted. Money is `Money { currency, minor }`
(`crates/agentrix-types/src/effect.rs:16-21`); KES has two minor digits, so KSh 450 is `45000`.

| Entity | Key fields | Class | Store | Retention | Erase · Export |
|---|---|---|---|---|---|
| Product | site, sku, name, description, family, size, attributes (notes, allergens, ingredients), images (digests), status, published release seq | C0 published; C2 draft | REC; a C0 copy in REL | Life | S · B+csv |
| Configurable product | product, option groups and choices with price deltas and rule codes, constraints, lead times, sketch parts | C0 confirmed; C2 `Assumed` | REC; REL copy (`docs/06-website-experience.md:726-749`) | Life | S · B |
| Service | `service_id`, name, duration, capacity, price, place | C0 | REC | Life | S · B+csv |
| Price list | site, version (`v14`), currency, lines (sku or rule code → amount), valid from and until, source, approving decision | C0 | REC; REL copy | Every version (quotes pin to it) | S · B+csv |
| Stock level | site, sku, on hand, as of, next batch at, sold out until, source (owner tap, POS, connected tool) | C2; C0 once shown | REC (changes too often for a release) | Current plus 13 months *h* | S · B |
| Delivery zone and slot | zone, fee, days and windows, slot capacity, slots taken | C0 | REC | Life | S · B |
| Basket | `cart_id`, site, customer or device token, lines, quote pin, expires | C3 signed in; else C2 | REC | 7 days *h* | H · Sx |
| Quote | `quote_id`, site, customer, lines with rule codes, totals, currency, rate pin, price-list version, release seq, valid until, artifact digest | C3 | REC | Owner-controlled; 13 months if no order *h* | H; P if ordered · B+csv, Sx |
| Order | `order_id` (`KB-1043`), site, customer, channel (site, agent door, WhatsApp), totals, currency, rate pin, delivery (zone, slot, address ref), note, state, decision, idempotency key, agent tier | C3 | REC | Business-record law (Kenya: to confirm) (`docs/06-website-experience.md:991-1000`) | P · B+csv, Sx |
| Order line | order, sku or configuration, quantity, unit price, rule code, amount | C3 by association | REC | As the order | P · B+csv |
| Order event | order, step (requested, approved, paid, baking, out, delivered), at, actor, receipt | C3 | REC | As the order | P · B, Sx |
| Hold | `hold_id`, site, sku, quantity, until, customer ref, standing grant | C3 | REC | Expiry plus 30 days *h* | H · B |
| Booking | `booking_id`, site, service, slot, customer, party size, state, deposit | C3 | REC (today adk-website's `website_bookings`, `docs/09-data-architecture.md:386`) | Owner-controlled | H; P if paid · B+csv, Sx |
| Customer payment | `payment_id`, order, rail (M-Pesa STK, PayPal, card through Paystack), provider ref, amount, currency, rate, fee, state, Effects Gateway receipt | C3; card and PIN never held | REC; credentials in the Effects Gateway; card data stays EXT | Legal retention | P · B+csv, Sx |
| Refund | `refund_id`, payment, amount, reason, standing rule or decision, state, receipt | C3 | REC | Legal retention | P · B+csv, Sx |
| Settlement | `payout_id`, rail, period, gross, fees, net, masked bank ref | C2 | REC | Legal retention | S · B+csv |
| AP2 mandate | `mandate_id`, agent, checkout hash, amount, chain to the approval fingerprint | C3 | REC (adk-payments keeps it in memory today, `docs/09-data-architecture.md:237`) | Legal retention | P · B |

### 1.4 Content (7)

| Entity | Key fields | Class | Store | Retention | Erase · Export |
|---|---|---|---|---|---|
| Post or story | site, slug, title, standfirst, body digest, bylines, section, tags, published at, updated at, entitlement (free, subscriber), status, release seq | C0 published; C2 draft | REC source; REL copy | Life | S · St, B |
| Author | `author_id`, name, short bio | C0 | REC | Life | S · B |
| Correction | post, at, what changed, note | C0 | REC; REL copy | Life; never deleted | S · B |
| Story timeline entry | story thread, at, line, source post | C0 | REC; REL copy | Life | S · B |
| Media | digest, alt text, credit, licence, dimensions, AI-generated flag | C0 | CAS; index in REC | With its release | S · St |
| Digest subscription | subscriber, topics, cadence (Friday 07:00), channel, double opt-in state, consent receipt | C3 | REC | Until stopped; topics deleted with it (`docs/06-website-experience.md:777-793`) | H · Sx |
| Digest edition | site, topic, date, post ids, recipients count | C0, C1 | REC | 13 months *h* | S · B |

### 1.5 Customers (10)

| Entity | Key fields | Class | Store | Retention | Erase · Export |
|---|---|---|---|---|---|
| Customer | `customer_id`, site, name, kind (person, business), emails, phones, preferred channel, locale, first and last seen, keyed subject hash | C3 | REC | Life of the account, or owner-controlled | H; P where orders exist · B+csv, Sx |
| Visitor account | `account_id`, customer, entitlements (customer, subscriber), MFA enrolled, created | C3 | REC (U19, `docs/09-data-architecture.md:399`) | Life of the account | H · Sx |
| Visitor sign-in material | passkeys (C3); password and TOTP as sealed blobs under the site's sign-in key (C4); linked sign-ins (C3); recovery codes | C3, C4 sealed | REC blobs; open in KEYS | Until removed | H · passkey public keys in the site bundle; secrets never (`docs/09-data-architecture.md:925-932`) |
| Visitor session and sign-in event | token hash, device, rough place, created, last seen | C3 | REC | 13 months *h* | H · Sx |
| Consent receipt | `cr-…`, subject, purpose (order updates, account memory, offers, visit counts), what, shared with, kept for, given, withdrawn | C3 | EDGE (`adk_awp_*` consent receipts) | Life of the consent, then proof period *h* | H (the erasure receipt keeps the hash) · Sx |
| Address | customer, label, area, lines, delivery note | C3 | REC | Until deleted | H · Sx |
| Lead | `lead_id`, contact ref digest, source desk, need, consent receipt, state, owning WebAgent | C3 | EDGE (`adk_awp_leads`) | Owner-controlled; 13 months unconverted *h* | H · B+csv, Sx |
| Customer list | `list_id` (`l-xmas25`), rule, members with their consent receipts | C3 | REC | Until deleted | H members · B |
| Visitor memory item | subject, kind, value, source turn, consent receipt, created | C3 | REC, public partition (`docs/05-webagents.md:614-618`) | Until deleted or consent revoked, within one hour | H within 1 h · Sx |
| Subject registry entry | keyed subject hash, kind, state (active, erasing, forgotten) | C1 | KRN (adk-core `SubjectRegistry`) | Life plus 90 days | Tombstone kept · N |

Device memory is not an entity: it stays in the visitor's browser and nothing is stored on the
server (`docs/05-webagents.md:631-636`).

### 1.6 Conversations (11)

| Entity | Key fields | Class | Store | Retention | Erase · Export |
|---|---|---|---|---|---|
| Thread | `thread_id`, site, channel (Ask, desk, agent door, Inbox, WhatsApp), visitor (session hash or customer), current desk, speaker (desk, owner, none), language, started, last turn, outcome (answered, doesn't say, handover, held), parked decision | C3 | REC (D15) | 90 days, owner-configurable (`docs/03-trust.md:84`) | H · B, Sx |
| Turn | thread, seq, role (visitor, desk, owner, system), desk slug, text, citations (route, anchor, release seq), tool calls, screened flag, usage ref, receipt | C3 | REC | 90 days | H · B, Sx |
| Desk session | adk-session run state for one desk turn | C3 | KRN | 90 days | H (`delete_all_sessions`) · N |
| Takeover | thread, asked at, reason, wait promised, taken by, taken at, handed back at | C3 | REC | 90 days | H · B |
| Passed question | as today, plus state, answered at, linked gap | C3 | REC (today CTL `passed_questions`) | 90 days *h* | H · B |
| Content gap | `gap_id`, cluster key, redacted sample questions, times asked, first and last asked, state, answering fact, release | C2; samples C3 | REC | Until closed plus 90 days *h* | H samples · B |
| Agent door call | `call_id`, agent signer and key id, tier (anonymous, known, verified, partner), capability, idempotency key, outcome (served, held, refused), decision, receipt | C1; arguments may be C3 | EDGE (`adk_awp_events`, idempotency) | 13 months; idempotency 30 days *h* | H arguments · B |
| Agent client | `agent_id`, signer domain, key directory, tier assignment, first and last seen | C1 | EDGE | Life | S · B |
| Visitor artifact | `artifact_id` (an access secret), kind (brief, plan, quote, comparison, deck, reading list), maker, version, sealed digest, pinned release seq and valid until, visibility, saved | C3 | CAS sealed; index in REC | 30 days unless saved (`docs/06-website-experience.md:795-846`) | H · Sx |
| Artifact share | artifact, person, permission (view, comment, choose, request), prices hidden | C3 | REC | With the artifact | H · Sx |
| Artifact version | artifact, version, who, what changed, digest | C3 | REC | With the artifact | H · Sx |

### 1.7 WebAgents (17)

| Entity | Key fields | Class | Store | Retention | Erase · Export |
|---|---|---|---|---|---|
| WebAgent | site, slug, `WebAgentSpec` version, partition, principal, base role, mark seed and colour, paused | C2 | KRN agents; spec in REC | Life | S · B |
| Job and dial | site, WebAgent, job id, level (Watch … Undo only), ceiling and reason, lowered at and why, promotion counters (reviewed approvals unchanged in 30 days) | C2 | KRN autonomy store and charter | Life; history 25 months | S · B |
| Charter | site, version, body (`Charter`, `crates/agentrix-types/src/records.rs:52`), digest, signer, signature | C2 | KRN charter store (append-only); today CTL `charters` | Life | S · B |
| Process | process id, WebAgent, job, kind, state (running, waiting, sleeping, parked), next wake, spend today, tool calls | C1 | KRN (`adk_runs`, process store) | Finished runs 30 days *h* | S · N |
| Wake | process, reason (signal kind, schedule, approval, run finished, budget replenished), at, source id | C1 | KRN; the wake front lives in Valkey | 13 months, through the ledger | S · N |
| Mission | `mission_id` (`m-3`), goal in words, KPI and baseline, target, deadline, owning jobs, budget slice, authority (may only lower), wakes, state, start and end, signing decision | C2 | REC (platform type over goals, agenda and triggers; F5, `docs/07-owner-console.md:1373`) | Life plus 25 months | S · B |
| Mission report | mission, period, measure, forecast with method, worked, didn't, next, citations | C2 | REC; document in CAS | Life | S · B |
| KPI sample | site, KPI, at, value, source | C1 | KRN `KpiStore`; `agx.kpi.v1` | 25 months (`docs/09-data-architecture.md:851`) | S · B |
| Fact | `fact_id`, text, provenance (`Known`, `Assumed`), source (owner's words, page, rows), status (active, forgotten), linked page, confirmed at | C2 | KRN (`FactStore`) in REC | Until forgotten; hard delete 30 days later | H (needs D4) · B |
| Fact embedding | fact digest, vector, model, dimensions, index version | C2 | VEC | With the fact, same transaction | H · N |
| Corpus chunk | release seq, route and anchor, text digest, vector | C0 | VEC (`corpus@<seq>`) | While its release is in the rollback set | S · N |
| Vector collection | name, model, dimensions, distance, source version, index version, chunker | C1 | REC `vector_collections` (until D3) | Life | S · N |
| Playbook version | WebAgent, job, version, body digest, approving decision | C2 | REC (adk-website learning) | Every version | S · B |
| Signal | kind (`lead_captured`, `content_gap`, `visitor_intent`, `support_escalation`, `review_seen`, `abuse_observed`), bounded fields (280 characters, screened, PII-redacted), topic, count, agenda item | C2 | KRN `SignalStore` | 13 months (needs D12) | H through D4 · B |
| Agenda item | `item_id`, origin (signal, proposal, owner), owning WebAgent, state, decision | C2 | KRN agenda | 13 months after done *h* | S · B |
| Activity ledger entry | kind, effect, duration, WebAgent, job, keyed subject hash, at | C1 | KRN `adk_kernel_ledger`; `agx.ledger.v1` | 13 months in the site and in bronze | H by subject hash (D4) · B |
| Brief | site, written at, headline, lines each with its citation, spend in words, events counted | C2 | REC (today CTL `briefs`) | 13 months *h* | S · B |

Presence (the WebAgent and desk state shown on marks) is fast state in Valkey with a TTL of
seconds, not an entity (`docs/09-data-architecture.md:370`).

### 1.8 Governance (9)

| Entity | Key fields | Class | Store | Retention | Erase · Export |
|---|---|---|---|---|---|
| Decision card | `decision_id`, site, kind (open set), raised by (WebAgent, job, agent-raised flag), title, body, effect (`EffectRequest`), fingerprint, risk, priority (P1, P2, digest), expires at, state, approval kind, method, signer, assertion, receipt | C2; C3 only by reference | REC (today CTL `decisions`) | Life plus 90 days | R (C3 fields redacted; fingerprint kept) · B |
| Approval | decision, fingerprint, method (passkey, totp), approver, kind (reviewed, one tap, spoken, batch, digest), surface, at, assertion or step-up evidence | C1; approver C3 | REC; copy in its receipt | Life plus 90 days | R (approver pseudonymised) · B |
| Standing rule | `grant_id` (`g-3`), WebAgent, job, tool, amount, recipients, frequency, expiry, signing device, signature, uses, revoked | C2 | KRN `StandingRule` (signed after U6) plus REC | Expiry, then as receipts | S · B |
| Approved plan | `plan_id`, job, items (page or recipient), limits, expiry, progress | C2 | REC (compiles to standing rules) | As its rules | S · B |
| Effect | fingerprint, site, tool, canonical-args digest, amount, status (allowed, held, executed, not executed, unknown), provider ref, gateway receipt | C1; arguments stay in the site | Effects Gateway journal; REC | Life plus 90 days | R · B |
| Receipt | `receipt_id`, digest, kind, site, subject, inputs and outputs, policy, observed, issuer, signature, previous digest, seq | C1; `detail` may hold C2 or C3 | LOG (today CTL `receipts`) | Life plus 90 days (`docs/09-data-architecture.md:126`) | R · `receipts/` in the bundle |
| Quarantined item | `item_id`, Move, title, reason, location, evidence, state, restore decision | C2 | REC (today CTL `quarantine`) | Life | S · B |
| Alarm | priority, kind, at, acknowledged | C1, C2 | EDGE (adk-website alarms) | 13 months *h* | H if about a subject · B |
| Attention setting | quiet hours (IANA zone), daily card cap (default 5), channels per priority, snoozes | C2 | REC | Life | S · B |

### 1.9 Trust (11)

| Entity | Key fields | Class | Store | Retention | Erase · Export |
|---|---|---|---|---|---|
| Support grant | `grant_id`, site, ticket, named grantees, scopes, duration (default 4 h), fingerprint, signer, revoked, each access as a receipt | C1, C2 | CTL; mirrored in the site's charter store | Life plus 90 days | S · B |
| Break-glass | id, site, two approvers, reason, scope, start and end, owner told at | C1 | CTL; operator ledger | Life of the platform | — · in the site's receipts |
| Agency role grant | site, partner, named people, charter role (Editor, Approver, Publisher), ring opt-in, yearly review | C2 | CTL; charter | Until revoked, then audit | S · B |
| Operator ledger entry | `op-…`, person or operator WebAgent, verb class, tenant-path target, approval, effect, result, previous hash | C1 | CTL, hash-chained (`docs/08-control-plane.md` §4.6) | Life of the platform | — · copies in each site's receipts |
| Audit record | kind (for example `surface.gesture`), `{gesture, verb, outcome}`, command id | C1 | LOG (`docs/05-webagents.md:908-915`) | 13 months *h* | S · B |
| Erasure request | `request_id`, scope (subject, record, site), site, subject hash, asked by, at, deadline, per-store answer (`Hard`, `Eventual{within}`, `External`) with counts and digests, completed at, receipt | C1 | CTL erasure ledger | Life of the site plus 90 days (`docs/09-data-architecture.md:858`) | Kept as proof · B |
| Export | `export_id`, site or subject, asked by, manifest digest, location, expires | C2, C3 sealed | `agx-<cell>-exports`; row in CTL | 7 days | T · — |
| Data key | site, wrapped data key, KEK id, state (active, destroyed) | C4 wrapped | KEYS | Until closure | Destroyed at closure (K) · N |
| Unwrap count | site, principal, day, count | C1 | KEYS | 13 months *h* | — · N |
| Platform hold | scope (site, partner, cell, fleet), reason, by, at, lifted | C1 | CTL | Life | — · in the site's receipts |
| Platform notice | kind (train ring, incident, hold, rollback), words, at, links | C1 | CTL | Life | — · in the site's receipts |

### 1.10 Move (12)

| Entity | Key fields | Class | Store | Retention | Erase · Export |
|---|---|---|---|---|---|
| Assessment | `asmt_…`, site, URL, state, report (findings, speed, parity preview, common questions), created, finished | C0 (public crawl); C2 report | CTL | 30 days unclaimed *h*; life once claimed | S · B |
| Preview | site, link token, expires (7 days), visits | C1 | CTL | 7 days plus 30 days *h* | T · N |
| Growth lead | email, consent receipt, source assessment | C3 | CTL | Until the preview expires plus 30 days, unless kept *h* | H · Sx |
| Move | `move_…`, site, state, created, updated | C1 | CTL | Life of the site | S · B |
| Move step | Move, seq, step, state, summary, receipt, resume data | C1 | CTL | Life of the site | S · B |
| Sealed-room job | `job_…`, site, Move or assessment, kind, signed spec, state, result, receipt | C1 | CTL | 1 year (`docs/03-trust.md` §5) | S · B |
| Finding | `fnd_…`, detector, severity, category, title, explanation, where, evidence digests | C2 | CTL | 1 year | S · B |
| Evidence | screenshots, HARs, frames, crawled pages, WXR exports | C2; may hold C3 from the old site (a shop's customers) | CAS (C3 sealed) | 1 year | S, K · digests in receipts |
| Legacy archive | encrypted copy of the old site and its database | C3 possible | `agx-<cell>-archive` | Until the owner deletes | S, K · to the owner on request |
| Migration inventory | pages, plugins, products, customers found; old-to-new mapping | C2 | REC | Life | S · B |
| Parity result | page, scores (words, links, products, look), state (pass, retry, flagged) | C2 | REC (today only in `ui_events` and receipts) | Life | S · B |
| Gauntlet run | `run_id`, site, score, out of, attacks, receipt | C1 | CTL | Life | — · B |

### 1.11 Operator and fleet (12)

All C1, in CTL plus the lake, kept for the life of the platform, with no erasure beyond the
lake's subject tombstones: cell; runner pool; release train; ring membership; rollout wave with
canary and control metrics; incident (`INC-0142`, severity, commander, timeline); on-call rota;
site health score (`docs/08-control-plane.md` §4.1); cost line per site
(`cost_to_serve_monthly`); usage reconciliation (gateway versus Vultr `/v1/usage`); the outbox
with its relay cursors. The twelfth, server-sent UI events kept for replay (`ui_events`), is C1
and C2, kept 7 days *h* and never exported.

---

## 2. Built versus needed

### 2.1 What exists: tables

Everything lives in one file, `control.db`, opened by `crates/agentrix-node/src/db/mod.rs:59-73`.
The auth and claim schemas run as raw SQL on every open, before the migrator
(`crates/agentrix-node/src/db/mod.rs:46-55`). The Model Gateway opens the same file to write
`usage` (`crates/agentrix-models/src/meter.rs:13-25`).

| Table | Defined at | Writer | Entity (§1) | Verdict |
|---|---|---|---|---|
| `accounts` | `crates/agentrix-node/src/auth/schema.sql:5` | auth | Account | Change: password hash out (§2.4) |
| `credentials` | `auth/schema.sql:15` | auth | Passkey | Keep; add authenticator model |
| `totp_seeds` | `auth/schema.sql:27` | auth | Authenticator secret | Change: seal in KEYS |
| `sessions` | `auth/schema.sql:35` | auth | Session | Change: add device, last seen |
| `claims` | `crates/agentrix-node/src/claim/schema.sql:4` | claim | Domain claim | Change: FKs, CHECKs |
| `sites` | `crates/agentrix-node/src/db/migrations/0001_control.sql:17` | data (`db/sites.rs`) | Site | Change: many columns (§2.4) |
| `assessments` | `0001_control.sql:29` | coordinator | Assessment | Keep |
| `moves` | `0001_control.sql:41` | move | Move | Keep |
| `move_steps` | `0001_control.sql:54` | move | Move step | Keep |
| `jobs` | `0001_control.sql:71` | coordinator | Sealed-room job | Keep; rename to `room_jobs` in production (WebAgent jobs are a different thing) |
| `artifacts` | `0001_control.sql:90` | data (`db/cas.rs`) | Blob index | Rename to `blobs` (visitor artifacts are §1.6) |
| `findings` | `0001_control.sql:103` | assess, move | Finding | Change: open category list |
| `decisions` | `0001_control.sql:122` | move (`agentrix-move/src/store.rs:671`) | Decision card | Change (§2.4) |
| `quarantine` | `0001_control.sql:144` | move | Quarantined item | Keep; move to REC |
| `releases` | `0001_control.sql:161` | move (`launch/`) | Release | Change: summary, author |
| `site_pointer` | `0001_control.sql:176` | move | Live pointer | Keep; move to REC |
| `charters` | `0001_control.sql:185` | move | Charter | Keep until the kernel's charter store takes over |
| `receipts` | `0001_control.sql:198` | data (`db/receipts.rs:55`) | Receipt | Change: chain, redaction |
| `usage` | `0001_control.sql:218` | gateway (`meter.rs:64`) | Model usage | Change: purposes, WebAgent, job |
| `budgets` | `0001_control.sql:239` | data | AI budget | Change: currency, scopes |
| `briefs` | `0001_control.sql:249` | brief | Brief | Keep; add events counted |
| `gauntlet_runs` | `0001_control.sql:257` | gauntlet | Gauntlet run | Keep |
| `ui_events` | `0001_control.sql:270` | coordinator | UI event | Change: retention job |
| `passed_questions` | `crates/agentrix-node/src/db/migrations/0002_passed_questions.sql:4` | site (`db/questions.rs`) | Passed question | Change: state, gap link; move to REC |

24 tables, one entity each (the password hash rides in `accounts`). The first release also holds some
data outside tables: Ask answers are not stored
at all (only refusals leave a receipt, `crates/agentrix-node/src/site/mod.rs:368`); a held
order from the agent door exists only as a decision card's effect arguments
(`crates/agentrix-node/src/decisions/mod.rs:789-799`).

### 2.2 What exists: types in `agentrix-types`

| Type | Defined at | Covers |
|---|---|---|
| `SiteId`, `MoveId`, `AssessmentId`, `JobId`, `ReceiptId`, `DecisionId`, `FindingId`, `QuarantineId` | `crates/agentrix-types/src/ids.rs:61-100` | Prefixed UUIDv7 ids, sorted by time |
| `Digest` | `crates/agentrix-types/src/digest.rs:14` | `sha256:<hex>` content addresses |
| `Money` | `crates/agentrix-types/src/effect.rs:16` | ISO 4217 code plus minor units |
| `EffectRequest`, fingerprint | `effect.rs:25`, `:44` | The effect seam (ADR-006) |
| `CardKind`, `Risk`, `DecisionCard`, `ApprovalMethod`, `Approval` | `crates/agentrix-types/src/decision.rs:14`, `:25`, `:39`, `:94`, `:105` | Three card kinds; no expiry, approval kind or raiser |
| `Finding`, `Severity`, `Category` | `crates/agentrix-types/src/finding.rs:12`, `:24`, `:37` | Detector output |
| `ReceiptKind` (15 kinds), `Subject`, `ReceiptBody`, `Receipt` | `crates/agentrix-types/src/receipt.rs:15`, `:113`, `:154`, `:307` | Signed receipts; no chain field |
| `Level`, `JobLevel`, `Charter`, `Brief`, `BriefLine`, `GauntletAttack`, `QuarantineEntry` | `crates/agentrix-types/src/records.rs:18`, `:35`, `:52`, `:69`, `:59`, `:82`, `:97` | JSON shapes in `control.db` |
| `JobSpec`, `SignedJobSpec`, `JobResult`, `JobEvent` | `crates/agentrix-types/src/job.rs:123`, `:153`, `:308`, `:207` | Sealed-room jobs |
| `UiEvent` and its events | `crates/agentrix-types/src/ui.rs:347` | SSE for Assess and the Move |

**Missing types** that docs already say exist or must exist:
- `OperatorId`, `PartnerId`, `OrgId`, `TenantPath` (`docs/08-control-plane.md:223` says they are
  in `agentrix-types`; they are not). Contract C1 in 02-architecture.md §6.1, owner W6.
- `WebAgentSpec`, `Mission`, the plan preview, wall counts (`docs/07-owner-console.md:1354-1361`).
- Ids for every new entity: `OrderId`, `CustomerId`, `ThreadId`, `ArtifactId`, `GrantId`,
  `ErasureId` and the rest, each with its prefix.
- A versioned `DecisionCardV2` with `expires_at`, `raised_by` and approval kind. Never change the
  signed `DecisionCard` in place (02-architecture.md §5.3).
- `ReceiptKind` values for the product's own events: `order.*`, `payment.*`, `refund.*`,
  `erasure.*`, `export.*`, `grant.*`, `support.access`, `release.published`, `mission.*`.
- A data-class marker (`C0`–`C4`) for event types, so a lint can refuse C2 or C3 fields in
  boundary events (`docs/09-data-architecture.md:403-405`).

### 2.3 What is missing, by domain

| Domain | Missing tables (owner, §4.3) |
|---|---|
| Platform | operators, partners, orgs, memberships, linked sign-ins, recovery codes, sign-in events, devices (W6); plans, subscriptions, invoices, invoice lines, platform payments, exchange rates, tickets (W3) |
| Site | domains, drafts, brand voice, redirects (W4, W7) |
| Commerce | all 17 entities (W2; payments, refunds, settlements and mandates W6) |
| Content | all 7 (W2) |
| Customers | all 10 (W2; sign-in material and consent receipts W6) |
| Conversations | threads, turns, takeovers, content gaps, agent door calls, agent clients, visitor artifacts, shares, versions (W1, W2) |
| WebAgents | WebAgent, job and dial, missions, mission reports, vector collections, playbook versions (W1); the kernel's process, wake, signal, agenda, ledger and KPI tables come from adk-rust's own migrations once W1 runs a kernel |
| Governance | approvals, standing rules, approved plans, effects, alarms, attention settings (W5, W6) |
| Trust | support grants, break-glass, agency grants, operator ledger, erasure requests, exports, data keys, platform holds and notices (W3, W6) |
| Move | previews, growth leads, migration inventory, parity results (W4) |
| Cross-cutting | the outbox (`docs/09-data-architecture.md:420-426`), the data catalogue `data/catalog/*.toml` (`:988-997`), per-site store directories (W8) |

### 2.4 Changes to existing tables

| Table | Problem | Change | Owner |
|---|---|---|---|
| `accounts` | `password_hash` holds Argon2id in PHC form, unsealed (`auth/schema.sql:9-10`), against ADR-025 and `docs/09-data-architecture.md:156` | New `password_secrets` table with a sealed blob that opens only in KEYS; stop writing the column; drop it after one release train | W6 |
| `totp_seeds` | Sealed with the node's own key (`auth/schema.sql:25`), not by `agentrix-keys` | Re-seal under the global sign-in key held by KEYS (A8) | W6 |
| auth tables | Times are Unix seconds (`auth/schema.sql:3`); every other table uses RFC 3339 text (`0001_control.sql:4-5`) | Pick one. New tables use RFC 3339; keep auth as is until the identity store moves | W6 |
| `sessions` | No device, last seen or place; the security screens need them | Add nullable columns; add `sign_in_events` | W6 |
| `claims` | No foreign keys and no CHECK on `state` or `method` (`claim/schema.sql:4-16`) | Add them when the file becomes a real migration | W4 |
| `sites` | No org, plan, cell, residency tag, archetype, tier, currency, languages or lifecycle state. `utc_offset_minutes` (`0001_control.sql:22`) cannot drive quiet hours across daylight-saving changes; 05 §7 needs an IANA zone. `fixture` (`:24`) cannot tell a gauntlet fixture from a sample site | Add nullable columns; add `time_zone`; add `origin` (`real`, `sample`, `fixture`) and keep `fixture` until readers move | W3 |
| `decisions` | `kind` allows three values (`0001_control.sql:125`), so no refund, price, campaign, promotion, grant or support card fits. No `expires_at`, raiser or approval kind. The card body embeds the customer's name and email (`crates/agentrix-node/src/decisions/mod.rs:797-799`), which is C3 in the shared database. The idempotency lookup scans JSON with no index (`decisions/mod.rs:767-769`) | Drop the CHECK in a new table version; add columns; reference the order or customer by id; add an `idempotency_key` column with a unique index per site | W5 (card store, 02-architecture.md:327) |
| `receipts` | No previous digest or sequence, so there is no hash chain; the concepts show "chain predecessor" and "412 entries since go-live" (`concept/site.html:339`). A `concierge.refusal` receipt carries a 200-character excerpt of the visitor's words (`crates/agentrix-node/src/site/mod.rs:368`), which is C3 | Add `seq` and `prev_digest` per site, or move receipts onto the site log (`AuditLogStore`). Add a redaction state so erasure can redact `body` and keep `digest` | W6 |
| `usage` | `purpose` allows nine values (`0001_control.sql:221-222`); 05 §3.7 adds `webagent.act`, `webagent.write`, `webagent.summarise`, `plan` and `image`. `provider` allows two. No WebAgent, job or process, so "today KSh per process" (`concept/console/shell.html:260-271`) has no source. No reserved amount, only settled | Replace the CHECKs with a reference table; add `webagent`, `job`, `process_id`, `reserved_micro_usd` | W8 (or W3) |
| `budgets` | Micro-USD only, one row per purpose (`0001_control.sql:239-245`); owners see KSh (`concept/console/shell.html:51`) and budgets per WebAgent and mission | Add `scope` and `display_currency`; convert with the site's posted rate; keep micro-USD as the stored unit | W3 |
| `releases` | No summary or author; the console lists "v14 Sunday hours" | Add `summary`, `author_kind`, `author` | W4 |
| `artifacts` | The name clashes with visitor artifacts (M5). Blobs are not sealed (`crates/agentrix-node/src/db/cas.rs:1-8`) and one on-disk store serves every site | Rename to `blobs`; add `class` and `sealed`; per-site prefix in production | W8 |
| `findings` | `category` allows four values (`0001_control.sql:110`) | Open the list for new detectors | W4 |
| `passed_questions` | No state, answer or link to a gap; `contact` is C3 in the shared database (`0002_passed_questions.sql:9`) | Add `state`, `answered_at`, `gap_id`; move to REC | W1 |
| `ui_events` | Nothing ever deletes rows | A retention job, 7 days *h* | W5 |
| all | Site data and personal data share one file with control data. Erasure has no adapter for any table | Split per §3; register a `SubjectDataStore` for every table that holds C3 (ADR-028) | W8, W6 |

---

## 3. Stores per profile

### 3.1 Single node (self-host, development, the first release's successor)

`docs/09-data-architecture.md:264-283` is the target layout: `control.db` for control data, and a
directory per site holding `kernel.db`, `kernel/sessions.db`, `edge.db`, `wallet.db`,
`records.db`, `log.db`, `vectors/` (LanceDB) and the release host, plus the CAS mirrored to
Object Storage. 02-architecture.md:357 proposes splitting the global file further by owning
crate (`identity.db`, …). That works for SQLite, with one caveat: a decision card is site data,
so it belongs in the site's directory, not in a global `decisions.db`.

What it needs that the first release lacks: the per-site directories; WAL shipping or snapshots to Object
Storage (RPO 15 minutes *h*); `agentrix-keys` as its own process; LanceDB with `purge_deleted` on
erasure; the outbox table in `records.db`.

### 3.2 Fleet (per cell)

As `docs/09-data-architecture.md:299-372`: Managed PostgreSQL 17 or 18 on Business or higher
(PITR 14 days), a schema and role per site, pgvector (HNSW over `halfvec`) beside the records,
`platform.outbox` with RLS on `site_id`, Object Storage buckets per class with one service user
each, Managed Valkey for fast state only, Managed Kafka for content-free events, Iceberg over
Parquet per cell, DataFusion in services and DuckDB for analysts.

### 3.3 What production v1 should use

The pilot is ten sites and the first year is hundreds, not thousands. Each moving part costs
operations time. This is the smallest set that keeps every promise in 03 §5 and ADR-025 to 030.

| Need | Production v1 | Later (second cell, or about 2,000 sites per cell) |
|---|---|---|
| Records | One Managed PostgreSQL cluster in `ams-1` (Business, PG 17 or 18). Database `cell` with a schema and role per site; database `control` beside it for the global layer; read replica in `fra-1` | Separate control cluster; more cells |
| Vectors | pgvector in the site schema, HNSW on `halfvec`, hybrid with `tsvector`, rerank through the gateway (needs D1–D3, D13). Vultr's vector store only for the first release's demo (C0) | Qdrant for a site that outgrows pgvector (ADR-026) |
| Objects | Object Storage in Amsterdam: releases, CAS, archive, exports, lake buckets. The platform's copier to Milan with copy receipts | Archive tier for legacy archives |
| Fast state | Managed Valkey (smallest plan), because production runs the edge on two hosts and rate limits and presence must be shared | Per-cell sizing |
| Events | The outbox in each site schema's transaction; the relay writes date-partitioned Parquet straight to the lake bucket. **No Kafka** | Managed Kafka when a second cell or a second consumer group appears (`docs/09-data-architecture.md:700-704`) |
| Lake | Parquet read by DuckDB and by DataFusion inside `agentrix-analytics`. **No Iceberg** yet; erasure rewrites the affected day partitions | Iceberg metadata over the same files, equality deletes, compaction |
| Keys | `agentrix-keys` as its own service (a small Rust service or OpenBao Transit), one KEK for the cell under two-person control | KEK per cell |
| Telemetry | OTel collector to VictoriaMetrics and VictoriaLogs with Grafana (the permissive option in `docs/09-data-architecture.md:692-696`); James decides | — |
| Self-host | The single-node layout of §3.1 | SQLRight instead of LanceDB if it passes the pilot (`docs/09-data-architecture.md:502-505`) |

Two pilot measurements decide the fleet's shape: migration time for one schema of about 60
tables, and planner overhead with many schemas (`docs/09-data-architecture.md:316-318`).

---

## 4. Migrations for parallel work

### 4.1 Two rules exist today, and they disagree

- **how-we-work.md:60-69**: one migrator in `agentrix-node`, ranges per workstream (0100–0199 W1 …
  0600–0699 W6; the first release keeps 0003–0099), strictly additive. W7 and W8 have no range.
- **02-architecture.md:357**: one database file and one migrator per owning crate, UTC timestamp
  versions, the first release's `control.db` frozen at 0002, and a one-off importer.

Both are right for different times. sqlx 0.9 (the pinned version, `Cargo.lock`) applies every
unapplied version whatever its order (`sqlx-core-0.9.0/src/migrate/migrator.rs:262-284`), refuses
a changed checksum, refuses an applied version missing from the source unless
`set_ignore_missing` is on, and lets a migrator use its own table name
(`Migrator::dangerous_set_table_name`, `migrator.rs:111`). So ranges work in one shared file
today, and per-crate sets can share one Postgres schema later.

### 4.2 The rule

**Phase A: now, until W8's store layer lands.** New tables that must live in today's
`control.db` follow how-we-work.md's ranges, extended:

| Range | Owner |
|---|---|
| 0001–0099 | The first release (frozen after the live run, 9 Nov) |
| 0100–0199 | W1 WebAgent runtime |
| 0200–0299 | W2 Sample websites |
| 0300–0399 | W3 Control plane |
| 0400–0499 | W4 Onboarding and owner journey |
| 0500–0599 | W5 Owner console |
| 0600–0699 | W6 Identity, payments, trust |
| 0700–0799 | W7 Design system and voice (new) |
| 0800–0899 | W8 Platform core (new; 02-architecture.md:330) |
| 0900–0999 | W0 Integration: cross-workstream links and contract steps (new) |

File names carry the owner: `0213_w2_orders.sql`. Additive only. Prefer new crates' own files
over `control.db`: anything that is site data and can wait for the store layer should wait.

**Phase B: production.** One migration set per **crate and store**:

```text
crates/<crate>/migrations/<store>/<dialect>/<UTC yyyymmddhhmm>_<noun>.sql
  store   = control | site
  dialect = sqlite | postgres
```

- **Namespace by crate.** Each crate embeds its own migrator per store and names its own
  migrations table, `_sqlx_migrations_<crate>` (for example `_sqlx_migrations_records`). Crates
  never share a version list, so two workstreams cannot collide.
- **Timestamps inside a crate.** Two sessions on one crate pick UTC minutes, which do not
  collide in practice. If they do, the later merge renames its file before it reaches `main`.
- **Both dialects or neither.** CI fails if a version exists in one dialect only, and runs both.
- **The `site` store** runs per site: against each `records.db` on a single node, and per schema
  in the fleet, ring by ring (`docs/09-data-architecture.md:1019`). A kernel refuses a schema
  older than it supports.
- **Expand, migrate, contract** (`docs/09-data-architecture.md:1020`): add nullable columns and new
  tables; stop using a column in one release; drop it in a later release, through W0.
- **No foreign keys across workstreams.** A row may reference another workstream's row by its
  typed id, checked in code. Foreign keys are allowed inside a crate and to `sites`. This keeps
  any migration order valid on a fresh database.
- **Never edit or renumber a merged migration** (sqlx checksums it).
- **The raw schema files go.** `auth/schema.sql` and `claim/schema.sql` become versioned
  migrations in their new crates (`agentrix-identity`, `agentrix-onboarding`), written with
  `IF NOT EXISTS` so existing databases accept them.
- **The first release's `control.db`** is frozen. Moving its rows into the new layout is a one-off importer,
  or a fresh start (02-architecture.md §9 item 4, James decides).

**CI checks for both phases:**
1. Version unique within its set; in Phase A, inside its owner's range.
2. Every table has a primary key (Vultr enforces it, `docs/09-data-architecture.md:354-355`).
3. Every `site` table has `site_id NOT NULL` (defence in depth, `0001_control.sql:7`).
4. Every table has an entry in `data/catalog/<table>.toml` with owner workstream, class,
   retention, erasure method and export format (`docs/09-data-architecture.md:988-997`).
5. Every C2 or C3 table has a registered `SubjectDataStore` or a site-drop method (ADR-028).
6. No migration creates or alters a table its workstream does not own.
7. A fresh database migrates in any merge order: CI applies each set alone, then all sets.

### 4.3 Who owns which tables

Crate names are 02-architecture.md §5.1's.

| Workstream | Crate(s) | Store | Tables |
|---|---|---|---|
| **W1 WebAgent runtime** | `agentrix-kernel` | site | webagents, webagent_jobs (dial, ceiling, lowered), missions, mission_reports, briefs, playbook_versions, vector_collections, threads, turns, takeovers, passed_questions, content_gaps, desk_manifests (draft side); plus adk-rust's own kernel, session, harness and memory migrations, which W1 runs and does not edit |
| **W2 Sample websites** | `agentrix-records`, `agentrix-edge` | site | products, configurable_products, services, price_lists, price_lines, stock_levels, delivery_zones, delivery_slots, baskets, quotes, orders, order_lines, order_events, holds, bookings, posts, authors, corrections, story_timelines, media, digest_subscriptions, digest_editions, customers, addresses, customer_lists, list_members, visitor_memory_items, visitor_artifacts, artifact_shares, artifact_versions, agent_door_calls, agent_clients, presentation_scripts |
| **W3 Control plane** | `agentrix-operator`, `agentrix-fleet`, `agentrix-billing` | control | sites (registry columns), domains, operators, partners, orgs, plans, subscriptions, invoices, invoice_lines, exchange_rates, tickets, budgets, cells, runner_pools, trains, rings, rollouts, incidents, health_scores, reconciliations, partner_payouts, platform_holds, platform_notices, operator_ledger |
| **W4 Onboarding and owner journey** | `agentrix-onboarding`, `agentrix-move`, `agentrix-releases` | control (Move), site (releases) | assessments, previews, growth_leads, claims, moves, move_steps, room_jobs, findings, quarantine, migration_inventory, parity_results, gauntlet_runs, releases, site_pointer, redirects, drafts index |
| **W5 Owner console** | `agentrix-console`, `agentrix-decisions` | site | decisions (card store, 02-architecture.md:327), approved_plans, attention_settings, snoozes, ui_events, notification_deliveries |
| **W6 Identity, payments, trust** | `agentrix-identity`, `agentrix-effects`, `agentrix-keys` | control, site | accounts, memberships, credentials, password_secrets, totp_seeds, recovery_codes, linked_identities, sessions, sign_in_events, devices; visitor_accounts, visitor_credentials, visitor_sessions, consent_receipts (site); approvals, standing_rules, effects, receipts or the site log, customer_payments, refunds, settlements, ap2_mandates, platform_payments; support_grants, break_glass, agency_grants, erasure_requests, erasure_answers, exports; data_keys and unwrap_counts (inside `agentrix-keys`) |
| **W7 Design system and voice** | `agentrix-ui` | site | brand_voice, webagent_marks (seed and colour, ADR-031), words_catalogue overrides per site (Swahili), voice_settings |
| **W8 Platform core** | `agentrix-store`, `agentrix-models` | both | blobs (today `artifacts`), outbox, relay_cursors, usage, lake catalog; the per-site directory and schema creation |

If James does not adopt W8 (02-architecture.md:507), its tables go to W3.

### 4.4 Tables built so far (the catalogue, kept by W0)

What is on the `platform` branch, with its single writer. Every new table gets a row here when
its slice merges. Where this differs from §4.3, this table is what was built.

| Table | Writer | Migration | Store | Notes |
|---|---|---|---|---|
| webagent_sites | W1 `agentrix-kernel` | `crates/agentrix-kernel/migrations/0100_webagents.sql` (own migrator, `_sqlx_migrations_agentrix_kernel`) | control.db | Which roster a site runs |
| webagent_specs | W1 `agentrix-kernel` | same | control.db | The nine WebAgents' specs and MarkSpecs per site |
| webagent_dial_changes | W1 `agentrix-kernel` | `0101`, `0102` (same migrator) | control.db | Every dial lowered or raised; raises carry the approval's fingerprint and receipt |
| operator_roles | W3 `agentrix-operator` | `crates/agentrix-node/src/db/migrations/0300_operator.sql` | control.db | From the roles file |
| operator_sessions | W3 | same | control.db | Own 12 h sessions, `__Host-agx_ops` |
| operator_ledger | W3 | same | control.db | Hash-chained, verified on every page |
| support_grants | W3 | same | control.db | Owner-approved; planned under W6 in §4.3, built by W3 |
| support_access | W3 | same | control.db | Each use of a grant, receipted to the owner |
| break_glass | W3 | same | control.db | Planned under W6 in §4.3, built by W3 |
| platform_holds | W3 | same | control.db | Recorded and receipted; the edge enforces them after the handover |
| visitor_accounts | W6 `agentrix-identity` | `crates/agentrix-identity/migrations/0600_w6_visitor_accounts.sql` (own migrator, `_sqlx_migrations_identity`) | control.db | Customer accounts per site host |
| visitor_credentials | W6 | same | control.db | Passkeys per site (RP id = the site's host) |
| visitor_recovery_codes | W6 | same | control.db | Hashed, shown once, single use |
| visitor_sessions | W6 | same | control.db | Never cross sites |
| visitor_sign_in_events | W6 | same | control.db | New-device alerts |
| consent_receipts | W6 | same | control.db | Each also issues `consent.given` or `consent.withdrawn` |
| media_images | W11 `agentrix-media` | `crates/agentrix-node/src/db/migrations/1200_media.sql` | control.db | Every picture a site has (made, uploaded, moved), by digest in the content store; drafts until the owner approves; blocked attempts keep no bytes |
| voice_notes | W11 `agentrix-media` | `crates/agentrix-node/src/db/migrations/1201_voice_notes.sql` | control.db | What the owner said to the Chief and what it did; never the recording |
| owner_recovery_codes, owner_recovery_holds, owner_code_sign_ins | W6 | `crates/agentrix-identity/migrations/0601_w6_owner_recovery.sql` | control.db | Codes hashed and single use; holds last 24 hours; every code sign-in receipted |
| shell_quiet_hours, shell_freezes, shell_visits | W5 `agentrix-shell` | `crates/agentrix-shell/migrations/0500_owner_console.sql` (own migrator) | control.db | Night Time hours, Freeze state, what the owner last saw |
| visit_counts | W8 `agentrix-store` | `crates/agentrix-store/migrations/control/0800_visit_counts.sql` (own migrator) | control.db | Daily totals by route, referrer host, device class, country; fewer than 5 shown as "fewer than 5" |
| usage (rebuilt), usage_purposes | W8 | `0801_usage_open.sql` | control.db | The first release's `usage` rebuilt with the same columns plus units for images and audio; the old binary still writes it |
| health_hours | W8 | `0802_health_hours.sql` | control.db | The status page's hourly checks |
| message_outbox | W9 `agentrix-messages` | `crates/agentrix-node/src/db/migrations/1000_message_outbox.sql` | control.db | Every message before it is sent; codes never stored |
| rec_* (27 tables: sites, products, stock, bake plan, price lists and rules, zones, slots, customers, carts, orders, order lines and events, payments, refunds, holds, bookings, questions, counters, authors, stories, corrections, timeline, subscribers, digest subscriptions, sessions, codes) | W2 `agentrix-records` | `crates/agentrix-records/migrations/0200_records.sql` (own migrator) | `data_dir/agentrix-samples/records.db` | The three sample sites' records, loaded from bundles marked Sample |
| rec_feedback; rec_questions.answered_at | W2 `agentrix-records` | `0252_feedback.sql` (same migrator) | `records.db` | "Was this right?" on the samples' desk answers, one row per answer (desk, question, the words when known, yes or no, a note on a No), wiped with the sample on reload; when the owner taught an answer, which the desks then give for the same question |
| channel_link_codes, channel_links, channel_cursors, channel_alerts | W13 `agentrix-channels` | `crates/agentrix-channels/migrations/1300_channel_links.sql` (own migrator, `_sqlx_migrations_agentrix_channels`) | control.db | One-time link codes kept only as SHA-256, single use, 15 minutes; chats linked to an owner's site; each update and each alert handled once; link and unlink receipted as `channel.changed` |
| channel_turns | W13 | `1301_channel_turns.sql` | control.db | Chat turns (question cut to 200 characters, the answer, message ids), kept 30 days; `/forget` deletes a chat's turns |
| channel_devices | W13 | `1302_channel_devices.sql` | control.db | Phones paired for the owner app: token kept only as SHA-256, shown once; removed in Settings → Channels or after 30 days unused; a token never approves on its own |
| decision_rules | W5 `agentrix-decisions` (built on `w0/decision-cards`) | `crates/agentrix-decisions/migrations/0551_owner_edits_and_rules.sql` (own migrator, `_sqlx_migrations_agentrix_decisions`) | control.db | Standing rules the owner made while declining a card, in their words, for the WebAgent and job that raised it. Removing one keeps the row with its receipt. The same migration adds `editable`, `owner_edit` and `decline_reason` (nullable) to `decision_cards_v2` |
| decision_grants | W5 `agentrix-decisions` | `0552_standing_grants.sql` (same migrator) | control.db | "Allow, and tell me": a grant signed with the approval, capped at the card's own amount, 90 days, stoppable; `uses` counts cards approved under it. `terms.person` is what the card's effect named (C3 when a raiser names a person rather than an id), so erasure must cover it. Adds `grant_id` to `decision_cards_v2` |
| subscriptions, subscription_events, invoices, invoice_lines, payments, invoice_runs | W3 `agentrix-billing` | `0310_billing.sql` (next merge) | control.db | Plans in USD; invoice runs approved by op.finance |

---

## 5. Seed data for three sample websites

### 5.1 Which three, and why

- **Kilele Bakery** (local services). The most-drawn site: about 35 concept files, the fixture
  (`fixtures/wordpress-infected/README.md:3`), the journeys. Fictional, on `.example`.
- **A perfume shop** (WooCommerce). The search, artifact and gift-plan concepts
  (`concept/experience/search.html`, `artifact.html`) exercise catalogue, stock, comparisons and
  shared artifacts. **Not under the name Classy Perfumes:** classyperfumes.shop is a real
  Zavora-run site in ring 0 (`docs/08-control-plane.md:411`, `:1523`) and a pilot site
  (`docs/01-product.md:274`). Fictional stock and orders under a real shop's name would read as
  real. The sample takes the concept's catalogue under a fictional name and `.example` domain;
  W2 checks the name against real businesses and James approves it.
- **The Kestrel Post** (national news). Fictional (`docs/06-website-experience.md:1380`). It is
  the only archetype with volume, entitlements, corrections and digests, so it tests retrieval
  and erasure paths the other two never touch.

02-architecture.md:297 assumes a company site as the third. zavora.ai is real, so it cannot be a
sample, and its pages (tours, quote ranges) need little the other two do not already test. This
is James's call (02-architecture.md:508).

### 5.2 Rules for every seed

1. **A seed is an export bundle** (`docs/09-data-architecture.md:910-917`) under
   `fixtures/samples/<slug>/`: `records/*.jsonl`, `site/` source, `manifest.json`. Loading a seed
   is importing a bundle, so the import path is tested every time.
2. **Marked at the source.** `sites.origin = 'sample'`. Hosts end in `-sample`
   (how-we-work.md:78). Every page shows "Sample business · fictional data"; every desk's first
   answer says so; the console shows a Sample chip on every screen of that site; every document
   artifact carries it in its footer.
3. **No real effect, ever.** The Effects Gateway refuses any effect for a sample site except to
   sandbox rails (PayPal sandbox, Daraja sandbox) and a mail sink. Contact details use reserved
   values: emails at `example.com` or the site's `.example` domain; UK numbers from Ofcom's
   drama range (`+44 7700 900…`, as the journeys already use); Kenyan numbers stored masked and
   flagged not sendable.
4. **Numbers are computed, never typed in.** "23 of 60 preorders" is 23 order rows. "41 visitors"
   is 41 beacon counts. A seed holds rows; the metric catalogue computes the figures.
5. **Dates are relative.** Seeds store offsets from an anchor (Sat 7 Nov 2026, 09:42 EAT) and the
   loader shifts them to load time, so "today's bakes" and "Sat 14 Nov" stay current.
6. **Out of fleet numbers.** The lake tags `dim_site.origin`; revenue, health and fleet views
   exclude samples; their model spend is billed to the operator tenant.
7. **One fact, one value** (`specs/platform/01b-ui-owner-and-operator.md:1437-1452`). Where the
   concepts disagree, the seed picks one value below and the concepts follow it.

### 5.3 Kilele Bakery (`kilele-bakery-sample`, KES)

**Site:** Kilele Bakery, Westlands, Nairobi; delivers to Westlands and Kilimani. Owner Wanjiru
Kamau (fictional), `wanjiru@kilele-bakery.example`. `Africa/Nairobi`, KES, English and Swahili.
Plan Commerce (the Cashier and payments need it). Tier T3. Hours Mon–Sat 06:30–19:00, Sun
07:00–14:00. First release v1 at go-live; the live release in the seed is v14.

**Catalogue** (KSh; from `concept/site.html:219-222`, the fixture menu
`fixtures/wordpress-infected/site/seed.php:132`, and `concept/journeys/customer-browse.html:112`):

| Item | Price | Stock in the seed |
|---|---|---|
| Country sourdough | 450 | 6 left |
| Rye and caraway | 500 | 10 |
| Butter croissant | 180 | next batch 10:00 |
| Cardamom bun | 150 | 14 |
| Mandazi, six | 200 | 8 |
| Banana bread slice | 220 | 9 |
| Cinnamon roll | 220 | sold out until 06:30 tomorrow |
| Red velvet slice | 350 | 12 left |
| Weekend bake box (a sourdough loaf, four cinnamon buns, a banana loaf) | 1,350 | 5 |

**Celebration cake** as a configurable product, from `concept/experience/creator.html:229-240`:
1 tier (serves 12–15, 2 days' notice) 3,800; 2 tiers (25–30, 2 days) 4,500; 3 tiers (40–45,
3 days) 6,900. Sponge vanilla or lemon +0, chocolate +200, red velvet +300, gluten-free +600
(1–2 tiers only). Finish smooth or naked +0, ombré +250, drip +400. Decoration berries +500,
flowers +700, sprinkles +150. Message free, 22 characters. Every line has its rule code
(`size.2`, `finish.ombre`, `deco.berries`, `delivery.zone1`). Price list v14, valid until Sat
14 Nov 08:00.

**Service:** Saturday baking class, KSh 4,500, eight seats, lunch included
(`fixtures/wordpress-infected/site/seed.php:135`).

**Delivery and collection:** delivery to Westlands and Kilimani KSh 300, Saturday windows
10:00–12:00 and 12:00–14:00. Collection slots on Sat 19 Dec: 12, with 11 taken; the mission's
slot card proposes 16 (`concept/console/mission.html`). "Karen costs KSh 500" is an `Assumed`
fact, never published (`concept/console/trust.html:182`).

**Customers** (fictional): Njeri (London, `njeri@example.com`, `+44 7700 900418`, pays in USD by
PayPal); Amani W.; Mary K.; Peter O.; Riverside Co. (a business); 31 "Sample customer" rows with
consent receipts for offers, forming list `l-xmas25`.

**Orders and money** (one scheme, `KB-` plus four digits; rate 1 USD = KSh 129.00,
`docs/06-website-experience.md:976-977`):
- KB-1042, Njeri: 2 sourdough, bake box, delivery to Kilimani. KSh 2,550 = USD 19.77, paid,
  delivered Sat 14 Nov.
- KB-1043, Njeri for Amina: 2-tier lemon sponge, forest ombré, berries, "No nuts". KSh 5,250 =
  USD 40.70. Held, approved by passkey, paid.
- KB-1044, Amani W.: 2-tier vanilla, "Happy 30th Amani", KSh 4,500, M-Pesa, collect Sat 14 Nov
  10:00. Waiting for Wanjiru (the console's order card).
- Refund R-1042-1: the bake box, KSh 1,350 = USD 10.47, under the Cashier's rule (up to KSh 2,000
  an order, five a week, until 31 Dec).
- Refund to Peter O., KSh 800, under the same rule.
- 23 Christmas preorders (fruit cakes, collection 19–23 Dec), so the mission reads 23 of 60.
- Hold H-0193: two sourdough until 12:00, inside standing rule g-2.

**Leads and quotes:** Mary K., wedding cake for 120 guests on 9 Jan (quotes above KSh 10,000 need
a 30% deposit; she asked for gluten-free 3 tiers, which the rules refuse honestly); Riverside
Co., 40 breakfast boxes (quote only, no list price); one birthday request. These three
unanswered quotes are what the Sales listing's trigger counts (`concept/console/store.html:166`).

**Booking:** one seat on the next baking class.

**Content:** "Our sourdough starter turns five" (fixture), "We moved! Same cakes, faster site",
"This weekend: 2-for-1 croissants before 9 am", "Christmas cake orders open".

**WebAgents:** the roster of `docs/05-webagents.md:76-84` at their charter v1 levels; mission
`m-3` "Christmas cakes": 60 preorders by 18 Dec, budget KSh 400, 26 Oct–24 Dec; standing rules
g-2 (holds) and g-3 (refunds up to KSh 3,000). Facts: hours and delivery fee `Known`; Karen
delivery `Assumed`. One thread (`t-2291`, Mary K., handed to Wanjiru) and enough Ask turns to
produce the overnight brief from rows.

**The pending price change:** draft v15 changes two cake prices that differ from Wanjiru's
WhatsApp list (`concept/console/live.html:168-170`). In the seed these are the Editor's draft,
not the live list, so the creator's prices above stay the published truth.

### 5.4 The perfume shop (`<name>-sample`, KES)

**Site:** a fictional perfume shop in Westlands, Nairobi. Name chosen by W2 and approved by James
(§5.1). KES, English. Plan Commerce. Tier T3 (ceiling T4). Live release in the seed: v31.

**Catalogue** (from `concept/experience/search.html:229-234`; fields: name, size, price, family,
notes, vanilla score 0–3, gift-ready, stock):

| Item | Size | Price | Family | Stock |
|---|---|---|---|---|
| Vanilla Gift Set (candle and 30 ml) | 30 ml | 4,500 | Gourmand; gift-ready | 7 |
| Citrus Dawn | 50 ml | 4,900 | Fresh citrus | 3 |
| Amber Noir | 50 ml | 6,400 | Amber | 14 |
| Rain Oud | 30 ml | 5,800 | Woody; limited run | 9 |

Search needs more than four items to rank, compare and say "over budget by KSh 1,400". W2 adds
eight more fictional scents across the same families (4,000–9,000), each name checked against
real products, plus 2 ml samples at no charge.

**Policies:** Nairobi same day if ordered before 14:00, KSh 300; Mombasa 2–3 working days, KSh 650;
14-day returns on unopened bottles; free gift wrap with a handwritten card
(`concept/experience/search.html:235-240`).

**Customers and artifacts:** Achieng (a bride, fictional) and her gift plan `plan-k29x` for five
bridesmaids, budget KSh 25,000, deliver by Fri 4 Dec, link expires 7 Dec
(`concept/experience/artifact.html:187-192`, `:287-293`). The five invitees (Wambui, Njeri,
Atieno, Muthoni, Zawadi) are artifact shares, not customers. 20 more sample customers with past
orders.

**Orders:** the gift-plan request (five items, gift wrap, total computed from the chosen rows),
held for the owner; 15 past orders across Nairobi and Mombasa; one refund of KSh 2,300 waiting
(`concept/console/shell.html:332`). Stock "as of 11:02".

**Content:** journal posts "How amber wears in cold air" and "Scents for the long rains".

**Desks:** Scents, Gifts, Orders, Journal. AI budget KSh 3,000 a month.

**Names:** Achieng, Njeri and Muthoni also name people in other concepts (an SRE, a Kilele
customer, an agency lead). W2 keeps one person per name across all samples.

### 5.5 The Kestrel Post (`kestrel-post-sample`, KES)

**Site:** a fictional national newspaper, `kestrelpost.example`. KES (the concept quotes a
KSh 41bn rail budget). English. Tier T1 (ceiling T3). Desks: Politics, Business, Climate,
Transport, Sport, Explainers (`concept/experience/news.html:201`).

**Stories** (from `concept/experience/news.html:130-149`):
- "Council approves first phase of the northern rail extension", Amani Otieno, Thu 5 Nov 11:04;
  vote 31–12; corrected 16:20 ("KSh 41bn, not 48bn"); timeline: proposal Mon 2 Nov, tenders open
  in January.
- "Long rains arrive early across the highlands", Lea Mwangi, Fri 6 Nov.
- "What the fuel price review means for fares", Daniel Kiprop, Wed 4 Nov, subscriber-only.
- "Residents split over the northern line's route", Lea Mwangi, Wed 4 Nov.

**Archive:** about 300 generated stories over 90 days across the six desks, from templates in the
manner of the fixture's journal (`fixtures/wordpress-infected/site/seed.php:139-150`), so recency
ranking, "12 stories this week" on Transport and the subscriber entitlement index have real
rows. Every generated story is plainly marked sample text.

**Readers:** 40 visitor accounts, 12 of them subscribers. The subscription price is not in any
concept; W2 sets it behind a named placeholder and asks James (how-we-work.md:142-144). 25 digest
subscriptions to "Friday five" (Friday 07:00, keeps email and topics only).

**Corrections and entitlements:** at least one correction per desk; 15% of stories
subscriber-only, so headline-only answers for non-subscribers are tested.

---

## 6. Tenancy, residency, erasure, export, backups and DR

| Area | Production must | Gap today |
|---|---|---|
| **Tenancy** | A store set per site: a directory on a single node, a schema and role in the fleet; RLS on the tenant path in control (`docs/09-data-architecture.md:811-823`); capability tokens scoped to one site; the cross-tenant suite with 08 §2.7's three cases; one CAS prefix and one data key per site | One `control.db` for every site and lane. The site wall is a column (`0001_control.sql:7`). No org, partner, operator or tenant path type. C3 sits in the shared file: the customer's name and email in `decisions.card` (`decisions/mod.rs:797-799`), `passed_questions.contact` (`0002_passed_questions.sql:9`), refusal excerpts in receipts (`site/mod.rs:368`). The CAS is one directory for all sites; blobs are deduplicated across sites and not sealed (`db/cas.rs:1-8`). The Model Gateway trusts an `x-agentrix-site` header (`crates/agentrix-models/src/gateway.rs:235`) |
| **Residency** | A residency tag per site that placement honours; records, vectors, objects and the site log in the home cell; backups in the copy region; the DPA names the US for inference (A9); African sites' stores in the EU until `jnb-1` has Object Storage (`docs/09-data-architecture.md:936-965`) | No residency column. The first release runs in `atl-0` (US), so any pilot owner's data sits in the US. Inference runs in the US for everyone. Kenya's Data Protection Act needs ODPC registration and a basis for cross-border transfer (`docs/08-control-plane.md` §4.6) |
| **Erasure** | ADR-028's orchestrator on adk-core `SubjectRights`, one per site; an adapter for every C3 table; per-store receipts (`Hard`, `Eventual`, `External`); deadlines (visitor memory 1 hour, other requests 30 days); payment records pseudonymised; receipts redacted, hashes kept; the erasure ledger replayed after every restore; site closure drops the schema or directory and destroys the data key | None of it exists. No adapter, no ledger, no request table. Deleting an owner account fails, because `decisions.signer` and `charters.signed_by` reference `accounts` with no `ON DELETE` rule (`0001_control.sql:131`, `:190`); they need pseudonymising. Credentials and sessions do cascade (`auth/schema.sql:17`, `:28`, `:37`). Receipts cannot be redacted (`body` is the signed JSON). `ui_events` and `passed_questions` have no retention |
| **Export** | The signed bundle: `records/` (JSON Lines and Parquet, CSV for leads, orders, bookings), `site/` (the static release), `receipts/` (verifiable with `agentrix audit verify --site`), `analytics/`, `manifest.json`; no vectors; no password hashes or TOTP seeds; a person's own export; re-import elsewhere | No export of any kind. `agentrix audit verify` has no site log to check |
| **Backups and DR** | Single node: WAL shipping to Object Storage, RPO 15 minutes. Fleet: PITR 14 days, nightly per-site logical exports sealed with the site key to Milan, the copier with copy receipts, release pointer switches only after copies exist, monthly restore drill that verifies the receipt chain and erasure replay, quarterly failover to `fra-1`, KEK backups under two-person control (`docs/08-control-plane.md:607-640`, `docs/09-data-architecture.md:971-984`) | `control.db` has no backup: nothing in `deploy/` copies it, and `deploy/provision.sh:12` destroys the VMs. The only off-host copy is the CAS mirror to Object Storage. Control's receipt-signing key and the site's release-log key (`{domain}/.well-known/awp/log`, `specs/first-release/design.md` §4.5) are files on the control VM; losing them ends the receipt chain and the site log. No drill has run |
| **Keys** | `agentrix-keys` holds KEKs and sign-in keys and never releases them (ADR-030, A8) | No key holder. Every key is a file on the control VM (02-architecture.md:26-27) |

---

## 7. Data the UI shows that nothing produces yet

Each row is a number or list drawn on a concept page that no table, receipt or event produces
today. Line numbers are approximate (within two lines) for pages not checked by hand here.

### 7.1 Owner console

| UI shows | Where | Missing source | Owner |
|---|---|---|---|
| "41 visitors", "3 live", visitors per room | `concept/console/shell.html:46`, `:83`, `:167-188` | Edge visit counts (`adk_awp_beacon`, `agx.edge.v1`); live presence in Valkey | W2, W8 |
| "9 questions, 2 in Swahili, none corrected"; answers per desk | `shell.html:83`; desk stats `:180-188` | Threads and turns with language and desk; owner corrections | W1 |
| AI budget "KSh 38", "KSh 9 overnight", "KSh 38 of 1,500" | `shell.html:51`; `concept/console/phone.html:115` | Budget in the site's currency and a posted rate; `usage` is micro-USD only | W3, W8 |
| Dock states ("asleep until 02:00"), badges | `shell.html:59-65` | WebAgent and process tables, next wake, presence | W1 |
| Activity monitor: woke because, next, today KSh per process | `shell.html:260-271` | Wake reasons; usage per WebAgent, job and process | W1, W8 |
| Mission card 23/60, +9 this week, KSh 112 of 400, forecast "about 52 by 18 Dec" | `concept/console/mission.html:86-91`; `shell.html:83` | Missions, mission reports, orders tagged to the mission, a forecast method | W1, W2 |
| "Across the wall": `lead_captured`, `content_gap ×4`, `visitor_intent ×6` | `shell.html:79-134` | Kernel `SignalStore` | W1 |
| Teach your site gaps with counts | `shell.html:79-134`; `concept/console.html` (3 gaps) | Content-gap clusters; `passed_questions` has no counts | W1 |
| Guardian: 212 probes blocked from 3 networks; "release check 2 min ago" | `shell.html:133`, `:146`; `concept/console/trust.html:86` | Edge block counters; a stored release-verification time | W2, W4 |
| Night timeline entries | `shell.html:139-162` | Activity ledger | W1 |
| Front-of-house live conversation (Mary K., 4 min, "wants you") | `shell.html:167-188`; `concept/console/live.html:85-113` | Threads, takeovers | W1 |
| Sales proposal "31 customers, 31 consent receipts, about KSh 6" | `shell.html:193-232`; `mission.html` | Customer lists, consent receipts, a cost estimate on cards | W2, W6, W5 |
| Store listings: version, eval passing, trigger "3 unanswered quotes in 30 days", promotion threshold, budget slider | `concept/console/store.html:88-160` | `WebAgentSpec` and pack listings; quotes | W1, W2 |
| Autonomy per job with "lowered 3 Nov"; promotion offer "20 approvals in 30 days" | `trust.html:103-139`, `:112` | Job dial history; approval kinds | W1, W5 |
| Standing rules with uses and expiry (g-2 used 6 times) | `trust.html:145-146` | Signed standing rules (U6) with use counts | W6 |
| "Actions without authority: 0" | `trust.html:86-92` | An invariant check over effects and receipts | W6 |
| Platform notices (update 3.4 at ring 2; incident 2 Nov 21:10–21:40) | `trust.html:152-154` | Platform notices from W3's trains and incidents | W3 |
| Support grant, ticket #218, 4 h | `trust.html:160` | Tickets, support grants | W3, W6 |
| Erasure request "11 of 13 stores" with per-store status | `trust.html:167-173` | Erasure requests and per-store answers | W6 |
| Memory facts with provenance ("from 41 orders since 2 Nov") | `trust.html:179-183` | Facts with sources | W1 |
| Receipt verify block with chain predecessor | `trust.html:195-201` | `prev_digest` on receipts, or the site log | W6 |
| Devices and people | `trust.html:203`; `concept/console.html` (Devices 2) | Devices, memberships | W6 |
| Versions "v14 Sunday hours, v13 Delivery zones FAQ" | `concept/console.html` | Release summaries | W4 |
| "Certificate renews in 61 days" | `concept/console.html:419` | Domain certificate expiry | W3 |
| Customers list (initials, summary, status) | `concept/console.html` | Customers | W2 |
| Multi-site cards: decisions, mission progress, budget used of cap | `shell.html:331-333` | Missions and budgets per site across a portfolio | W1, W3 |
| Lock-screen notifications, WhatsApp brief deep link | `phone.html:101-103`, `:190-193` | Notification deliveries | W5 |

### 7.2 Sites and visitor experience

| UI shows | Where | Missing source | Owner |
|---|---|---|---|
| "6 left today", "next batch 10:00", "sold out, back at 6:30" | `concept/site.html:219-222`, `:307`; `concept/experience/nav-agents.html:285-286` | Stock levels with batch times | W2 |
| Held order tracker for KB-1042 | `site.html:246` | Orders and order events (today only a decision card) | W2 |
| "412 entries since go-live" | `site.html:339` | The site log, or chained receipts | W6 |
| Catalogue with stock "as of 11:02", release v31 | `concept/experience/search.html:229-240` | Products, stock levels | W2 |
| Quote "price list v14, valid until Sat 14 Nov" with rule codes | `concept/experience/creator.html:229-240` | Price lists, configurable products, quotes | W2 |
| Gift plan with shares, versions, hidden prices | `concept/experience/artifact.html:187-293` | Visitor artifacts, shares, versions | W2 |
| "This week 12 stories, updated 09:10"; corrections; subscriber flag | `concept/experience/news.html:130-149`, `:210` | Posts, corrections, entitlements | W2 |
| "Was this right? Yes/No" feedback | `concept/experience/experience.js:116` | A turn feedback field | W1 |

### 7.3 Customer and owner journeys

| UI shows | Where | Missing source | Owner |
|---|---|---|---|
| Sessions list; "New sign-in on a Windows PC in Leeds" | `concept/journeys/customer-security.html:115`, `:132` | Visitor sessions and sign-in events | W6 |
| "What the site keeps" with retention per item | `concept/journeys/customer-account.html:127-134` | The data catalogue, rendered per site | W6, W8 |
| Consent receipts with given and withdrawn times | `customer-account.html` (cr-4e1b…, cr-91d0…, cr-c75a…) | Consent receipts on visitor accounts | W6 |
| Delivery slots with "full" | `concept/journeys/customer-order.html` | Delivery slots | W2 |
| USD amount at a pinned rate | `customer-order.html`; `customer-pay.html` | Exchange rates pinned on orders | W2, W6 |
| Order status timeline; change cut-off | `concept/journeys/customer-after.html` | Order events | W2 |
| Refund rule checks (up to KSh 2,000, five a week) | `customer-after.html` | Standing rules with use counts | W6 |
| Erasure receipt across 10 stores | `customer-security.html:357-371` | Erasure requests and answers | W6 |
| Agent door call log, tier, AP2 mandate chain | `concept/journeys/customer-agent.html` | Agent door calls, agent clients, mandates | W2, W6 |
| Books: gross, fee, net per order; withdrawal | `concept/journeys/owner-order.html:40` | Customer payments, settlements | W6 |
| Invoice INV-2026-11-0143, eTIMS, next charge, AI spend $3.40 of $30 | `concept/journeys/owner-billing.html:48` | Plans, subscriptions, invoices | W3 |
| "Essential plan from KSh 3,700/month" | `concept/assess.html:328` | Local prices in the plan catalogue | W3 |
| Parity per dimension (words 99, links 100, products 96, look 92); speed 4.8 s vs 0.9 s; "23 common questions" | `concept/assess.html` | Parity results and speed stored per page; common questions from the crawl | W4 |
| 31 redirects, 18/18 prices | `concept/move.html:136-141` | Redirects; price parity counts | W4 |

### 7.4 Control plane (operator and portfolio)

| UI shows | Where | Missing source | Owner |
|---|---|---|---|
| Sites live 612 (+9), moves in flight, edge availability 99.97%, error budget, Ask p95, inference spend and forecast | `concept/control-plane/fleet.html:66-70` | Fleet registry, gold tables, telemetry | W3, W8 |
| Cells, runner pools and utilisation, Migration Factory stages, SLO table | `fleet.html:76-144` | Cells, runner pools, SLO records | W3 |
| Operator ledger (op-9f1a…) | `fleet.html` | Operator ledger | W3 |
| Client table: plan, health, waiting, AI spend of cap, role, ring; margin $1,140 | `concept/control-plane/portfolio.html:89-96` | Partners, orgs, health scores, `partner_monthly` | W3 |
| Rings with site counts; canary vs control metrics; gate scores | `concept/control-plane/rollout.html:70`, `:127` | Trains, rings, rollouts, canary metrics | W3 |
| INC-0142 timeline, notices, lever | `concept/control-plane/incident.html` | Incidents | W3 |
| MRR $30,260, collected 94%, outstanding, reconciliation variance, rail first-attempt rates, dunning, partner payouts | `concept/control-plane/billing.html:67`, `:96` | Invoices, payments, reconciliations, `revenue_monthly` | W3 |

---

## 8. Decisions for James

1. **The three samples:** Kilele Bakery, a perfume shop under a new fictional name, and The
   Kestrel Post (§5.1). 02-architecture.md proposes a company site instead of the newspaper.
2. **Production v1 without Kafka and Iceberg** until a second cell (§3.3).
3. **The migration rule** (§4.2): ranges in `control.db` now, per-crate sets in production.
4. **Telemetry backend:** VictoriaMetrics, VictoriaLogs and Grafana, or the Grafana stack
   (`docs/09-data-architecture.md:1213-1214`).
5. **Retention hypotheses** marked *h* above, and the legal floor for orders and payments in
   Kenya (KRA), before the pilot's DPA.
