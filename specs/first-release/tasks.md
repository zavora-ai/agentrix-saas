# The first release: Tasks and sprints

Status: v1.0 for James's review, Sat 7 Nov 2026. Spec chain: [requirements.md](requirements.md) →
[design.md](design.md) → **tasks (this file)**.

Generated from [tasks.json](tasks.json), the single source. Live status is on the **the first release
board** artifact; sessions update their task rows there (`todo` → `doing` → `review` →
`done`, or `blocked` with a note). A task is `done` only when its *done when* holds and the
spec session has checked it.

**Rules for every lane:** work only in your own crate or folder (the scaffold owns root files);
`cargo check -p <crate>` locally, heavy builds on the control VM; commit with named paths
(`git add -- <paths> && git commit -m … -- <paths>`); no push without James; no AI
attribution; stop local builds if free disk < 10 GB.

## Lanes

| Lane | Who |
|---|---|
| `james` | James |
| `spec` | The first release spec session (this board's owner) |
| `coordinator` | Coordinator (integration lead) |
| `data` | Data architecture session |
| `architecture` | WebAgent architecture session (sandbox plane) |
| `control-plane` | Control plane session (deploy, sign-in) |
| `console` | Owner console session |
| `experience` | Website experience session |
| `framework` | adk-rust framework session |
| `move` | Move builder (new session, proposed) |
| `gauntlet` | Gauntlet builder (new session, proposed) |
| `disk` | Disk cleanup session |

## S0 · Ground (Sat 19:00–22:00 EAT)

**Goal:** Everyone can build against one pinned workspace; Vultr access, VMs and model ids confirmed.  
**Gate:** G0 22:00: scaffold compiles, Vultr key works, VMs provisioning. Else rooms develop on local Linux Docker and deploy moves to Sun 08:00.

| Id | Lane | Pri | Task | Reqs | Est h | Depends | Done when |
|---|---|---|---|---|---|---|---|
| S0-01 | `james` | M | Approve requirements and design (or mark changes) | all | 0.25 | — | James says go, or lists changes on the board |
| S0-02 | `james` | M | Put VULTR_INFERENCE_API_KEY and a Vultr API key (for provisioning) in a local .env; confirm credits | R11, R14 | 0.25 | — | Keys present in the .env files named by S0-04 and S0-05; nobody else types them |
| S0-03 | `james` | M | Choose the platform domain and the fixture domain; delegate both to Vultr DNS | R3.3, R14.1 | 0.25 | — | Domains named on the board; NS records point to Vultr |
| S0-04 | `control-plane` | M | cloud-init and provisioning script for control, sandbox (VX1) and fixture VMs, VPC and firewall groups; James approves creation | R14.1, R14.2 | 2 | S0-02, S0-03 | Three VMs reachable over the VPC; firewall groups as in design §2; script in deploy/ |
| S0-05 | `data` | M | Confirm Vultr model ids via GET /v1/models; fix the routing table and the embedding model | R11.2 | 0.5 | S0-02 | Routing table in design §4.3 updated with confirmed ids and tool/vision support |
| S0-06 | `coordinator` | M | Workspace scaffold: every the first release crate as a compiling skeleton, adk-rust pinned rev, shared deps, CI (fmt, clippy, test); later tasks never edit root files | N10 | 1.5 | — | cargo check --workspace passes; CI green; crates per design §3 exist |
| S0-07 | `control-plane` | M | Passkey spike: adk-surface WebAuthn verification vs webauthn-rs; record decision D9 | R2.1 | 0.5 | S0-06 | Decision written in design D9 with the reason |
| S0-08 | `data` | M | agentrix-types: JobSpec, JobResult, Receipt, Finding, DecisionCard, EffectRequest; Ed25519 + RFC 8785 signing | R12.5, R10.3 | 1.5 | S0-06 | Types and sign/verify round-trip tests pass |
| S0-09 | `data` | M | SQLite schema and migrations (design §5) and the CAS store | N9, R12.5 | 1.5 | S0-06 | Migrations run on a fresh DB; CAS put/get by digest tested |
| S0-10 | `disk` | M | Free disk on James's Mac to >= 25 GB (stale target dirs), with James's OK per deletion | N6 | 0.5 | — | df shows >= 25 GB free |
| S0-11 | `framework` | M | Land #1348; rerun #1345, #1347, #1349; tell the coordinator the adk-rust rev to pin | D3 | 1 | — | Pinned rev builds the scaffold; PR states on the board |
| S0-12 | `spec` | M | Spec committed; the first release board live; lanes briefed | all | 1 | — | Board shows every task; each lane has its brief |

## S1 · Vertical slice (Sat 22:00 – Sun 04:00 EAT)

**Goal:** Paste a URL → sealed crawl → report → sign in → claim → the Move to a candidate with parity → the site serves a signed release with Ask.  
**Gate:** G1 04:00: if replay is not working, R4.4 fallback (crawl + WXR) is the path.

| Id | Lane | Pri | Task | Reqs | Est h | Depends | Done when |
|---|---|---|---|---|---|---|---|
| S1-01 | `architecture` | M | agentrix-runner: verify signed specs; Docker + runsc rooms; caps; read-only root; non-root; artifacts by digest; destroy and confirm; JobEvent stream | R12.1, R12.2, R12.4, R12.5 | 4 | S0-08 | A fetch job runs in gVisor on the sandbox VM and returns a result with destroyed=true |
| S1-02 | `framework` | M | Room images: room-fetch, room-render, room-build (pinned digests) | R1.2, R6.2, R12.2 | 3 | S1-01 | Each image runs one job to completion under its caps |
| S1-03 | `architecture` | M | Egress wiring: per-job allowlist from the spec into agentrix-egress; sandbox nftables force room traffic through it | R12.2, R12.3 | 2 | S1-01 | A room cannot reach any host off its allowlist, nor 169.254.169.254 |
| S1-04 | `data` | M | agentrix-models: purpose routing, metering, budgets, backoff, /v1/screen and /v1/decide | R11.1, R11.2, R11.3, R11.4 | 4 | S0-05, S0-08 | Each purpose answers from Vultr; usage rows written; 429 path tested |
| S1-05 | `coordinator` | M | Broker client in agentrix-node and the Assess pipeline: crawl job → adk-site scan findings → plain-words report | R1.1, R1.2, R1.3, R1.5, R1.6, R1.7 | 4 | S1-01, S1-02 | Assessing the fixture domain lists its planted spam links and footer script with evidence |
| S1-06 | `move` | M | agentrix-move: inventory (crawl + WXR) → sanitise, detect, quarantine → rebuild → render → parity; persisted, resumable steps | R4.4, R5.1, R5.2, R5.3, R5.5, R6.1, R6.2, R6.3, R6.4, R6.5, N9 | 6 | S1-01, S1-04 | The fixture reaches a candidate release with parity >= 0.95 and the infection quarantined |
| S1-07 | `control-plane` | M | Sign-in server: passkey register/login; password + TOTP fallback; sessions; claim by DNS TXT or file, verified from the sandbox | R2.1, R2.2, R2.3, R3.1, R3.2, R3.3 | 4 | S0-07, S0-09 | Register, sign out, sign in by passkey and by TOTP; claim verified on the fixture domain |
| S1-08 | `console` | M | Port templates: start/assess, sign-in, claim, the Move (SSE), console shell | R1, R2, R3, R6.5, R10.1 | 5 | S0-06 | Screens render real data on a phone width, light and dark |
| S1-09 | `experience` | M | Converted site theme; edge serves verified releases by Host; Ask this site (SSE, citations, AI disclosure, honest don't-know) | R8.1, R8.2, R8.3 | 5 | S1-04 | A cited answer and an honest don't-know on the rebuilt fixture site |
| S1-10 | `framework` | S | Sealed browser for room-fetch (B1) and the frame feed (B2, #1349); fallback screenshotter | R1.4 | 4 | S1-02 | Frames stream to the Assess screen from a sealed room |
| S1-11 | `gauntlet` | M | Fixture: infected WordPress compose from examples/website S3 + the infected overlay + the ten attacks; on the fixture VM | R13.1, R3.3 | 3 | S0-04 | The fixture site is live on the fixture domain with all ten attacks planted |
| S1-12 | `control-plane` | M | Deploy pipeline: build on the control VM, ship runner and egress to sandbox, systemd units, Caddy TLS, smoke test | R14.1, R14.3 | 3 | S0-04, S0-06 | deploy/deploy.sh takes a commit to a running stack with a passing smoke test |

## S2 · Owner decides (Sun 04:00–10:00 EAT)

**Goal:** Go-live with a passkey, undo, Trust tab, replay, the agent door, first deploy on Vultr, egress review passed.  
**Gate:** G2 08:00: egress review passes or deploy is blocked. G3 10:00: agent race or its fallback.

| Id | Lane | Pri | Task | Reqs | Est h | Depends | Done when |
|---|---|---|---|---|---|---|---|
| S2-01 | `move` | M | LaunchGate: card fingerprint (release digest + charter v1); sign the release on approval; atomic pointer switch; probes; rollback for 30 days | R7.1, R7.3, R7.4, R8.1 | 4 | S1-06 | Approve → live < 60 s; rollback restores the previous release |
| S2-02 | `console` | M | Go-live card and Decisions: passkey approval over the fingerprint; starting levels in plain words; locked positions | R7.1, R7.2, R7.5, R10.1, R10.2 | 3 | S1-07, S2-01 | The owner approves go-live with a passkey on a phone |
| S2-03 | `console` | M | Trust tab (receipts: list, open, verify; gauntlet score) and the Today view | R10.1, R10.3, R13.2 | 3 | S0-08 | Every receipt opens and verifies against the published key |
| S2-04 | `data` | M | Chief WebAgent brief for Today, from activity and receipts (concierge purpose) | R10.1 | 2 | S1-04 | A plain-words brief of what happened, with links to receipts |
| S2-05 | `architecture` | M | room-replay: WordPress from WXR/dump, internal-only network; microVM on VX1 if available, else gVisor | R4.1, R4.2, R4.3 | 4 | S1-01 | Replay inventory produced; the phone-home attempt fails and is on the receipt |
| S2-06 | `console` | S | Quarantine review; restore as a signed decision with a reason | R5.4 | 2 | S1-06 | Owner restores a false positive; decision on the receipt |
| S2-07 | `experience` | S | Agent door: AWP discovery + MCP request_custom_order → held → owner card → pollable status | R9.1, R9.2 | 3 | S1-09 | An agent's order raises the owner's card and polls its status |
| S2-08 | `experience` | M | Concierge refusal policy (no discounts, no instructions); the attempt shows in the console; AWP/Markdown twins | R8.4, R8.5 | 2 | S1-09 | Gauntlet attack 10 refused and surfaced |
| S2-09 | `framework` | M | Independent egress security review: one test per case (rebinding, redirects, IPv6/encoded, CONNECT, smuggling); architecture fixes what fails; sign-off | R12.3, R12.6 | 3 | S1-03 | All cases pass in crates/agentrix-egress/tests; sign-off noted on the board |
| S2-10 | `control-plane` | M | First deploy on Vultr: three VMs, HTTPS live URL | R14.1, R14.2, R14.3 | 2 | S1-12, S2-09 | Live URL serves the start page over HTTPS |
| S2-11 | `data` | M | Usage meter and budget in the console; model cost per Move | R11.3, N5 | 1.5 | S1-04 | Cost per Move shown; budget stop tested |

## S3 · Gauntlet and polish (Sun 10:00–15:00 EAT)

**Goal:** 10/10 gauntlet from receipts on the live URL; acceptance run; README, slides, video.  
**Gate:** Video recorded by 15:00.

| Id | Lane | Pri | Task | Reqs | Est h | Depends | Done when |
|---|---|---|---|---|---|---|---|
| S3-01 | `gauntlet` | M | Gauntlet runner: ten attacks against a deployment; score from receipts; on the Trust tab; runs on every deploy | R13.2, R13.3 | 3 | S1-11, S2-10 | 10/10 on the live URL, computed from receipts |
| S3-02 | `spec` | M | Acceptance run: verify every Must criterion on the live URL; file defects on the board | all | 2 | S2-10 | Each Must criterion marked verified or failed with a defect |
| S3-03 | `james` | S | Agent race rehearsal with computer use; keep or fall back by 10:00 | R9.3 | 1 | S2-07 | Decision on the board |
| S3-04 | `control-plane` | S | Releases and evidence on Vultr Object Storage | R14.4 | 2 | S2-10 | A release served from Object Storage-backed CAS |
| S3-05 | `experience` | S | Accessibility and performance pass on site and console | R8.6, N3, N7 | 2 | S2-10 | AA checks pass on demo paths; LCP < 2.5 s; Ask first token < 2 s |
| S3-06 | `data` | S | Concierge retrieval on Vultr's vector store | R11.5 | 2 | S1-09 | Ask answers cite chunks retrieved from Vultr |
| S3-07 | `spec` | M | README: How we use Vultr, architecture, how the agent does real work, disclosure of what pre-existed | R14.3, R15.1 | 2 | S2-10 | README reviewed by James |
| S3-08 | `spec` | M | Slides (PDF) and the video script under 5 minutes: problem, market, revenue, competition, future | R15.1, R15.2 | 2 | — | Slides and script approved by James |
| S3-10 | `data` | M | Switch gate: when acceptance passes on Claude (Phase 1), run the bake-off, switch routes to Vultr by configuration, re-run acceptance and the gauntlet on Vultr | R11.2, R11.6, R13.2 | 2 | S3-02 | Routes on Vultr; acceptance and 10/10 gauntlet pass on Vultr; bake-off result and route change on receipts |
| S3-09 | `james` | M | Record the video (live gauntlet, live go-live, the race or its fallback) on Vultr routes | R13.3, R15.1 | 2 | S3-01, S3-08, S3-10 | Video uploaded |

## S4 · Ship (Sun 15:00–20:00 EAT)

**Goal:** Defects burned down; freeze 17:00; submitted by 20:00 (hard close Mon 02:00).  
**Gate:** Submitted.

| Id | Lane | Pri | Task | Reqs | Est h | Depends | Done when |
|---|---|---|---|---|---|---|---|
| S4-01 | `coordinator` | M | Defect burn-down from the acceptance run | all | 3 | S3-02 | No open Must defect |
| S4-02 | `coordinator` | M | Feature freeze 17:00; final deploy; gauntlet re-run 10/10 | R13.2, R14 | 1 | S4-01 | Final deploy tagged; score 10/10 from receipts |
| S4-03 | `spec` | M | Submission package: every item in 04 §6 checked | R15.1 | 1 | S4-02 | Checklist complete |
| S4-04 | `james` | M | Fill the lablab form and submit | R15.1 | 0.5 | S4-03 | Submitted before 20:00 EAT |

## Load by lane (estimated hours)

| Lane | Hours |
|---|---|
| `data` | 15 |
| `control-plane` | 13.5 |
| `console` | 13 |
| `experience` | 12 |
| `framework` | 11 |
| `architecture` | 10 |
| `move` | 10 |
| `coordinator` | 9.5 |
| `spec` | 8 |
| `gauntlet` | 6 |
| `james` | 4.25 |
| `disk` | 0.5 |

## After Sunday: pilot sprints

P1 payments through the Effects Gateway (PayPal, M-Pesa) · P2 fleet profile and multi-site ·
P3 Vultr Marketplace app · P4 WhatsApp brief and approvals · P5 social sign-in ·
P6 fleet learning tier A (security signatures). Each is designed in docs 01–09.
