# The first release: Requirements

Status: v1.0 for James's review, Sat 7 Nov 2026, 18:30 EAT. Owner: the first release spec session.
Spec chain: **requirements (this file)** → [design.md](design.md) → [tasks.md](tasks.md).
Progress is tracked on the first release board artifact; this file defines what "done" means.

The first release is the single-node profile of Agentrix WebAgent Platform
([02 §11.1](../../docs/02-architecture.md#111-single-node-profile-first-release-self-host-marketplace)),
entered in the Vultr Agent Rush (Challenge 1). It is the first shippable product, not a demo:
every requirement here stays in the product. Submissions close **Mon 9 Nov 2026, 02:00 EAT**;
the target is **Sun 8 Nov, 20:00 EAT**.

**Headline:** *Upgrade your website into WebAgents.*
**Pitch:** *Your WordPress site was hacked. We moved it anyway, without carrying the hack
across, and nothing it tried got out.*

## 1. Who must be convinced, and by what

The first release is judged by people who could buy it. Each buyer needs to see one thing work for real.

| Buyer | What convinces them | Proved by |
|---|---|---|
| **A small-business owner** (WordPress, often hacked or neglected) | A free, honest health report of their own site in minutes; a new site that keeps their content, leaves the malware behind, answers customers, and goes live only when they say so, with undo | R1, R5, R6, R7, R8 |
| **An agency** (runs 10–200 client sites) | Migrations that are safe by construction, with receipts they can show a client; the same flow for every site | R4, R5, R7, R12, R13 |
| **Vultr** (judge, partner, possible customer) | Real work in real sandboxes on Vultr, the agent's model calls on Vultr Serverless Inference (the gateway is provider-agnostic, R11), containment that holds under attack, and a product that sells Vultr capacity (VMs, Inference, Object Storage) through every site it moves | R11, R12, R13, R14 |
| **A customer's AI agent** | A site it can use through a door built for agents, not screenshots | R9 |

**Vultr as a customer.** The same single-node build is a Vultr Marketplace app (self-host)
and the unit of Zavora's managed service on Vultr. Vultr could run it for its own sites and
offer it to its customers' WordPress sites. The first release must therefore install from a documented,
repeatable recipe (R14), not by hand.

## 2. Conventions

- Requirements use EARS: *WHEN* (event), *WHILE* (state), *IF … THEN* (unwanted behaviour),
  *WHERE* (option); otherwise ubiquitous. "The platform" means the first release deployment.
- Priority: **M** must ship Sunday; **S** should ship Sunday; **C** could; **W** won't this
  weekend (kept for the pilot).
- Each criterion has an id (`R3.2`). Tasks in [tasks.md](tasks.md) cite these ids; the board
  shows which criteria are covered, in progress, or verified.
- **Honesty rule (applies everywhere):** nothing on screen or in the video may claim more than
  the system did. Live means live. A fixture is labelled a fixture.

## 3. Functional requirements

### R1. Assess: a free health report for any public site (M)

*As a site owner, I want to paste my URL and see an honest report of my site, so that I know
what is wrong before I sign up.*

1. WHEN a visitor submits a public URL on the start page, THE platform SHALL start an
   assessment without an account and show its progress within 2 seconds. (M)
2. THE platform SHALL fetch and render the site only inside sealed rooms on the sandbox VM,
   with egress allowed to that site's hosts only, robots.txt respected, rate-limited, and
   identified as the platform's crawler. (M)
3. WHILE an assessment runs, THE Assess screen SHALL show the receipt ticking: hosts
   contacted, requests, bytes, refusals. (M)
4. WHILE an assessment runs, THE Assess screen SHOULD show frames from the sealed browser,
   live (adk-browser sealed mode) or, as a fallback, sealed screenshots at about one frame a
   second over SSE, labelled as such. (S)
5. WHEN the crawl of up to 50 pages finishes, THE platform SHALL show a health report in plain
   words with evidence for every finding: security signals (injected scripts, spam links,
   suspicious files, outdated components where visible), speed, broken links, and
   AI-readiness (can agents read and use this site?). (M)
6. IF the site cannot be reached or refuses the crawler, THEN THE report SHALL say so plainly
   and suggest the next step; it SHALL NOT invent findings. (M)
7. THE report SHALL end with one clear call to action: *Move this site*, which leads to sign-in
   and the claim (R2, R3). (M)

### R2. Owner sign-in (M)

1. THE platform SHALL let an owner create an account and sign in with a **passkey** first. (M)
2. WHERE the owner has no passkey-capable device, THE platform SHALL offer email + password
   with an authenticator (TOTP) code. (M)
3. THE platform SHALL store no password in plain form and no TOTP seed outside the sealed key
   store ([09 §2.3](../../docs/09-data-architecture.md)); sessions SHALL be HttpOnly, Secure,
   SameSite cookies. (M)
4. Social sign-in (Google, LinkedIn, GitHub). (W, pilot)

### R3. Claim: prove you own the site (M)

1. WHEN an owner starts a Move, THE platform SHALL ask for proof of control by a DNS TXT record
   or a file at a well-known path, and SHALL verify it from the sandbox VM. (M)
2. THE platform SHALL NOT replay, copy or rebuild any non-public content of a site until its
   claim is verified. (M)
3. FOR the demo, THE platform SHALL support the same flow on Zavora's own fixture domain
   (an infected WordPress we host), labelled as Zavora's fixture. (M)

### R4. Replay: read the old site from the inside, sealed (M)

1. WHEN a claim is verified, THE platform SHALL accept a WordPress export (WXR) or database
   dump and replay the old site in a sealed room on the sandbox VM: microVM where available,
   otherwise gVisor, with an internal-only network. (M)
2. THE replay SHALL produce a content inventory (pages, posts, media, menus, forms) stored by
   digest, with a receipt. (M)
3. IF replayed code tries to reach the network, THEN THE attempt SHALL fail with no route
   out, and SHALL be recorded on the receipt. (M)
4. IF the replay runtime is unavailable, THEN THE platform SHALL fall back to the public crawl
   plus the export (adk-site `migrate` without replay) and SHALL say so on the Move screen. (M)

### R5. Clean: leave the infection behind (M)

1. THE platform SHALL run the sanitiser and detectors over every inventory item before it can
   reach the new site. (M)
2. WHEN a detector fires, THE item SHALL be quarantined with: what was found, where, why it is
   dangerous, in plain words, and the evidence (digest, excerpt, screenshot). (M)
3. THE platform SHALL screen untrusted text for instructions aimed at AI agents (Vultr
   content-safety model + systemone risk score) and SHALL treat page content as data, never as
   instructions. (M)
4. THE owner SHALL be able to review quarantine and keep an item out, or restore a false
   positive with a reason; a restore SHALL be a signed decision. (S)
5. Quarantined items SHALL never be served, copied to the new site, or sent to a model as
   instructions. (M)

### R6. Rebuild and prove parity (M)

1. THE platform SHALL rebuild the site as a static, signed-release site from the clean
   inventory, using the builder model through the Model Gateway. (M)
2. THE platform SHALL render every rebuilt page in a sealed room with no network, and SHALL
   compute parity per page: perceptual and text similarity, plus a vision verdict with a
   one-line explanation. (M)
3. IF a page fails to build or render, THEN THE error SHALL be fed back to the builder and
   retried, at most 3 times, before the page is flagged for the owner. (M)
4. THE Move SHALL produce a candidate release only when site parity ≥ 0.95 and every flagged
   page is either fixed or accepted by the owner. (M)
5. THE Move screen SHALL show each step (Assess → Claim → Replay → Clean → Rebuild → Prove →
   Go live) with live status, time taken, and receipts. (M)

### R7. Go live: the owner decides, with a passkey (M)

1. WHEN a candidate release is ready, THE platform SHALL raise a **go-live card** showing:
   parity, what was left behind (quarantine summary), what the WebAgents will do *without
   asking* (only reversible work, per [ADR-012](../../docs/adr/012-autonomy-per-job.md)),
   that the old site is kept, and how to undo. (M)
2. THE owner SHALL approve the card with a passkey signature bound to the card's fingerprint
   (release digest + charter v1); WHERE no passkey exists, an MFA step-up. (M)
3. WHEN approved, THE platform SHALL sign the release, switch the site pointer atomically, and
   start probes; the new site SHALL be live within 60 seconds. (M)
4. THE owner SHALL be able to undo (roll back to the previous release, or back to the old
   site) in one action, for 30 days. (M)
5. Starting autonomy levels SHALL be written into charter v1 and signed on the same card;
   levels above *Ask me first* SHALL be listed in plain words; signing the card unchanged
   SHALL be a safe choice. (M)

### R8. The converted site, and "Ask this site" (M)

1. THE edge SHALL serve only signed releases; IF a release signature does not verify, THEN the
   edge SHALL refuse to serve it and keep the last good release. (M)
2. THE converted site SHALL offer **Ask this site**: answers grounded in the site's own
   content, with citations (title + span), streamed, AI-disclosed. (M)
3. IF the site's content does not answer a question, THEN the Concierge SHALL say it does not
   know and offer to pass the question to the owner. (M)
4. THE Concierge SHALL have no authority to promise prices, discounts, dates or refunds the
   site does not state; IF a visitor tries to make it do so (e.g. demands a discount or its
   instructions), THEN it SHALL refuse and the attempt SHALL appear in the owner's console. (M)
5. Pages SHALL have AWP/Markdown twins so agents can read them. (S)
6. THE site SHALL meet WCAG 2.2 AA on the demo pages and load its largest content in < 2.5 s
   on a mid-range phone over 4G. (S)

### R9. The agent door: a site built for AI customers (S)

1. THE site SHALL publish an agent door (MCP and AWP discovery) listing its capabilities. (S)
2. WHEN a customer's agent requests a custom order through the door, THE platform SHALL hold
   it, raise an approval card for the owner, and return a status the agent can poll. No
   payment this weekend. (S)
3. THE agent race (old site by screenshots vs new site by door) SHALL run live; IF the order
   flow is not ready by Sun 10:00, THEN it shrinks to a cited question (fallback). (S)

### R10. Owner console slice (M)

1. THE console SHALL be phone-first and offer: **Today** (the Chief WebAgent's brief),
   **Decisions** (cards: approve with passkey / decline), **The Move** (R6.5), **Trust**
   (receipts and the gauntlet score). (M)
2. THE WebAgents view SHALL show each WebAgent's jobs and dial positions (read-only this
   weekend), with positions above a job's ceiling shown locked. (S)
3. Every consequential action in the console SHALL produce a receipt the owner can open and
   verify. (M)
4. Gestures, voice, WhatsApp brief. (C)

### R11. Model Gateway: provider-agnostic, Vultr and Anthropic (M)

*Decided 7 Nov by James: the platform is not tied to one model provider. adk-rust supports
both, so the gateway ships with two: Vultr Serverless Inference and Anthropic.*

1. THE Model Gateway SHALL be the only process holding model-provider keys (Vultr Inference,
   Anthropic). (M)
2. EVERY model call SHALL go through it, naming a purpose; it SHALL route each purpose to the
   provider and model named in its routing table (build, concierge/vision, content-safety
   screening, decisions, embeddings) and SHALL never call a provider or model outside that
   table. (M)
3. THE gateway SHALL meter usage per site, purpose and provider, enforce a per-site budget,
   and show the meter in the console. (M)
4. IF a provider returns 429 or 5xx, THEN the gateway SHALL back off with jitter and surface
   a plain-words status; it SHALL NOT silently switch providers. A provider change is a
   routing-table change, recorded on a receipt and visible to the owner. (M)
5. Concierge retrieval on Vultr's vector store. (S)
6. THE gateway SHALL support both providers through adk-model's clients (the
   OpenAI-compatible Vultr preset and the Anthropic client), switchable per purpose by
   configuration with no code change. (M)

### R12. Sealed rooms and containment (M)

1. EVERY fetch, render, replay and build job SHALL run in a sealed room on the separate
   sandbox VM; the control VM SHALL run no untrusted content. (M)
2. EACH room SHALL have: gVisor (or microVM), a read-only root, a non-root user, CPU, memory,
   pids and time caps, no secrets, and a per-job egress allowlist enforced by the egress
   proxy and host nftables. (M)
3. THE egress proxy SHALL refuse instance metadata (169.254.169.254), loopback, private and
   link-local ranges, in every encoding (decimal, octal, IPv4-mapped IPv6), after redirects
   and DNS resolution (no rebinding), and SHALL refuse `CONNECT` abuse and smuggled
   requests. (M)
4. WHEN a job ends, THE room SHALL be destroyed and the destruction confirmed on the
   receipt. (M)
5. EVERY job SHALL return content-addressed artifacts and a signed receipt (inputs, policy,
   hosts contacted, refusals, resource peaks, result digests). (M)
6. A hard security review of the shipped egress proxy SHALL pass, one test per case in R12.3,
   before deploy; a failed case blocks the deploy. (M)

### R13. The containment gauntlet (M)

1. THE fixture SHALL plant the ten attacks of [04 §3](../../docs/04-vultr-agent-rush.md#3-the-containment-gauntlet-the-videos-containment-moment). (M)
2. THE score (*n/10 contained*) SHALL be computed from receipts, never typed in, shown on the
   Trust tab, and re-run on every deploy. (M)
3. THE video SHALL show the gauntlet running live against the deployed platform. (M)

### R14. Runs on Vultr, reproducibly (M)

1. THE platform SHALL run on Vultr: a `control` VM (edge, control, kernel, gateways, broker;
   Caddy TLS) and a `sandbox` VM (VX1 for microVMs; runner, egress proxy, gVisor), in one
   region, on a VPC; plus a small `fixture` VM serving the infected WordPress on Zavora's
   fixture domain. (M)
2. Firewall groups SHALL allow `control` 80/443 only and `sandbox` only the broker port from
   `control`. (M)
3. THE deployment SHALL be reproducible from the repo: cloud-init, a deploy script, and a
   README section "How we use Vultr". (M)
4. Releases and evidence SHOULD be stored on Vultr Object Storage. (S)
5. One-click Vultr Marketplace packaging. (C, pilot)

### R15. The submission (M)

1. Public MIT repo, README (Vultr services used, architecture, how the agent does real work,
   what pre-existed vs. built this weekend), live URL, ~4-minute video with the containment
   moment, slides (PDF), cover image. (M)
2. THE video SHALL cover problem, market, revenue, competition and future within 5 minutes. (M)

## 4. Non-functional requirements

| Id | Requirement | Target |
|---|---|---|
| N1 | Time from Move start to go-live card, 50-page WordPress | < 10 min p90 |
| N2 | Assess first visible progress | < 2 s |
| N3 | Ask this site: first token | < 2 s p50 on the demo site |
| N4 | Go-live to serving the new release | < 60 s |
| N5 | Model cost per moved site (Vultr Inference) | Shown per Move; target < $1 for 50 pages |
| N6 | Secrets | Only the Model Gateway holds the Vultr key; the sandbox VM holds none; no key in repo, logs or receipts |
| N7 | Accessibility | Console and site demo paths WCAG 2.2 AA |
| N8 | Honesty | No faked live frames, timings or scores (§2) |
| N9 | Recoverability | A crashed Move resumes from its last completed step |
| N10 | Licence | Whole repo MIT; pre-existing code disclosed |

## 5. Out of scope this weekend (W)

Payments (the fleet release; PayPal appears in journey mockups only), the fleet view and multi-site
operation, DNS cutover automation, VKE, billing, social sign-in, WhatsApp, voice, fleet
learning. Each is designed in docs 01–09 and returns in the pilot.

## 6. Open questions for James

1. **Fixture domain:** which domain hosts the infected WordPress fixture and the live
   platform (Vultr DNS is needed for auto SSL)?
2. **Region:** Atlanta (per 04) unless Vultr's VX1 is unavailable there.
3. **Requirement approval:** approve R1–R15 as written, or move any M ↔ S.
