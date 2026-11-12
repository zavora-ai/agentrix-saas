# Agentrix SaaS: the PayPal AI Hackathon entry (spec v2)

**Deadline:** Thu 12 Nov 2026, 12:00 PST, which is 23:00 EAT. We submit at **18:00 EAT**.
- Feature freeze is **Wed 11 Nov, 18:00 EAT**.
- After submission, nothing linked may change until winners are announced (about 21 Dec).
- The research and the first plan are in `~/Developer/hackathons/reports/PayPal AI Hackathon winning plan.md`.
- v2 replaces that plan's design where they differ (James, 9 Nov night).

**One line:** Agentrix is an autonomous AI company. Its agents bill, collect, pay and settle with other AI-run companies through PayPal. Every dollar is booked in Zavora ERA, bounded by a mandate the owner signs, and provable.

## The story (what the video shows)

The core is **receivables, payables and debt**. Agentrix runs a SaaS business: it sells its own WebAgent plans.

1. **Human to agent.** A customer chats with Agentrix's sales desk at `money.agentrix.website`.
   - The customer subscribes with PayPal Subscriptions.
   - ERA records the lead, then the win, the customer and the recurring invoice.
   - The receipt posts when PayPal's signed webhook checks out.
   - Support handles a cancellation and a refund.
2. **Agent to agent (the hero).** Kilele Bakery is a customer that runs its own Agentrix staff at `kilele.money.agentrix.website`.
   - Agentrix's Receivables agent issues Kilele's annual-plan invoice in ERA and sends it to Kilele's agent door as a signed A2A request.
   - Kilele's Payables agent checks the invoice against the mandate Kilele's owner signed with a passkey, and settles it on PayPal.
   - **Both ERA ledgers post from one PayPal transaction id:** Agentrix's receivable clears and Kilele's payable clears.
3. **Debt.** When Kilele is short, the two agents agree an instalment plan inside both owners' mandates.
   - ERA tracks the debt: ageing, credit limit and reminders.
   - Receivables collects each instalment on its due date. The kernel wakes on the date; nothing polls.
4. **Payables.** Agentrix's own bills are paid through PayPal Payouts inside the founder's mandate. These are Vultr and Anthropic usage, a contractor, and a referral partner. Over the mandate, the founder approves with a passkey.
5. **Proof.** A vendor email with an attached PDF says "our bank details changed". The PDF is parsed in a no-network sandbox, and the model proposes paying the new account. The governed path refuses, and a signed receipt records the refusal. Then:
   - A forged PayPal webhook is rejected.
   - A crash mid-payout still pays exactly once.
   - `adk-rust audit verify` passes on the books, then fails when one byte is flipped.

## Systems and subdomains

These are explicit A records under `agentrix.website`, all pointing to one VM, `saas-node`. **No existing record in the zone changes.** The wildcard keeps serving the Vultr entry for every other name.

| Host | Service | What it holds |
|---|---|---|
| `money.agentrix.website` | SaaS node, company 1 | Agentrix's WebAgents front (sales and support desks), the owner console, and the AG Studio treasury board |
| `kilele.money.agentrix.website` | SaaS node, company 2 | Kilele Bakery's AI staff and owner console |
| `pay.agentrix.website` | SaaS node, agent door | A2A, the AWP manifest, checkout, PayPal webhooks, and published agent keys (`/.well-known/...`) |
| `books.agentrix.website` | Zavora ERA demo instance | Both companies' books and CRM: double entry, AR, AP, ageing, credit limits, reminders, recurring invoices, CRM. Postgres, Redis and the ERA API in Docker |

**Books are ERA, not ours.** Agents reach ERA through the open-source `mcp-erp` (Apache-2.0, its Zavora ERA backend, 44 tools) and ERA's REST API for CRM and recurring invoices.
- ERA itself is proprietary and runs here as a hosted service, as PayPal does. It is not copied into this repo.
- **No app-specific ledgers.** The adk-rust action log (exactly-once) and the signed audit log are runtime guarantees, not books.

**Money is PayPal, through direct REST** (`crates/agentrix-effects`). It covers Subscriptions, Orders v2, Invoicing, Payouts, refunds, Vault and signed webhooks.
- Every PayPal event posts to ERA through a "PayPal clearing" bank account.
- Every ERA payment leaves through PayPal.
- Agent-to-agent settlement has two routes:
  - **Primary:** Kilele's owner saves PayPal once (Vault). Settlement is then a merchant-initiated Orders v2 capture, made only with Kilele's agent's signed approval of that invoice.
  - **Fallback:** a B2B payout from Kilele's own PayPal app.

## adk-rust carries the product

★ means it's on camera. The README lists every crate with where it's used.

| Crate | Job |
|---|---|
| ★ adk-kernel | Resident supervisor for each company. Wake sources: PayPal webhooks, ERA events, due dates (collections, instalments), founder messages. Also the event bus and control plane |
| ★ adk-graph | Invoice-to-cash and procure-to-pay as graphs with interrupts and checkpoints. A held approval or a crash resumes where it stopped |
| ★ adk-runner | Runs every agent turn. The founder can steer a running agent (`Runner::steer`) |
| ★ adk-guardrail | Input, output and tool-call validation. Injection screening on inbound email and invoices. PII redaction before anything is logged or sent to a model |
| ★ identity: `adk-core::identity` + adk-auth | Typed app, user, session and execution identity for every agent of every company. Signed agent requests between companies (Web Bot Auth, RFC 9421). Per-agent scoped tokens through RFC 8693 token exchange. A credential broker, so no secret ever enters a model |
| ★ adk-memory | Each counterparty's payment behaviour and promises to pay. That memory drives debt plans and collections tone |
| ★ adk-core governance | Deny-by-default policy, approvals bound to the exact call (an edit voids them), quorum, freeze, exactly-once effects, and spend metering that includes model tokens |
| ★ adk-auth signed audit | RFC 9162 Merkle log with signed checkpoints, checked offline with `adk-rust audit verify` |
| ★ adk-payments | AP2-style intent and cart mandates on both sides, and the governed checkout. A **PayPal adapter** is contributed upstream |
| ★ adk-server | A2A v1 between the companies; an SSE stream to the AG Studio harness |
| ★ adk-tool (MCP) | `mcp-erp` client to Zavora ERA |
| ★ adk-sandbox + adk-code | Untrusted vendor documents are parsed in a container with no network |
| ★ adk-harness | The governed harness that AG Studio's swapped-in harness talks to |
| ★ adk-surface | Decisions in plain words; passkey approvals on the phone |
| ★ adk-awp, adk-site, adk-website | The public WebAgents front and its agent door |
| ★ adk-eval | Attack suite. The case list is committed by hash first, then scored from ERA and PayPal, never from the conversation |
| adk-agent, adk-session | The team of agents and the sessions per customer and counterparty |
| adk-model, adk-anthropic | Claude and Vultr through one gateway, with prompt caching |
| adk-skill | Amos's playbooks: AR chase, AP run, cash briefing, month-end |
| adk-rag | Price list, plan terms and the contract a dispute refers to |
| adk-realtime, adk-audio | The founder talks to the Chief by voice; the Chief's narration |
| adk-browser, adk-computer-use | The Chief drives the product during the recording |
| adk-drivers, adk-deploy, adk-telemetry, adk-bench, adk-retry-reflect, adk-plugin | Collections email and DNS; deploy; cost per run; a governed-path timing figure; recovery from PayPal 429 and 5xx errors |

There is no `adk-identity` crate. Identity lives in `adk-core::identity` (typed identities) and adk-auth (who an agent is to the outside world).

**Pin:** adk-rust stays at `3e797c38`, the platform's pin. Moving to a newer `origin/main` rev is allowed only if a needed feature is missing, and must be checked across the whole workspace first.

## Models

One gateway (`crates/agentrix-models`) and a hard cap per day across both companies (`SAAS_DAILY_MODEL_CAP_USD`).

| Provider | Model | Used for |
|---|---|---|
| Claude | Sonnet 5.5 | The Chief, and negotiation of debt plans and disputes |
| Claude | Haiku 5.5 | Routine agent turns |
| Vultr | Inference | Desks (chat), classification, invoice extraction, embeddings, text-to-speech |

Both providers' costs are posted to ERA as the company's own expenses.

## Partner and presenter

- **Sponsor partner: AG Grid.** The treasury board is built with AG Studio, themed in the Agentrix look. Its widgets are a mandate gauge, an audit-verified badge and a debt-plan timeline. Studio's AI harness is **swapped** for a thin TypeScript `AgAiHarness` that streams to an adk-rust agent. Fallback after the Wed 12:00 cut line: AG Grid Community (MIT) tables.
- **Presenter:** the Chief presents its own company.
  - The Agentrix mascot is the face.
  - The Chief writes the script from its run log, and James checks every claim.
  - Vultr text-to-speech voices it, and HeyGen Avatar IV renders it.
  - The edit uses real one-take product footage, an "AI presenter" label, and a 10–15 second cameo from James.

Setup for every partner is in `partners.md`.

## Separation from the Vultr entry

- **This repo** has its own git history. It was split from the WebAgent platform's `saas` branch on 9 Nov.
- **Public repo:** `zavora-ai/agentrix-saas`, exported with `scripts/publish-public.sh`, which now defaults here.
- **Build cache:** `~/Developer/hackathons/.agentrix-saas-target` (`scripts/wcargo`).
- **Three-VM deploy:** `deploy/deploy.sh` and `deploy/provision.sh` are the Vultr entry's deploy scripts. They're disabled here; this entry gets its own single-VM deploy (`deploy/saas/`, to be written).
- **Infrastructure and keys:**
  - VM label prefix `saas-`, with its own firewall group and SSH key.
  - Units are `agentrix-saas-*`, and data lives in `/var/lib/agentrix-saas`.
  - Its own PayPal sandbox apps, Anthropic key and Vultr Inference key, all in `.env`.
  - Passkeys use RP ID `money.agentrix.website`.
- **Never touch:** the `agentrix-webagent-platform` repo and its worktrees, agentrix.website's existing DNS records, the `agx-*` VMs, the shared PayPal sandbox app and its webhooks, or the Telegram bot.

## Hosting for a month: saas-node

- **One Vultr VM, 4 vCPU / 8 GB**, runs everything:
  - the SaaS node (both companies)
  - ERA (Postgres, Redis and its API in Docker)
  - sandbox containers
  - Caddy, with a certificate for each of the four hosts
- **No separate sandbox or fixture VMs.** Snapshot it after submission.
- **Agents wake on events.** Nothing polls.
- **Spend limits.** The daily model cap, per-visitor rate limits, and a judge mode that replays the attack demos with the scripted model, at zero model cost.

## Build slices

Each slice gets its own worktree from `main` in this repo and its own crate. The coordinator merges, after fmt, clippy and tests pass.

| Slice | Owns | Builds |
|---|---|---|
| A. Books | `crates/saas-books`, `deploy/saas/era/` | Local ERA demo instance; two tenants seeded with a PayPal clearing account; the `mcp-erp` client; posting rules from PayPal to ERA; CRM and recurring invoices through ERA REST |
| B. Company | `crates/saas-company` | Kernel per company, graphs, agents (Chief, Revenue, Receivables, Payables) on the governed path, guardrails, memory, identity, model mix. Reference: branch `wip-company-agents`, minus its own mandate and ledger |
| C. Settle | `crates/saas-settle` | A2A between the companies with Web Bot Auth; AP2-style intent mandate (passkey) and per-invoice cart mandate; PayPal settlement through Vault, with Payouts as fallback; instalment plans. Reference: branch `wip-agent-checkout` |
| D. Face | `crates/saas-web`, `web/saas-board/` | WebAgents front and owner console on the four hosts; AG Studio board with the adk-rust harness |
| E. Proof and ship (Wed) | `deploy/saas/`, `specs/saas/` | Attack suite, sandbox parsing, audit verify, saas-node deploy, presenter pipeline, video |

**Contracts between slices** come first, in `crates/saas-core`:
- the `Books` trait (A)
- company and agent identities (B)
- mandate and settlement types (C)
- the board's data feed (D)

Fakes let slices build in parallel.

**Cut lines:**
- **Tue 18:00:** the hero settlement runs end to end in sandbox, or the instalment negotiation drops to a scripted turn.
- **Wed 12:00:** if AG Studio isn't running, fall back to AG Grid Community.
- **Wed 18:00:** freeze.

## Words

- Say "AP2-style", "ACP-shaped" and "Web Bot Auth (IETF working-group draft)". Never "compliant".
- Say "Autonomous inside the authority the owner signs". Never "no human".
- Zavora ERA is "hosted", and its code is not in this repo.
- Quote test counts only from a real run on the submitted commit.
