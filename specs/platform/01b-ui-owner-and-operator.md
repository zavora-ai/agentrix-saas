# 01b · The owner's and the operator's UI: every page, what is built, what is missing

Status: analysis v1, Sat 7 Nov 2026. Read-only survey of the designed UI so build sessions can
turn it into a production product. Scope: everything a **site owner** and a **platform
operator** (Zavora, agencies, resellers) see. The visitor and customer side is out of scope.

**Sources read in full**
- Concepts: `concept/index.html`, `assess.html`, `move.html`, `console.html`, `concept/console/*`,
  `concept/journeys/owner-*.html` with `owner.css` and `owner.js`, `concept/control-plane/*`,
  `concept/styles.css`.
- Docs: `docs/07-owner-console.md`, `docs/08-control-plane.md`, `docs/01-product.md`,
  `docs/03-trust.md`; `specs/platform/voice.md` and `how-we-work.md` for house rules.
- Code (the first release): `crates/agentrix-console` (Page enum, views, templates, static) and the
  `auth`, `claim`, `launch`, `decisions`, `brief`, `pages`, `assess` modules of
  `crates/agentrix-node/src`; `specs/first-release/requirements.md` and `status.md`.

**How to read a page entry.** Every page has the same fields:

| Field | Meaning |
|---|---|
| Route | The first release route if built; otherwise a proposed route (marked *proposed*) |
| Purpose | The one job of the page |
| Components | The kit components it uses (§5) |
| Data and source | What it shows, and the store, type or service that supplies it |
| Actions | What the owner or operator can do, and what each action triggers |
| States | Empty, loading, running, waiting, done, error, frozen, offline… |
| Status | **Built** (the first release code), **Partly built**, **Drawn** (a concept page), **Described** (docs only), **Missing** |
| Tags | Workstreams (below) |

**Workstreams**

| Tag | Workstream |
|---|---|
| W1 | WebAgent runtime |
| W2 | Sample websites |
| W3 | Control plane |
| W4 | Onboarding and owner journey |
| W5 | Owner console |
| W6 | Identity, payments and trust |
| W7 | Design system and voice |
| **W8 (proposed)** | **Channels and notifications**: WhatsApp, email, push doorbell, voice, on-call paging, the message catalogue (§4.7). Nothing in W1–W7 owns outbound messages to owners and operators, and every surface depends on them |
| **W9 (proposed)** | **Data, metrics and data rights**: the metric catalogue every number comes from (07 §8, 09 §6.1), server-rendered charts, export bundles, erasure with per-store receipts. Owners see these numbers on every page; no workstream owns their source |

**Route scheme (proposed).** The concepts use four hosts: `agentrix.ai` (assess.html:358),
`os.agentrix.ai/kilele/...` (shell.html:39), `console.agentrix.ai/kilele` (console.html:196) and
`console.agentrix.ai/operator/...` (fleet.html:48). The first release serves everything on one origin with
`/console/*` for the signed-in owner's single site (pages/mod.rs:44-53). Proposal:
- Public: `/`, `/assess/{id}`, `/keep/{assessment}`, `/auth/*`. Previews on
  `{slug}.preview.agentrix.site` (assess.html:331).
- Site scope: `/console/{site}/…`. Keep the first release's `/console/…` as a redirect to the last site
  used. Short decision links `/d/{card}` (phone.html:193) for WhatsApp, email and CLI QR codes.
- Portfolio scope: `/portfolio/{partner}/…`; white-label hosts such as
  `clients.mwangaza.example` map onto it (portfolio.html:60).
- Operator scope: `/operator/…` (fleet.html:48, rollout.html:56, incident.html:54,
  billing.html:55).
- Receipts stay at `/receipts/{digest}`; the first release renders them signed out too (pages/mod.rs:214-236).

---

## 0. Summary

- **Pages counted:** 34 onboarding screens and states (§1); 44 owner-console pages and panels
  plus 12 object modules (§2); 7 control-plane concept pages (6 product pages and a scope
  overview) plus 45 operator, portfolio and self-host pages that doc 08 specifies but no concept
  draws (§3). §4 lists 56 gaps and a catalogue of 23 messages the owner receives.
- **The first release built 13 server-rendered pages** (`Page` enum, crates/agentrix-console/src/lib.rs:27-54):
  Start, Assess, Sign in, Register, Claim, the Move, Today, Decisions, Decision, Trust, Receipt,
  WebAgents (read-only), Quarantine. Plus the export upload, Move restart, parity accept, undo
  (rollback) and the approval ceremony with passkey or authenticator code bound to the card's
  fingerprint (auth/passkey.rs:4-8, decisions/mod.rs:7-13).
- **The first release did not build the OS shell.** Its console is a five-tab app (Today, Decisions, The
  Move, Trust, WebAgents; templates/console/base.html:14-20). No dock, lane, worlds, command bar,
  Night Time, Activity monitor, Freeze, Store, missions, live takeover, phone tabs, WhatsApp,
  billing or settings. Nothing in the control plane is built.
- **The biggest gaps:** account recovery, devices and sessions, team members and roles,
  notification settings, Freeze, the site's own records (catalogue, orders, bookings, pages) as
  editable objects, the standing-rule editor and the four card types the docs depend on (digest,
  agent-raised, plan, held-by-cap), billing and invoices, the whole control plane except six
  concept screens, and every email and WhatsApp message the owner receives.
- **Design system:** three stylesheets share `styles.css` tokens but diverge on marks, decision
  cards, approval sheets, WhatsApp threads and night/dark themes. The first release's `app.css` diverged
  again (system fonts, pill buttons, its own dark theme). §5 lists the kit.
- **Copy:** mostly plain already. The faults are slogan headlines, "not X but Y" lines, cute
  labels that hide meaning ("Walls holding", "Earned it?", "Across the wall"), unexplained
  numbers (risk "0.08") and internal terms on owner screens. §6 has 34 rewrites and the rules.

---

## 1. Owner onboarding journey, page by page

The concepts and 07 §7.1 order the journey as **discover → account → plan → claim → Move → go
live → first brief → first order → billing** (owner-account is step 2, owner-plan step 3;
concept/journeys/journeys.js:23-31). The brief for this analysis put plan before account. Keep
the concept order: the subscription approval and its receipt need an account to belong to, and
the first charge is on go-live day anyway (08 §4.2), so asking for a plan first buys nothing.

### O1.1 Start: paste your address
- **Route:** `GET /` (assess/mod.rs:231), `POST /assess` (:232), `GET /assess/fixture` (:233).
- **Purpose:** start a free assessment with one field and no account.
- **Components:** top bar, eyebrow, field, primary button, info card, fixture label.
- **Data and source:** none before submit. Optional fixture link from config
  (`StartView.fixture_url`, view.rs:177-182).
- **Actions:** submit an address → the address is checked to be public, a site row and an
  assessment are created, redirect to O1.2. "Try it on Zavora's fixture" starts the fixture.
- **States:** default; validation error in words (`StartView.error`, start.html:13).
- **Status:** **Built** (templates/start.html:1-34). **Drawn** richer in assess.html:173-188
  (three promises: no account, no passwords, only what's public) and owner-discover.html:36-44
  (how owners arrive: shared preview, footer, agency, Marketplace).
- **Gaps:** referral source capture (shared preview link, "Made with Agentrix" footer, agency
  code) for the growth loops (01 §7); consent capture if the visitor leaves contact details
  (08 §4.1 Lead stage).
- **Tags:** W4, W7.

### O1.2 Assess: reading your site, live
- **Route:** `GET /assess/{id}` (running state), SSE `GET /assess/{id}/events` (assess/mod.rs:234-235).
- **Purpose:** show the crawl happening in sealed rooms, so the owner believes the report.
- **Components:** sealed-room chip, tick counters (hosts, requests, bytes, refusals), live
  frame with honest label, progress line, live findings list.
- **Data and source:** assessment row and broker receipts; frames from the CAS (`/frames/{digest}`,
  pages/mod.rs:53); totals (`Tick`, view.rs:59-65).
- **Actions:** none; the page streams.
- **States:** running; no frame yet ("The sealed browser hasn't sent a frame yet",
  assess.html template:32); live versus sealed screenshot label (FrameKind, view.rs:79-83).
- **Status:** **Built** (templates/assess.html:21-38). **Drawn** in assess.html:191-204, which
  adds two things the first release lacks: a page list with a per-page sealed chip, and a "Refused: 1
  redirect" callout (assess.html:201).
- **Tags:** W4, W1.

### O1.3 Health report
- **Route:** `GET /assess/{id}` (done state).
- **Purpose:** an honest report with evidence for every finding.
- **Components:** summary card, severity chips, report sections, finding with evidence
  disclosure, receipt link, call to action.
- **Data and source:** `Report` (view.rs:218-224) from the assessor's findings; each finding's
  evidence and receipt (view.rs:96-111).
- **Actions:** open evidence; "Move this site" → `/auth/signin?next=/sites/{id}/claim`
  (assess/report.rs:266-267).
- **States:** done; a section with no findings shows `empty_words` (assess template:65-67).
- **Status:** **Built** (templates/assess.html:50-76). **Drawn** in assess.html:207-244 and
  owner-discover.html:47-54.
- **Gaps:** "Share this report"; a printable or PDF version an owner can show their web person.
- **Tags:** W4.

### O1.4 Assess could not finish
- **Route:** `GET /assess/{id}` (unreachable or refused).
- **Purpose:** say what happened and what to do, without inventing findings (R1.6).
- **Data and source:** `Unreachable { said, next_step }` (view.rs:245-248).
- **Actions:** "Try another address" → O1.1.
- **Status:** **Built** (templates/assess.html:40-48). **Not drawn.**
- **Tags:** W4, W7.

### O1.5 Preview: your new website, alive
- **Route:** *proposed* `/assess/{id}/preview`, served from `{slug}.preview.agentrix.site`.
- **Purpose:** show the rebuilt site next to the old one before any account exists.
- **Components:** two mini browser frames, speed chips, parity card with four bars (words and
  pages, links, products and prices, look), "Private preview · 7 days · not indexed" chip.
- **Data and source:** a preview release built by the Mover from the public crawl; parity from
  the render room (assess.html:274-284).
- **Actions:** open the preview; share it.
- **States:** building, ready, expired after 7 days (no screen drawn for expired).
- **Status:** **Drawn** only (assess.html:247-286; owner-discover.html:55-66). The first release builds a
  release only after a verified claim (R3.2), so there is no pre-account preview.
- **Note:** the preview needs an answer to R3.2: a preview built from public pages only does not
  break it, but it must be labelled as a public-pages preview.
- **Tags:** W4, W1, W2.

### O1.6 Talk to the preview, and teach it
- **Route:** inside the preview host; Ask is `POST /ask` (site/mod.rs:106).
- **Purpose:** the owner asks their own site a question and sees an honest answer with a source.
- **Components:** chat thread, source line, "teach your site" card with three answers.
- **Data and source:** Concierge grounded on the crawl; unknowns become a `content_gap`.
- **Actions:** ask; answer a teach card ("Yes, always", "Only to order", "No",
  assess.html:301) → a Known fact, held until an account exists.
- **Status:** **Drawn** only (assess.html:289-304). Ask is built for live sites, not previews.
- **Tags:** W4, W1.

### O1.7 Meet your WebAgents
- **Route:** *proposed* `/assess/{id}/webagents`.
- **Purpose:** one first suggestion per WebAgent, drawn from this site's evidence.
- **Components:** agent card (mark, name, suggestion, one button).
- **Data and source:** findings and gaps of this assessment; each is a proposal, nothing acts.
- **Actions:** "See the questions", "Review prices", "See evidence", "Turn on"
  (assess.html:313-316). Before an account these can only lead to "Keep it".
- **Status:** **Drawn** only (assess.html:307-318).
- **Tags:** W4.

### O1.8 Keep it, or share the preview
- **Route:** *proposed* `/keep/{assessment}` → O2.1.
- **Purpose:** turn the preview into a site in setup; or send the preview to someone.
- **Components:** passkey glyph, primary button, price line, share box with copy link.
- **Data and source:** preview URL; plan "from" price (assess.html:328).
- **Actions:** "Continue with passkey" → O2.1 with the assessment attached; "Share preview" →
  copy link (the first referral moment, 01 §3).
- **Status:** **Drawn** (assess.html:321-336; owner-discover.html:67-71). The first release has only "Move
  this site" (templates/assess.html:70-75).
- **Tags:** W4.

### O2.1 Create the account
- **Route:** `GET /auth/register`; `POST /auth/passkey/register/start|finish`; `POST /auth/password`
  (auth/routes.rs:44-51).
- **Purpose:** passkey first; without one, email and password with an authenticator app.
- **Components:** fields (name, email), primary button, disclosure "No passkey on this device?",
  error line.
- **Data and source:** `AuthView { next, passkeys }` (view.rs:254-259); accounts in `auth/schema.sql`.
- **Actions:** create with passkey → session cookie and redirect to `next`; password path →
  authenticator enrolment (O2.2).
- **States:** passkeys off on this deployment (register.html:9); error in words
  (`#auth-error`).
- **Status:** **Built** (templates/auth/register.html:1-46). **Drawn** with tabs and social
  buttons (owner-account.html:37-73). Social sign-in is **Won't** for the first release (requirements R2.4).
- **Gaps:** email confirmation; terms and privacy acceptance; residency disclosure at sign-up
  (01 §11.3, 03 §5); passkey on a second device.
- **Tags:** W6, W4.

### O2.2 Set up the authenticator app
- **Route:** `POST /auth/totp` (auth/routes.rs:51).
- **Purpose:** enrol TOTP for sign-in and for approvals on devices without a passkey.
- **Components:** QR, manual key, 6-digit code field.
- **Data and source:** `{key, otpauth, qr_svg}` from `/auth/password` (auth/routes.rs:11).
- **Actions:** confirm the code → account created, session set.
- **Status:** **Built** (register.html:31-42). **Drawn** (owner-account.html:60-72).
- **Gaps:** recovery codes shown once at enrolment; "Lost your authenticator?" path. Neither is
  drawn or built (see §4).
- **Tags:** W6.

### O2.3 Sign in
- **Route:** `GET /auth/signin`; passkey login; password + TOTP (auth/routes.rs:44-51).
- **Purpose:** return to the console.
- **Status:** **Built** (templates/auth/signin.html:1-40). **Not drawn.**
- **Gaps:** recovery, new-device alert, rate-limit and lockout copy, "signed out because your
  session expired".
- **Tags:** W6, W7.

### O2.4 The Chief WebAgent introduces itself
- **Route:** first landing after sign-up (`/console/{site}/setup`, *proposed*).
- **Purpose:** say who speaks for the team and what comes next (plan, claim, Move).
- **Components:** team mark, one paragraph, next-step button.
- **Status:** **Drawn** (owner-account.html:75-79). **Not built:** The first release lands a signed-in
  owner without a site on their pending claim or `/` (wiring/mod.rs:199-212).
- **Tags:** W4, W5.

### O3.1 Choose a plan and set the AI budget (the fleet release)
- **Route:** *proposed* `/console/{site}/setup/plan` (owner-plan.html:33).
- **Purpose:** pick Essential, Business or Commerce; set the monthly AI cap; see the real cost.
- **Components:** Chief WebAgent suggestion, plan cards (one pressed, "Suggested for Kilele"
  badge), budget slider with live note, cost note, payment-rail radio cards.
- **Data and source:** plan catalogue (`plans.toml` in `zavora-fleet`, 08 §4.2); the
  suggestion's reason from assessment evidence; cost estimate from 01 §6.2.
- **Actions:** pick plan → budget max follows (owner.js:25-34); move budget → note says what is
  billed above the included amount (owner.js:37-46); pick a rail → the next button names the
  provider (owner.js:49-56).
- **States:** inside included budget; above it (overage at cost plus a disclosed margin).
- **Status:** **Drawn** (owner-plan.html:37-80). **The fleet release**; nothing built.
- **Gaps:** currency choice and display (USD here, KSh elsewhere; see §6.3); tax lines; trial or
  no trial; plan comparison detail page; agency-sold plans where the agency bills.
- **Tags:** W6, W4.

### O3.2 Pay: the provider's own page, then back
- **Route:** provider pages (PayPal, Paystack), M-Pesa prompt; return to *proposed*
  `/console/{site}/setup/plan/return`.
- **Purpose:** approve a subscription without card data touching the platform.
- **Components:** provider hand-off frame, success line with receipt.
- **Data and source:** Effects Gateway subscription create; provider webhook; receipt.
- **Actions:** agree on PayPal; save card on Paystack; set up M-Pesa prompts.
- **States:** approved, nothing charged yet (owner-plan.html:106-110). Not drawn: cancelled on
  the provider page, provider error, webhook late.
- **Status:** **Drawn** (owner-plan.html:82-112). **The fleet release.**
- **Tags:** W6.

### O4.1 Claim the domain
- **Route:** `GET /sites/{id}/claim?method=dns|file`, `POST /sites/{id}/claim`,
  `POST /sites/{id}/claim/verify` (claim/mod.rs:534-535).
- **Purpose:** prove control with a DNS TXT record or a file, checked from the sandbox.
- **Components:** tabs (DNS, file), numbered steps, key-value block, check button, status lines.
- **Data and source:** `Claim` (view.rs:285-297): TXT `_agentrix.<domain>` =
  `agentrix-site-verification=<code>`, file at `/.well-known/agentrix-verify.txt`
  (claim/mod.rs:17-19); probe result.
- **Actions:** "Check now" → sandbox fetch → verified (owner set on the site, R3.2) or "what we
  saw" in words.
- **States:** pending, checking, verified, failed with `said` (claim.html template:16-51).
- **Status:** **Built** (templates/claim.html:1-52). **Drawn** (owner-domain.html:35-62), which
  adds the Caretaker as the speaker and registrar-specific help ("a Kenyan registrar",
  owner-domain.html:36).
- **Gaps:** registrar-specific guides; "I don't know who my registrar is" path; an agency doing
  the claim on the owner's behalf (08 §8 J1); re-check at go-live (03 §3).
- **Tags:** W4, W6.

### O5.1 Upload your exports and start the Move
- **Route:** `PUT /sites/{id}/uploads/{name}`, `POST /sites/{id}/moves` (decisions/moves.rs:3-9,
  :86-90); page `GET /console/move` (pages/mod.rs:48).
- **Purpose:** give the Mover the WordPress export, and optionally the database dump and
  wp-content, then start.
- **Components:** three file fields with per-file status, start button.
- **Data and source:** `UploadRow` (view.rs:433-440); uploads checked by type and magic bytes,
  stored by digest (decisions/moves.rs:32-33).
- **Actions:** upload (static/upload.js) → stored; "Start the Move" → Move run.
- **States:** none uploaded, uploaded with size and time; fixture sites skip uploads.
- **Status:** **Built** (templates/console/move.html:44-63). **Not drawn.** The concepts say the
  Move works "from its own export" (owner-move.html:39) but never show how the owner provides it.
- **Gaps:** step-by-step export help per host; what to do on Wix, Squarespace or Shopify (01
  names them; only WordPress exists).
- **Tags:** W4, W1.

### O5.2 The Move, live
- **Route:** `GET /console/move`; `GET /moves/{id}`, SSE `GET /moves/{id}/events`
  (decisions/watch.rs:47-48).
- **Purpose:** watch the site move step by step, with receipts.
- **Components:** step list with state dots and receipt links, current sealed room with ticks
  and frame, parity meter, left-behind summary, go-live banner, spend line.
- **Data and source:** `MoveView` (view.rs:414-430): steps, tick, frame, parity, quarantine,
  card, spend (`spend_words`, S2-11); `ui_events` stream `move:<id>` (launch/ports.rs:8).
- **Actions:** open receipts; open the go-live card when raised.
- **States:** none, running, waiting, live, stopped (view.rs:310-317); step pending, running,
  waiting, done, failed (view.rs:336-343).
- **Status:** **Built** (templates/console/move.html:64-110). **Drawn** much richer in
  move.html:134-322: a route line "old → new", an 8-stop rail (Read, Claim, Replay, Clean,
  Rebuild, Prove, Go live, Watch; move.html:147-156), room cards with network, keys, isolation,
  limits, timer and "destroyed ✓" (move.html:165-214), and a "Refused during the Move" log
  (move.html:312-319). owner-move.html:35-64 is the short version.
- **Gaps:** "Watch" (30 days after go-live) is drawn as a rail stop but has no page; an estimate
  for long Moves ("ready in about 6 hours", 08 §3.4); Move notifications (§4.7).
- **Tags:** W4, W1.

### O5.3 Left behind (quarantine)
- **Route:** `GET /console/quarantine` (pages/mod.rs:51); `POST /sites/{site}/quarantine/{item}/restore`
  (decisions/mod.rs:187).
- **Purpose:** explain each item kept out, with evidence; restore a false alarm only by a signed
  decision with a reason.
- **Components:** item card (title, plain words, where, evidence disclosure, state chip),
  restore form with required reason.
- **Data and source:** `QuarantineItem` (view.rs:733-745).
- **Actions:** "Ask to restore it" → raises a `quarantine-restore` card (CardKind, view.rs:552-557);
  executables can never be restored (decisions/mod.rs:16-18).
- **States:** left behind, restore asked, restored (view.rs:724-729).
- **Status:** **Built** (templates/console/quarantine.html:1-37). **Drawn** in move.html:217-269
  with "Keep left behind" / "Release… (passkey)" and "Can't be released: executable files never
  move" (move.html:254); the gestures concept shows a "leave items behind" card that the owner
  signs (gestures.html:133-139).
- **Note:** the concepts disagree on whether keeping items behind needs a signature
  (gestures.html:137) or is the default (move.html:232). Make "left behind" the default with no
  signature; only release is signed.
- **Tags:** W4, W6.

### O5.4 Parity per page, and accepting a flagged page
- **Route:** part of `/console/move`; `POST /moves/{id}/accept` (decisions/moves.rs:91).
- **Purpose:** show text, links, products and look per page; let the owner accept a flagged page.
- **Data and source:** `Parity`, `ParityPage` with a one-line vision verdict (view.rs:377-394).
- **Actions:** "Accept this page as it is" (move.html template:98).
- **Status:** **Built** (templates/console/move.html:92-101). **Drawn** with old/new screenshots
  per page and page chips (move.html:271-291), which the first release does not show.
- **Tags:** W4, W1.

### O5.5 The Move stopped
- **Route:** `/console/move` (stopped).
- **Purpose:** say why it ended and start again.
- **Actions:** "Start a new Move" → resumes a faulted Move or starts fresh (move.html template:35-42).
- **Status:** **Built.** **Not drawn.**
- **Tags:** W4, W7.

### O6.1 The go-live card
- **Route:** `GET /console/decisions/{id}` (pages/mod.rs:47); `POST /decisions/{id}/approve|decline`
  (decisions/mod.rs:185-186).
- **Purpose:** one card that says exactly what going live does, approved with a passkey or a
  code.
- **Components:** decision header (mark, "Mover asks", risk chip), title, effect list, facts
  table, starting levels per job with locked positions, "What your approval signs" disclosure
  with fingerprint fields, approval sheet (passkey button, code form), decline.
- **Data and source:** `Card` (view.rs:606-631): `effects` rendered from canonical arguments,
  `facts`, `levels` (charter v1, R7.5), `fingerprint`, `fields`, `passkey_options` with
  challenge = SHA-256(fingerprint ‖ nonce) (auth/passkey.rs:4-8).
- **Actions:** approve with passkey → `decision.approved` receipt naming the method → LaunchGate
  signs the release, moves the pointer atomically, probes (launch/mod.rs:5-17); approve with code;
  decline → `decision.declined` receipt.
- **States:** waiting, approved, declined, expired (view.rs:562-568); decided shows method, time
  and receipt (decision.html template:68-72).
- **Status:** **Built** (templates/console/decision.html:1-76). **Drawn** in owner-golive.html:32-56,
  move.html:295-310 and gestures.html:126-132 (raised by a thumb-up).
- **Gaps:** the DNS switch. Drawn as "Your domain's address record points to Agentrix's edge"
  (owner-golive.html:40) and "We guide the one DNS change" (move.html:299); the first release goes live on
  the platform address (`site.live_url`, view.rs:137; status.md:42) and DNS cutover is out of
  scope (requirements §5). Production needs a DNS step page: guided records at the owner's
  registrar, or Vultr DNS when the domain is with us, held for the owner (08 §3.3).
- **Tags:** W4, W6, W1.

### O6.2 You're live, and undo
- **Route:** `/console/move` (live); `POST /sites/{site}/rollback` with `to=previous|old-site`
  (decisions/mod.rs:188).
- **Purpose:** confirm the site is live; offer undo for 30 days.
- **Components:** success line, live link, two undo buttons, Chief WebAgent note.
- **Data and source:** release pointer; `release.rollback` receipt (launch/mod.rs:19-21).
- **Actions:** undo to the previous release or to the old site → atomic pointer switch.
- **States:** live; undone ("Undo started… Most visitors see it within minutes",
  owner-golive.html:55).
- **Status:** **Built** (templates/console/move.html:25-33). **Drawn** (owner-golive.html:50-65).
- **Gaps:** "fallback retired in a week" notice (owner-golive.html:88); undo confirmation step.
- **Tags:** W4, W5.

### O7.1 The first night (Night Time timeline)
- **Route:** *proposed* the console's Back office field during quiet hours (C3).
- **Purpose:** show what the WebAgents did overnight, as recorded.
- **Components:** timeline (time, mark, line), "what reached you" card.
- **Data and source:** ledger and receipts; quiet hours (`QuietHours`, 07 §3.5).
- **Status:** **Drawn** (owner-day1.html:32-41; shell.html:139-164). **Not built.**
- **Tags:** W5, W1.

### O7.2 The first morning brief
- **Route:** `GET /console/today` (pages/mod.rs:45); WhatsApp and email (*proposed*, W8).
- **Purpose:** what happened, what needs you, what it cost.
- **Components:** brief card with lines linked to receipts, event count, spend; WhatsApp bubble
  with quick replies; share button.
- **Data and source:** `briefs` table written by `ChiefBrief` from the last 24 h of receipts; the
  model writes only a one-sentence headline from counts (brief/mod.rs:1-17).
- **Actions:** open linked items; share ("my website did this overnight", owner-day1.html:52).
- **States:** no brief yet ("Your first brief comes at 07:00…", pages/map.rs:485).
- **Status:** **Partly built** (templates/console/today.html:25-35). The first release refreshes on a timer
  over 24 h (brief/mod.rs:199), not at 07:00 in the owner's time zone; no WhatsApp, email or share.
- **Note:** the empty state promises 07:00 (pages/map.rs:485) but nothing schedules a 07:00 brief.
  Fix the copy or the schedule.
- **Tags:** W5, W8, W1.

### O8.1 The first customer order
- **Route:** `GET /console/decisions/{id}` (card kind `order-held`, view.rs:556).
- **Purpose:** approve or decline a held custom order with the exact effect shown.
- **Components:** decision card, effect list, why-this-risk line, approve/edit/decline.
- **Data and source:** `HeldOrders` from the agent door, idempotent per key (decisions/mod.rs:22-23).
- **Actions:** approve with the card open (counts as `reviewed`, 07 §5.3) → the door reports the
  order completed; decline.
- **Status:** **Partly built** for agent-door orders (R9.2). **Drawn** (owner-order.html:34-53)
  for a visitor's order through the Orders desk, with currency conversion pinned to the order
  (owner-order.html:40).
- **Gaps:** "Edit" (change price, date) is drawn on every card but defined nowhere; an order
  record page after approval (see C-ORD in §2.20).
- **Tags:** W5, W1, W6.

### O8.2 The payment settles (the fleet release)
- **Status:** **Drawn** (owner-order.html:56-71): waiting row, paid card ("captured exactly the
  amount you approved").
- **Data and source:** Effects Gateway PayPal order; verified webhook.
- **Tags:** W6.

### O8.3 The books: payout and fees (the fleet release)
- **Status:** **Drawn** (owner-order.html:75-88): ledger table with gross, fee, net, withdrawal.
- **Data and source:** ledger postings; provider fees.
- **Gaps:** where this lives in the console (a Cashier "Money" object view), and export to the
  owner's accountant.
- **Tags:** W6, W9.

### O8.4 Refunds: under the rule, and above it (the fleet release)
- **Status:** **Drawn** (owner-order.html:93-103): a refund that ran alone under a signed rule,
  and a card above the rule with "Offer a partial refund".
- **Tags:** W6, W5.

### O9.1 Billing: invoices and AI spend (the fleet release)
- **Route:** *proposed* `/console/{site}/billing` (owner-billing.html:33).
- **Purpose:** see the plan, invoices, payment state and AI spend against the cap.
- **Components:** status chip, tabs (Invoices, Change plan, Cancel, Export everything), invoice
  table, budget meter.
- **Data and source:** `agentrix-billing` invoices; eTIMS state from zavora-era (08 §4.2); Model
  Gateway usage.
- **Actions:** download an invoice (not drawn); open a tab.
- **States:** paid up; overdue, in reminders, read-only suspension (not drawn; see §4).
- **Status:** **Drawn** (owner-billing.html:35-56). The first release shows model spend on Today against a
  **daily** limit in USD (`Meter`, view.rs:483-496), not the concepts' monthly cap.
- **Tags:** W6, W5, W9.

### O9.2 Change plan
- **Status:** **Drawn** (owner-billing.html:58-67): a downgrade card listing what stops on the
  date, signed. Upgrade is described only (prorated one-off charge, 08 §4.2).
- **Tags:** W6.

### O9.3 Cancel
- **Status:** **Drawn** (owner-billing.html:69-84): timeline (export first, full service to the
  period end, 30 days read-only, closure, deletion after 90 days), cancel card, undo until closure.
- **Tags:** W6, W4.

### O9.4 Export everything
- **Status:** **Drawn** (owner-billing.html:86-91; trust.html:167): build bundle, ready line,
  7-day link, verify. **Not built.**
- **Data and source:** signed bundle: records, every signed release, receipts, analytics (07 §5.1).
- **Tags:** W9, W6.

### O10.1 Leaving: read-only period, closure and deletion
- **Purpose:** the screens and messages an owner sees after cancelling, or after non-payment.
- **Status:** **Described** (08 §4.1-4.2) and drawn only as a timeline (owner-billing.html:70-76).
  No read-only console state, closure page or deletion receipt page is drawn.
- **Tags:** W6, W4, W8.

### O10.2 Transfer out from an agency; take it home (self-host)
- **Status:** **Described** (08 §2.1 "transfer out" without the partner's consent; 08 §4.5 "take
  it home"). **Not drawn.**
- **Tags:** W3, W6.

---

## 2. The owner console ("Agentrix OS for owners"), page by page

### 2.0 Two generations, one target

- **v0.1 tabs** (concept/console.html:226-235): Today, Decisions, WebAgents, Website, Customers,
  Trust. Superseded by `concept/console/` (concept/index.html:219; 07 §0). Still useful for the
  Website and Customers panels, which the OS concepts never redrew.
- **The first release** (templates/console/base.html:14-20): Today, Decisions, The Move, Trust, WebAgents,
  as a bottom bar on phones and a side bar from 900 px (app.css:190-204). `NavTab` has five
  values (view.rs:143-150).
- **Target: the OS shell** (07 §4.2; shell.html). Map from tabs to regions (07 §4.5): Today →
  Back office brief card and Night Time; Decisions → the lane; WebAgents → dock, Store, Activity
  monitor; Website → Editor's app and Walk the site; Customers → Front of house plus Sales and
  Cashier apps; Trust → Trust centre and the trust pill. Below 900 px the shell reflows into the
  phone order (console.css:487-511).

**Build note.** The first release's server-rendered pages are the right base (how-we-work.md:63-66). The
shell is a layout over the same pages: status bar, dock and lane are server-rendered partials;
the field hosts the page; SSE feeds presence and lane updates.

### C1 The shell (desk)
- **Route:** `/console/{site}` (*proposed*; concepts `os.agentrix.ai/kilele`, shell.html:39).
- **Purpose:** one frame for everything: status, apps, the selected world, what needs you, the
  command bar.
- **Components:** status bar (site chip with switcher, world pager with live count, time pill,
  budget pill with mini meter, trust pill with decision count, camera pill when on, Freeze
  pill); dock (WebAgent icons with presence marks and badges, then Store, Activity, Memory,
  Trust); field; lane; command bar; overlays (Activity monitor, passkey sheet, app windows,
  gesture bar).
- **Data and source:** site and release (sites, releases); presence per WebAgent (adk-surface
  feed, F3 in 07 §9.2); budget (Model Gateway); waiting cards (decisions); quiet hours.
- **Actions:** switch world (07 §3.3); toggle Day/Night (concept only, console.js:17-26); open an
  app; Freeze (C30); focus the command bar with `/`.
- **States:** day, night (`data-time`, console.css:66-86), frozen (console.css:107; frozen note
  shell.html:75), phone order under 900 px.
- **Status:** **Drawn** (shell.html:41-296). **Not built.**
- **Tags:** W5, W7.

### C2 Back office · Day Time (Today)
- **Route:** `/console/{site}` default view; the first release `GET /console/today`.
- **Purpose:** greeting, the brief, work in progress, missions, signals that crossed, teach your
  site, the Guardian's status.
- **Components:** field head with state line; cards: brief (accent, citations as `cite` chips,
  verbs), WebAgent at work (task steps), mission progress, "Across the wall today" (signal pairs),
  Teach your site, Guardian.
- **Data and source:** brief (briefs table); task steps (kernel process table, F2); mission
  progress (mission projection, F5); signals per day (wall counts type, 07 §9.1); content gaps.
- **Actions:** approve from the brief (opens the sheet); read the mission report; "Ask about
  it" fills the command bar; "Watch", "Pause Editor"; "Give to Editor" (also drag a card onto a
  dock icon, 07 §3.8).
- **States:** no brief yet; frozen note.
- **Status:** **Drawn** (shell.html:78-136). **Partly built**: the first release Today has Needs you, the
  brief, questions visitors passed, model spend and recent receipts (templates/console/today.html:16-62).
- **Tags:** W5, W1.

### C3 Night Time
- **Route:** same; view switches by quiet hours.
- **Purpose:** during quiet hours, a timeline of recorded work; the lane is silent except P1.
- **Components:** timeline; "What reached you overnight"; "Who works at night" with "Change
  quiet hours"; night lane with "Stop everything" explainer.
- **Data and source:** ledger; `QuietHours` in the owner's IANA zone (U8).
- **Status:** **Drawn** (shell.html:139-164, 223-232). **Not built.**
- **Open question:** automatic switch or owner choice (07 §11.2). Also decide how Night Time
  relates to the system dark theme the first release already has (app.css:26-52); see §5.4.
- **Tags:** W5, W7.

### C4 The lane: Needs you, Proposals, Done for you
- **Route:** right column on the desk; top of the phone order.
- **Purpose:** every held decision and proposal, routed by priority.
- **Components:** section headers with badge; lane card (`ldec`: from-line, title, one line,
  verbs, done line, "Why?" box); quiet summary.
- **Data and source:** decision cards; agenda proposals (`ProactivityPolicy`); attention routing
  (P1/P2/P3, digest, drop; 07 §3.5).
- **Actions:** Approve (opens sheet), Edit, Decline, "See the diff", "Why?", "Plan it", "Not
  now" (snooze), "Why am I seeing this?".
- **States:** empty ("Nothing needs you", built in decisions.html:11); held by the daily cap
  ("Waiting for your brief (3)", 07 §3.5) **not drawn**; snoozed.
- **Status:** **Drawn** (shell.html:194-222). The first release has the Decisions list instead.
- **Tags:** W5.

### C5 The command bar and plan preview
- **Route:** bottom of the field.
- **Purpose:** ask or give a job; see the plan, the decisions to come and the cost before
  anything runs.
- **Components:** team mark, text field, mic, send; plan panel (steps with WebAgent, job and
  level chip; footer chips: decisions to come, cost estimate, "Nobody is contacted before you
  sign"; Go, Change).
- **Data and source:** Chief WebAgent planner; plan preview type (07 §9.1); numbers only via
  `analytics.metric` (07 §3.2).
- **Actions:** type → plan appears (concept triggers on input length, console.js:120-127); Go →
  in-authority steps start, the rest become cards.
- **States:** thinking, plan shown, one clarifying question (07 §3.2), models unavailable (07 §1.12).
- **Status:** **Drawn** (shell.html:236-253). **Not built.**
- **Tags:** W5, W1.

### C6 Decisions list
- **Route:** `GET /console/decisions` (pages/mod.rs:46); phone Decisions tab.
- **Purpose:** every waiting and decided card.
- **Data and source:** `DecisionsView { waiting, decided }` (view.rs:542-547).
- **Status:** **Built** (templates/console/decisions.html:1-32). The phone's Decisions tab is
  drawn only as one card "1 of 2" (phone.html:132); no list is drawn.
- **Gaps:** filters (by WebAgent, by kind), expired cards, the "held for your brief" row.
- **Tags:** W5.

### C7 Decision detail
- **Route:** `GET /console/decisions/{id}`; short link `/d/{id}` (*proposed*).
- **Purpose:** the exact effect, why this risk, approve, edit, decline and teach.
- **Components:** from-line with mark and risk chip; title; facts line; effect box "If you
  approve, exactly this happens" with quoted message; risk reasons as chips; actions; teach box
  on decline (radio reasons that can become a rule, console.html:289, 301); done line with
  receipt; fingerprint disclosure (built).
- **Data and source:** `Card` (view.rs:606-631).
- **Actions:** Approve → C8; Edit (drawn everywhere, specified nowhere); Decline and teach →
  optional standing rule (07 §3.7).
- **Status:** **Built** for three kinds (go-live, quarantine-restore, order-held; view.rs:552-557).
  **Drawn** in console.html:278-314 and phone.html:127-153. The teach-on-decline box is not built.
- **Tags:** W5, W6.

### C8 The approval sheet
- **Route:** overlay on any card.
- **Purpose:** passkey first; a single-use authenticator code bound to the same fingerprint
  without one. The receipt names the method.
- **Components:** fingerprint glyph, one-line effect, short fingerprint, Cancel, "Use passkey",
  step-up link, 6-digit field, "Approve with code", signed state.
- **Data and source:** canonical arguments; ceremony options (`passkey_options`).
- **States:** ready, signing, signed, step-up, bad code (console.css:302-326; console.js:50-109).
- **Status:** **Built** as an inline section (decision.html template:50-67; static/passkey.js).
  **Drawn** as an overlay (shell.html:279-295, and copies in store, mission, live, trust,
  gestures, owner-golive, owner-order) and as a phone bottom sheet that **lacks the code path**
  (phone.html:143-150).
- **Note:** The first release binds the challenge to the fingerprint inside the platform
  (auth/passkey.rs:4-8), so "signed" is true there. 07 §5.2 still tracks F1 for adk-surface.
- **Tags:** W6, W5, W7.

### C9 Card types (inventory, not a page)
Each needs a canonical-argument renderer and an effect list. Status per type:

| Card | Where | Status |
|---|---|---|
| Go live | owner-golive.html:32-56 | Built |
| Restore a left-behind item | quarantine.html template | Built (raise); drawn move.html:232 |
| Held order (agent door) | owner-order.html:34-53 | Built (door), drawn (desk) |
| Publish a release (prices, post) | shell.html:203-210; console.html:293-303 | Drawn |
| Reply to a review (with credit) | console.html:305-314 | Drawn |
| Send an email to a list (mission step) | mission.html:143-150 | Drawn |
| Change a desk rule (collection slots) | mission.html:151-157 | Drawn |
| Promote a job ("Earned it?") | trust.html:112; console.html:320 | Drawn |
| Add a WebAgent (charter amendment) | store.html:159 | Drawn |
| Install or update a pack | cli.html:109-122 | Drawn |
| Support grant | trust.html:160 | Drawn |
| Refund above the rule | owner-order.html:97-103 | Drawn |
| Change plan; cancel | owner-billing.html:60-83 | Drawn |
| Raise the AI cap | 08 §4.2 | Described |
| Plan card (sign once for a week of work) | 07 §5.4 | Described |
| Digest card (trivial items, Approve all) | 07 §3.5 | Described |
| Agent-raised card ("Asked by Sales · new recipient") | 07 §3.5 | Described |
| Unfreeze; roll forward after a platform hold | 07 §5.5 | Described |
| Send to client (agency note) | portfolio.html:168-174 | Drawn (agency side) |

- **Tags:** W5, W1, W6.

### C10 Activity monitor
- **Route:** overlay from the dock.
- **Purpose:** every process: why it woke, state, next wake, today's spend, Pause.
- **Components:** window, process table (who, state line, woke because, next, today, Pause).
- **Data and source:** `/kernel/processes`, `/kernel/events` (F2 in 07 §9.2).
- **Actions:** Pause → `PauseAgent` per job (resuming needs the owner).
- **Status:** **Drawn** (shell.html:256-276). **Not built.**
- **Tags:** W5, W1.

### C11 The dock and a WebAgent's app window
- **Route:** dock everywhere; app window *proposed* `/console/{site}/apps/{webagent}`.
- **Purpose:** an app = a WebAgent plus the things it looks after (07 §3.4): listing, presence
  and current process, objects editable by hand, dial per job, budget meter, receipts,
  Appearance, Pause and Remove.
- **Status:** dock **drawn** (shell.html:58-71). **The app window is not drawn anywhere.** It is
  the page that hosts each WebAgent's objects (C-ORD, C-CAT and others in §2.20) and the reason
  the console still works when models are down (07 §1.12).
- **Tags:** W5.

### C12 WebAgents: dials, jobs, budgets
- **Route:** `GET /console/webagents` (pages/mod.rs:50).
- **Purpose:** each WebAgent's jobs and how much each may do alone.
- **Components:** agent card with mark; per-job level list with current and locked positions and
  reasons; ladder control (five positions, `your max` marker unused, console.css:329-335);
  budget meter; on-duty toggle (console.html:397).
- **Data and source:** charter (charters table); `JobLevels`, `Position` (view.rs:572-592).
- **Actions:** lower a job (immediate, receipt); raise (signed card with evidence); pause.
- **States:** current, locked above ceiling with reason, lowered with reason and date
  (trust.html:128), paused.
- **Status:** **Built read-only** (templates/console/webagents.html:12, "Read-only this weekend").
  **Drawn** interactive in trust.html:100-140, store.html:141-151 and console.html:318-322 (a
  three-position dial, older).
- **Tags:** W5, W6.

### C13 Appearance editor
- **Purpose:** colour (with 3:1 checks), corners, glyph, name, signature seed, previews at
  20/28/40 px; a change is a `MarkSpec` change that ships in the next release (07 §3.4).
- **Status:** **Described** only; the generator is in `concept/identity/marks.html`.
- **Tags:** W5, W7.

### C14 The Store
- **Route:** *proposed* `/console/{site}/store` (store.html:64).
- **Purpose:** add WebAgents, packs and connections.
- **Components:** shelf headings, filter chips, listing cards (mark or pack icon, name, one line,
  meta chips such as "Suggested for you", "off by default").
- **Data and source:** listings generated from `WebAgentSpec` and pack bundles, never written by
  hand (07 §3.4); plan entitlements (01 §6.1).
- **Actions:** select a listing → C15.
- **Status:** **Drawn** (store.html:82-110). **Not built.**
- **Gap:** plan gating. The store's site chip says "Essential plan" (store.html:67) while it
  offers Sales, which is a Business feature (01 §6.1; owner-plan.html:49). No upgrade path is drawn.
- **Tags:** W5, W1, W6.

### C15 Add a WebAgent (What → Permissions → Authority → Sign)
- **Purpose:** read can/never, see what wakes it, set each job's starting level, set the
  budget, sign one charter amendment.
- **Components:** sheet head, stepper, "why suggested" with citation, can/never with effect tags,
  wake chips, ladder per job, budget slider, sign button, first-look card after adding.
- **Data and source:** `WebAgentListing`; `Kernel::agents` preview digest (07 §3.4).
- **Actions:** sign → charter vN+1, icon drops into the dock, first look from real data
  (store.html:162-169).
- **Status:** **Drawn** (store.html:113-170). **Not built.**
- **Copy fault:** the listing says "4 jobs" (store.html:88) but the sheet and the signature list
  three (store.html:142-150, 159).
- **Tags:** W5, W6.

### C16 Missions list
- **Route:** *proposed* `/console/{site}/missions`.
- **Purpose:** all missions with progress, budget slice, end date, state.
- **Status:** **Missing.** A dock icon exists (mission.html:77; phone.html:171) but no list.
- **Tags:** W5.

### C17 Start a mission: say it, read the contract, sign
- **Purpose:** turn a goal in words into a contract (goal, measure, owners, wakes, budget slice,
  authority, end date) and approve or sign it.
- **Status:** **Described** (07 §3.5) and shown as a three-card explainer (mission.html:187-195).
  The contract editor and its preview are **not drawn**.
- **Tags:** W5, W1.

### C18 Mission report
- **Route:** *proposed* `/console/{site}/missions/{id}` (mission.html:64).
- **Purpose:** weekly report: progress, what worked, what didn't, what next, counting method.
- **Components:** report card (stats, progress forecast, sections with citations, next steps with
  level chips, method note), contract card, last-night timeline, mission lane with its cards.
- **Data and source:** mission projection over charter goals, agenda, triggers (F5); ledger.
- **Actions:** Change the goal; Pause mission; approve step cards; share.
- **Status:** **Drawn** (mission.html:82-160). **Not built.**
- **Tags:** W5, W1, W9.

### C19 Front of house: the live floor
- **Route:** world pager → Front of house.
- **Purpose:** pages as rooms, visitors and AI agents as dots, desks with presence.
- **Components:** room tiles, visitor dots (talking, agent), desk line, conversation card ("wants
  you"), Concierge card, wall note.
- **Data and source:** presence feed; open threads (rendered as untrusted text, 07 §3.3).
- **Actions:** Take over, Sit in, Answer later; Desk settings; Preview as a visitor.
- **Status:** **Drawn** (shell.html:167-189). The first release has no Front of house; 07 §10 asked for a
  read-only conversation list in the first release, which was not built.
- **Tags:** W5, W1.

### C20 Live thread: sit in, take over, hand back
- **Route:** *proposed* `/console/{site}/live/{thread}` (live.html:85).
- **Purpose:** watch unseen, step in as yourself, hand back.
- **Components:** conversation head with mode chip, thread (visitor, desk with AI tag, owner with
  person chip, system lines), composer (sitting-in bar, reply field, hand back), whispers lane,
  Teach-your-site card.
- **Data and source:** thread events; whispers from the desk, never the back office (07 §3.6).
- **Actions:** Take over → "Wanjiru from Kilele joined · a person"; Send; Hand back → "Cakes is
  back · AI" (live.html:239-257); "Draft it" → Known fact and Editor draft.
- **States:** sitting in, taken, handed back, auto hand-back after about 10 minutes.
- **Status:** **Drawn** (live.html:86-143). **Not built** (needs U21).
- **Tags:** W5, W1.

### C21 Walk the site by voice
- **Route:** *proposed* `/console/{site}/walk`.
- **Purpose:** open your own signed release, talk, see the pointer, get an Editor draft, sign on
  screen.
- **Components:** mini site, pointer ring with label, voice control with captions, draft card
  with "Sign and publish", "Stop everything" pill.
- **Status:** **Drawn** (live.html:146-217). **Not built** (F7, U18).
- **Tags:** W5, W8, W1.

### C22 Concierge app: desks and Front of house rules
- **Purpose:** front desk and section desks, starters, peek, voice and banned phrases,
  configurable products with Assumed → Known prices, presentation scripts, artifacts,
  interactive budget, Teach your site, preview as a visitor (07 §3.3).
- **Status:** **Described.** Drawn only as two buttons (shell.html:185) and a one-paragraph rules
  view (trust.html:186-189). This is where the owner administers what the site may promise.
- **Tags:** W5, W1.

### C23 Website (the Editor's app)
- **Purpose:** live release, versions with one-tap rollback, Teach your site, how AI assistants
  see you, page editing by hand.
- **Components:** version list (pin, title, who approved and how, Live chip or Roll back),
  gaps list, "seen" chips.
- **Data and source:** releases; content gaps; agent-door capability list.
- **Status:** **Drawn** in the v0.1 console only (console.html:325-343). The first release offers undo on the
  Move page (move.html template:25-33). A page editor is **missing**.
- **Tags:** W5, W1.

### C24 Teach your site
- **Purpose:** questions the pages could not answer, with counts; answering creates a Known fact
  and an Editor draft.
- **Status:** **Drawn** (shell.html:124-129; console.html:333-338; owner variant in
  assess.html:298-302). **Partly built:** Today lists "Questions visitors passed to you" with a
  reply contact (today.html:37-45; `PassedQuestion`, view.rs:507-514), but there is no "Answer"
  action.
- **Tags:** W5, W1.

### C25 Customers
- **Purpose:** people and AI agents the site served: conversations, leads, orders, handovers.
- **Components:** conversation rows (initials or AI tile, name, one line, state chip).
- **Status:** **Drawn** in the v0.1 console only (console.html:346-354). In the OS model it splits
  into Front of house plus the Sales and Cashier apps (07 §4.5); none are drawn as lists.
- **Tags:** W5.

### C26 Trust centre
- **Route:** `GET /console/trust` (pages/mod.rs:49).
- **Purpose:** proof and control in one place (07 §5.1, twelve sections).
- **Components and sections, as drawn (trust.html):**
  1. Walls card with the invariant line (84-93).
  2. Freeze, large (95-98).
  3. Ladder per job, with fixed rules for the Concierge and Undo only for the Guardian, an
     "Earned it?" offer and a lowered note (100-140).
  4. Standing grants with bounds, usage and Revoke (142-147).
  5. Platform notices (149-155).
  6. Support access: grant request and agency role (157-162).
  7. Export and erasure with per-store answers and external copies (164-174).
  8. Memory: Known and Assumed facts, visitor memory policy (176-184).
  9. Lane: signed decisions and receipts with Verify; devices and people; your data (192-205).
- **Not drawn:** attention settings (quiet hours, daily card cap; 07 §5.1 item 3); the
  four-limit rule editor; device management actions.
- **Status:** **Partly built.** The first release Trust shows receipts and the containment gauntlet score
  (templates/console/trust.html:16-33). Everything else is drawn only.
- **Tags:** W5, W6, W9.

### C27 Receipt and Verify
- **Route:** `GET /receipts/{digest}` (pages/mod.rs:52).
- **Purpose:** one receipt, checked on the server and again in the browser.
- **Data and source:** `ReceiptDetail` (view.rs:678-694); keys at `/.well-known/agentrix/keys`
  (app.rs:224).
- **Status:** **Built** (templates/console/receipt.html:1-35, static/verify.js). **Drawn** as an
  inline expansion with fingerprint, device and method (trust.html:195-201), including a
  code-approved receipt that says what it proves (trust.html:196).
- **Tags:** W6.

### C28 Memory: what your WebAgents believe
- **Purpose:** Known and Assumed facts with provenance; confirm, correct, forget; visitor-memory
  policy.
- **Status:** **Drawn** (trust.html:176-184). **Not built.**
- **Tags:** W5, W1.

### C29 Standing rules and the rule editor
- **Purpose:** list, inspect and revoke standing rules; create one with all four limits (amount,
  recipients, frequency, expiry) from "Always allow" (07 §5.4).
- **Status:** list **drawn** (trust.html:142-147); editor **described** only. The first release has none.
- **Tags:** W6, W5.

### C30 Freeze and unfreeze
- **Purpose:** stop every WebAgent at its next safe point; the site keeps serving its signed
  release. Unfreeze needs the passkey or a code on the PWA (07 §5.5).
- **Status:** **Drawn** (shell.html:53, 75; trust.html:95-98; phone.html:176). **Not built**
  (no `freeze` anywhere in agentrix-node or agentrix-console), although 07 §10 put Freeze in the
  The first release console scope.
- **Actions:** Freeze → `Kernel::freeze` parks every process; the edge serves the signed release
  read-only; the Concierge answers from pages then offers the form (07 §5.5). Unfreeze → the
  approval sheet (C8), PWA only, resumes exactly what the freeze parked. Also from WhatsApp STOP,
  voice "stop everything" and the open-palm gesture with a 5-second countdown (C42).
- **States:** running; frozen (banner on every page, greyed marks); platform hold in force (a
  separate banner the owner cannot lift).
- **Copy and design fault:** the v0.1 console unfreezes with a plain toggle (console.html:481-487),
  which contradicts 07 §5.5. The OS concepts toggle back too (console.js:29-38); only the button
  label says "unfreeze with passkey" (phone.html:176).
- **Tags:** W5, W6.

### C31 Platform notices and support access
- **Status:** **Drawn** (trust.html:149-162). **Not built.** See §3 for the operator side.
- **Tags:** W5, W3.

### C32 Export and erasure
- **Status:** **Drawn** (trust.html:164-174). **Not built.**
- **Tags:** W9, W6.

### C33 Devices and people
- **Status:** **Drawn** as a read-only list (trust.html:202-203). No add, remove, rename, revoke
  or role assignment is drawn. See §4.
- **Tags:** W6.

### C34 Phone: lock screen
- **Purpose:** the doorbell: "2 decisions are waiting", "Your morning brief is ready", with no
  details.
- **Status:** **Drawn** (phone.html:95-107). **Not built** (web push, `PushDoorbell`).
- **Tags:** W8.

### C35 Phone: Now
- **Status:** **Drawn** (phone.html:109-125): greeting, team mark, brief, Needs-you stack with
  swipe hint, live strip, mission widget, tab bar. **Not built** (the first release's phone layout is the
  five-tab bottom bar).
- **Tags:** W5.

### C36 Phone: one decision
- **Status:** **Drawn** (phone.html:127-153). Built equivalent: the decision page at phone width.
  The phone sheet lacks the code path (phone.html:143-150).
- **Tags:** W5, W6.

### C37 Phone: Decisions list and Live tabs
- **Status:** **Missing.** Tabs exist (phone.html:122) but only Now and Apps screens are drawn.
- **Tags:** W5.

### C38 Phone: Apps
- **Status:** **Drawn** (phone.html:155-181): app grid, hold-to-talk, Freeze. **Not built.**
- **Tags:** W5.

### C39 WhatsApp
- **Purpose:** the brief at 07:00, everyday cards (Allow once, Decline), consequential cards open
  the PWA, STOP freezes, free text goes to the Chief WebAgent (07 §4.4).
- **Status:** **Drawn** (phone.html:188-194; console.html:214-219; owner-day1.html:44-48).
  **Not built** (Won't for the first release, requirements §5).
- **Fault:** the v0.1 brief offers "Approve order" as a WhatsApp reply (console.html:217) while
  money needs the PWA (phone.html:193, 200). Use "Open the order".
- **Tags:** W8, W6.

### C40 Email
- **Purpose:** the brief and the weekly review with links; approves nothing (07 §4.4).
- **Status:** **Described** only. No email is drawn.
- **Tags:** W8.

### C41 Voice
- **Status:** **Drawn** as captions (live.html:179-196) and hold-to-talk (phone.html:175).
  **Not built** (F7).
- **Tags:** W8, W1.

### C42 Gestures
- **Pages and panels:** camera consent sheet (gestures.html:155-169); camera pill (98); gesture
  bar with dwell ring and mirrored camera tile (111-116); focus and opened card (110); stop
  countdown (151-153); failures in words (gestures.html script, around lines 447-459); audit log
  panel (208-213); calibration (described, 235-245).
- **Status:** **Drawn and working on a real camera** (gestures.js, gestures.test.js). Not in
  The first release (R10.4 Could).
- **Tags:** W5, W7.

### C43 All sites
- **Purpose:** an owner with several sites sees held decisions, missions, budget and wall status
  per site; the phone merges cards with site labels (07 §6).
- **Status:** **Described**; drawn only as three site cards (shell.html:326-335). The first release binds a
  session to one site (pages/mod.rs:97-105).
- **Tags:** W5, W3.

### C44 Settings (missing)
- No settings page exists in any concept. Items scattered across the docs that need one home:
  quiet hours and time zone, daily card cap, notification channels and their hours, WhatsApp
  pairing, language, currency, the AI budget, brand and Appearance, visitor-memory scopes,
  business details. See §4.
- **Tags:** W5, W8.

### 2.20 Site-specific administration: how the console hosts archetype modules

The owner must be able to run their business records by hand: 07 §1.12 ("every WebAgent's
objects can be edited by the owner directly") and §3.4 (Editor pages, Sales leads and quotes,
Cashier orders and payments, Reputation reviews, Concierge desks and artifacts, Guardian
incidents, Caretaker domain and backups). **No concept draws any of these object views.** The
closest are the v0.1 Website and Customers panels (console.html:325-354), the books table
(owner-order.html:77-86) and the mission desk-rule card (mission.html:151-157).

**Proposal: an app hosts object modules.**
- A **module** is a set of objects with list, detail and edit views, owned by one WebAgent app
  and shipped by a signed pack (08 §6.5), so archetypes add modules without changing the shell.
- Each module declares, per action, its effect class (07 §3.4 table). Editing by hand follows
  the same rule as a WebAgent: a public change becomes a draft release the owner signs; a record
  change (an order note) writes a receipt; money and messages go through a card.
- Routes: `/console/{site}/apps/{webagent}/{module}` and `/…/{module}/{id}`.
- Every module has the honest empty state, a "models are down" state that still allows hand
  edits, and CSV export.

**Modules the archetypes need** (ring-0 synthetic archetypes: bakery, clinic, shop, news;
08 §3.5):

| Id | Module | Owner app | Objects | Archetypes | Status |
|---|---|---|---|---|---|
| C-PAG | Pages and releases | Editor | pages, posts, menus, releases, diffs | all | Missing (versions drawn, console.html:327-332) |
| C-CAT | Catalogue | Concierge / Cashier | products, options, prices (Assumed → Known), stock, lead times | bakery, shop | Missing |
| C-ORD | Orders | Cashier | orders, status, payments, refunds, notes | bakery, shop | Missing (cards only) |
| C-BOK | Bookings and slots | Sales / desk | slots, capacity rules, calendar, holds | bakery, clinic, lodge | Missing (slot rule card, mission.html:153) |
| C-LED | Leads and quotes | Sales | leads, quotes, follow-ups, consent records | all | Missing |
| C-REV | Reviews and mentions | Reputation | reviews, drafts, replies | all | Missing (one card, console.html:305-314) |
| C-CNV | Conversations and artifacts | Concierge | threads (untrusted), artifacts, share links | all | Missing (floor only) |
| C-APT | Appointments | Sales / desk | appointments with health-privacy defaults (device memory off, 07 §3.3) | clinic | Missing |
| C-ART | Articles and digests | Editor | articles, digest subscriptions | news | Missing |
| C-HST | Domain, certificates, backups | Caretaker | domain, DNS records, certificates, backups | all | Missing |
| C-SEC | Incidents and probes | Guardian | incidents, blocked requests, rollbacks | all | Missing |
| C-BKS | Money and books | Cashier | payouts, fees, ledger | commerce | Drawn as a table only (owner-order.html:77-86) |

- **Tags:** W5 (module host), W2 (archetype modules and sample data), W1 (WebAgent object APIs).

---

## 3. Control plane for the platform operator, agencies and resellers

### 3.0 One shell, three scopes
- **Drawn:** the scope overview (control-plane/index.html:57-83) and the shared shell built by
  `cp.js` (scope chip, scope tag, pills, dock of apps then operator WebAgents, command bar;
  cp.js:26-86). Operator dock: Fleet, Rollouts, Incidents, Customers, Money, Partners, Trust
  (cp.js:34-35). Portfolio dock: Clients, Sites, Rollouts, Billing, Team (cp.js:54-55).
- **Broken links tell the story:** Operator "Customers" points to `fleet.html#customers`, which
  does not exist; "Partners" points to the agency's portfolio page; "Trust" points to the ledger
  card on Fleet (cp.js:34-35). Portfolio "Sites", "Rollouts", "Billing" and "Team" point to
  panels on the one Clients page (cp.js:54-55). So 3 of 7 operator apps and 4 of 5 portfolio
  apps have no page.
- **Not built:** nothing in the first release (08 §10: operator WebAgents "None", tooling: nothing).

### 3.1 Drawn operator pages

#### X1 Fleet (Operator · Fleet)
- **Route:** `/operator/fleet` (fleet.html:48).
- **Purpose:** the whole fleet at a glance and the operator's decisions.
- **Components:** ops brief card; five KPI tiles (sites live, Moves in flight, edge availability
  with error budget, Ask first token p95, inference spend with forecast); cell cards (region,
  role, key figures, Vultr product chips); operator WebAgents list; runner pools with meters;
  Migration Factory stage counts with fair share; SLO table; operator ledger rows; lane with
  two-signer cards.
- **Data and source:** `agentrix-fleet` registry; SLOs from the observability backend (08 §3.6);
  Model Gateway usage reconciled with Vultr (08 §4.2); operator ledger.
- **Actions:** approve a Terraform plan (`fleet.capacity.apply`); sign as second approver for a
  ring promotion; open the incident, train or money pages.
- **States:** all cells serving; an SLO burning (fleet.html:55).
- **Status:** **Drawn** (fleet.html:52-198). **Tags:** W3.

#### X2 Rollouts · a release train
- **Route:** `/operator/rollouts/{train}` (rollout.html:56).
- **Components:** ring cards (done, live, next), wave chips, canary-versus-control bars with
  verdicts, gates list (one paused), artefacts riding the train, train timeline, owner notice
  preview, halted-train card, lane promotion card with two signers.
- **Data and source:** rollout controller; `adk-eval` ExperimentAnalyzer (CP6); signatures.
- **Actions:** sign as second approver; "Hold the train".
- **Status:** **Drawn** (rollout.html:60-176). **Tags:** W3.

#### X3 Incidents · one incident
- **Route:** `/operator/incidents/{id}` (incident.html:54).
- **Components:** SEV badge, sparkline with SLO and lever markers, content-free evidence list,
  "did this alone" lever card with Undo, timeline, drafts for people to send (status page,
  partner, owners), authority can/needs-a-person, lane card held for the commander, WhatsApp
  on-call thread.
- **Actions:** undo the cap; publish status; approve the dedicated subscription; ask for more
  evidence; WhatsApp acknowledge, show evidence, keep cap longer, open in console.
- **Status:** **Drawn** (incident.html:58-164). **Tags:** W3, W8.

#### X4 Money · a month
- **Route:** `/operator/money/{yyyy-mm}` (billing.html:55).
- **Components:** KPI tiles (MRR posted, collected, outstanding, inference cost, gross margin),
  reconciliation table with held variances and notes, payment rails with first-attempt rates,
  reminders list, MRR by plan bars, zavora-era books list, lane: invoice run card, payout card
  with two signers.
- **Status:** **Drawn** (billing.html:59-166). **Tags:** W3, W6.

#### X5 Zavora CLI session (terminal) and the owner's pack card
- **Route:** a terminal, not a console page; the owner card lands in the site's lane.
- **Components:** terminal replay (prompt, plan box, approval boxes with y/t/n keys, blocked
  hook, provenance lines), side notes, pack train strip, owner "Update · Bookings 1.4.0" card
  with capability diff and starting level.
- **Status:** **Drawn** (cli.html:55-123). **Tags:** W3, W5.

### 3.2 Drawn portfolio page

#### P1 Portfolio · Clients (agency or reseller)
- **Route:** `/portfolio/{partner}` or the white-label host (portfolio.html:60).
- **Components:** portfolio brief; KPI tiles; clients table (plan, health, waiting, spend/cap,
  your role, ring); import progress with legend and owner-blocked items; "your ring" for a theme;
  November margin; brand settings (read-only); authority can/never; lane with an Approver card
  and a "Send to client" card.
- **Data and source:** charters (roles granted per site), health scores (content-free, 08 §4.1),
  Migration Factory, `agentrix-billing` estimates.
- **Actions:** Invite a client; Import sites; approve as Approver; Send to client; Freeze
  clients (pill).
- **Status:** **Drawn** (portfolio.html:64-177). **Tags:** W3.

### 3.3 Operator pages doc 08 specifies but no concept draws

| Id | Page | What it must do (08 ref) | Priority | Tags |
|---|---|---|---|---|
| X6 | Customers: orgs and sites | Search and list orgs and sites, content-free: plan, cell, version, health, payment state, partner, lifecycle stage (Lead → Leaving) (§4.1, §2.1) | Must | W3 |
| X7 | Org detail | Org's sites, billing account, members and roles, partner link, tickets, grants, notices sent | Must | W3, W6 |
| X8 | Site detail (content-free) | Placement, platform version and pin, pack versions, release history ids, health, receipts index, holds, active support grants (`agentrix site show`, §6.2) | Must | W3 |
| X9 | Platform holds | Place and lift a hold on a site, partner, cell or the fleet; reason shown to owners; notice-page option for phishing; two people for the fleet (§2.4, §3.8, §5.3) | Must | W3, W6 |
| X10 | Break-glass | Two-person ceremony, reason, 1-hour scope, owner told at the time, postmortem link (§2.5) | Must | W3, W6 |
| X11 | Support desk | Tickets from console, email, WhatsApp; Support WebAgent drafts; escalation lines 0–3; response targets per plan (§4.3) | Must | W3, W8 |
| X12 | Support grant requests | Ask an owner for a scoped, time-boxed grant; see active grants; every access is a receipt (§2.5) | Must | W3, W6 |
| X13 | Abuse desk | Inbox desk for Vultr reports, crawler complaints, phishing, spam; evidence from receipts; actions by who may do them (§3.8) | Must | W3 |
| X14 | Incidents list | Open and past incidents by severity, cell, commander, time to mitigate (§3.7) | Must | W3 |
| X15 | Postmortem | Draft from the timeline by the SRE WebAgent; humans own conclusions and actions (§3.7) | Should | W3 |
| X16 | Status page publisher | Per cell, white-labelled per partner; human publishes SEV1/SEV2 (§3.6) | Should | W3, W8 |
| X17 | On-call: schedule, paging, escalation | Rota, acknowledgement, escalation after 15 min, quiet hours for SEV3 and below (§7.4; CP8) | Must | W3, W8 |
| X18 | Trust: operator ledger and audit verify | Hash-chained ledger browser, Vultr Audit Log reconciliation, out-of-band change alerts, `audit verify` (§4.6, §2.7) | Must | W3, W6 |
| X19 | Cells and capacity | Cell detail, pools, autoscale bounds, Terraform plan cards with digest, reaper orphan proposals, cost forecast (§3.2-3.3, §3.10) | Should | W3 |
| X20 | Migration Factory | Queue by site, partner and pool; fair shares; reservations; honest estimates (§3.4) | Should | W3, W4 |
| X21 | Release trains list | All trains, rings, halts, rollbacks; expedited trains (§3.5) | Should | W3 |
| X22 | GitOps drift | Hourly plan-versus-actual and audit-log drift as cards (§6.3) | Should | W3 |
| X23 | Packs: channels and review | Pack versions per channel, capability diffs, certification reviews, signing status (§6.5, §4.4) | Should | W3 |
| X24 | Fleet learning | Gated aggregates, candidate playbook revisions, review and sign (§3.5) | Could | W3, W9 |
| X25 | Model routing table | Purpose → model and fallback per cell; changes are receipts and owner-visible (§3.5; R11.4) | Should | W3, W1 |
| X26 | Plans and pricing | The plan catalogue (`plans.toml`) as a read view with change history; edits go through GitOps (§4.2) | Should | W3, W6 |
| X27 | Invoice run preview | Drill into the run: per-org invoice previews, held variances, eTIMS state (§4.2) | Must | W3, W6 |
| X28 | Dunning and suspension | Orgs by reminder day, proposed suspensions for a human, closure notices (§4.2) | Must | W3, W6, W8 |
| X29 | Payouts | Partner statements, revenue share, two-signer payout runs (§4.2) | Should | W3, W6 |
| X30 | Revenue and cost to serve | MRR, NRR, margin per site and plan, collection rate, partner share, method beside each number (§4.2, §3.10) | Should | W3, W9 |
| X31 | Compliance | Access reviews, stale grants, retention checks, sub-processor register (§4.6) | Should | W3, W6 |
| X32 | People and platform roles | Zavora staff, `op.*` roles, devices, approval-chain membership (§2.3, §5.3) | Must | W3, W6 |
| X33 | Operator WebAgents | Each operator WebAgent's app window: ladder per job, grants, budget, receipts (§5.2-5.3) | Should | W3 |
| X34 | Operator charter | Read view of the signed operator charter, approval chains, red lines; changes need two directors (§5.3) | Should | W3 |
| X35 | Partners: list and detail (operator side) | Programme tier, contract, payout details state, health, sites, certification (§4.4) | Must | W3 |
| X36 | Partner onboarding | Checklist: verification, contract, payout details to the Effects Gateway, brand, domain, WhatsApp sender, import plan, canary sites (§4.4) | Should | W3 |
| X37 | Approval landing for CLI and links | The page a CLI QR code or `/d/{card}` opens: the card, the sheet, one approver or two (§6.2) | Must | W3, W6 |

### 3.4 Portfolio pages specified but not drawn

| Id | Page | What it must do | Priority | Tags |
|---|---|---|---|---|
| P2 | Sites | All client sites with state, version, ring membership | Must | W3 |
| P3 | Rollouts (agency ring) | Packs and themes on the agency's ring, opted-in sites, per-site candidate releases, owner cards | Should | W3 |
| P4 | Billing | Wholesale bill from Zavora; reseller invoices to clients through `agentrix-billing`; client prices (§2.2, §4.4) | Must | W3, W6 |
| P5 | Team and brand | Partner people and roles (`partner.*`); white-label settings: domain, logo, tokens, email sender, WhatsApp sender, status page, footer credit (§2.6) | Must | W3, W7 |
| P6 | Invite a client | Invitation that leads the owner to prove the domain, grant roles and choose a plan (08 §8 J1) | Must | W3, W4 |
| P7 | Import planner | Plan a bulk import with Partner Success, reserve capacity, see estimates (§3.4) | Should | W3 |
| P8 | Freeze clients | Fan-out freeze over grants, with the list of sites it touches | Should | W3, W6 |
| P9 | API and webhooks | Scoped, expiring tokens; signed webhooks `site.live`, `decision.held`, `invoice.issued`, `incident.update` (§6.2) | Should | W3 |
| P10 | Referral dashboard | Referred sites and revenue share, for referral partners (§2.2) | Could | W3 |
| P11 | Partner WebAgents | The agency's Chief WebAgent, Release, Support, Billing (§5.5) | Could | W3 |

### 3.5 Self-host pages (Marketplace)

| Id | Page | What it must do | Priority | Tags |
|---|---|---|---|---|
| S1 | First-run wizard | Ask for a scoped Vultr API key, store it in the local Effects Gateway, create the sandbox VM; keep migration off until it exists (§4.5) | Should | W3 |
| S2 | Updates | Channel, signature check, schema window, `agentrix upgrade` result | Could | W3 |
| S3 | Lift to Zavora Cloud / take it home | Export, import, receipt chain verified, DNS held for the owner (§4.5, §8 J7) | Could | W3, W9 |

---

## 4. Gaps: what a production product needs that no concept draws

Priority: **M** must have before paying customers, **S** should, **C** could. Tags as above.

### 4.1 Identity and access

| Gap | Why | P | Tags |
|---|---|---|---|
| Account recovery: lost passkey, lost authenticator | Passkey-first with no magic links (02 §8) means a lost phone locks the owner out of approvals. Needs recovery codes at enrolment and a staffed, audited recovery path | M | W6 |
| Recovery codes screen | Shown once at TOTP enrolment; regenerate in settings | M | W6 |
| Devices and passkeys: add, rename, remove | Trust centre lists devices read-only (trust.html:202-203) | M | W6 |
| Active sessions and sign out elsewhere | Sessions are device-bound (07 §5.2); owners need to end one | M | W6 |
| Change email, change password, reset TOTP | Account basics | M | W6 |
| Email confirmation | Briefs and receipts go to email (register.html:16) | M | W6, W8 |
| New-device and new-passkey alerts | Security notice when credentials change | M | W6, W8 |
| Step-up for sensitive settings | Adding a passkey, granting a role, exporting, changing payout details | M | W6 |
| Social sign-in (Google, LinkedIn, GitHub) | Drawn (owner-account.html:51-55), Won't for the first release | S | W6 |
| Session-expired and signed-out states | Copy and redirect | S | W6, W7 |
| Account switcher (one person, several memberships) | 08 §2.3: an agency employee who owns a business has two memberships | S | W6, W3 |

### 4.2 Team members and roles (owner side)

| Gap | Why | P | Tags |
|---|---|---|---|
| People page: invite, roles (owner, manager/approver, viewer, billing), remove | 07 §5.6 defines the roles; no page exists | M | W6, W5 |
| Invitation acceptance flow | Teammate signs up passkey-first and lands on the site | M | W6, W4 |
| Agency role grant and revoke | Drawn as one row with Revoke (trust.html:161); granting is not drawn | M | W6, W3 |
| Which cards each role can decide; approval chains with distinct people | `check_chain` (07 §5.6) | S | W6, W5 |
| Ownership transfer (sell the business) | The org owns the site (08 §2.1) | S | W6 |
| Billing delegate | 08 §2.5 | S | W6 |

### 4.3 Notifications and channels

| Gap | Why | P | Tags |
|---|---|---|---|
| Notification settings: channels per kind, quiet hours, time zone, P1 channels | Quiet hours drive Night Time and routing (07 §3.5) | M | W8, W5 |
| Daily card cap setting | 07 §3.5 says the owner changes it in the Trust centre | S | W5 |
| WhatsApp pairing and unpairing | Paired device per person (07 §4.4); not drawn | M | W8, W6 |
| Web push permission and doorbell | Lock-screen copy drawn (phone.html:101-103); permission flow not drawn | S | W8 |
| Email preferences and unsubscribe | Legal for non-transactional mail | M | W8 |
| Language (Swahili first, F8) | Strings are English constants today | S | W7, W8 |

### 4.4 Billing and plan

| Gap | Why | P | Tags |
|---|---|---|---|
| Invoice detail and download (PDF, eTIMS number) | Drawn as a table row only (owner-billing.html:48) | M | W6 |
| Payment method change | Switch rail, update card on Paystack's page | M | W6 |
| Failed payment and dunning states in the console | Day 0/3/7 reminders, read-only at day 14 (08 §4.2) | M | W6, W8 |
| Upgrade flow with prorated charge | Described only (08 §4.2) | M | W6 |
| Plan gating in the Store and elsewhere | Store offers Business features on Essential (store.html:67, 88) | M | W5, W6 |
| Raise the AI cap card | 08 §4.2 | M | W6, W5 |
| Currency and tax display | USD and KSh mixed across screens (§6.3) | M | W6, W7 |
| Receipts for the subscription itself in the Trust log | Plan changes are effects with receipts | S | W6 |

### 4.5 Data rights

| Gap | Why | P | Tags |
|---|---|---|---|
| Export: progress, failure, bundle verify | Drawn as ready only | M | W9 |
| Erasure request from the owner (a customer asks to be forgotten) | Drawn as progress (trust.html:168-173); the request form is not drawn | M | W9 |
| Account deletion and site closure | Close the account, destroy the data key (03 §5), confirm with step-up | M | W9, W6 |
| Data retention settings (conversation retention 90 days default, 03 §5) | Owner-configurable per 03 | S | W9 |
| Residency disclosure at sign-up | 01 §11.3 | M | W6, W7 |
| Terms, privacy notice, DPA acceptance | Legal | M | W6 |

### 4.6 The site and its records

| Gap | Why | P | Tags |
|---|---|---|---|
| WebAgent app window (C11) and object modules (§2.20) | The owner must edit pages, prices, orders and bookings by hand | M | W5, W2 |
| Page editor and release diff view | "See the diff" is a button everywhere; no diff is drawn except on the walk-the-site mini page (live.html:164-175) | M | W5 |
| Business details: name, hours, address, contact, currency | Used by briefs, desks and invoices | M | W5 |
| Domain and DNS page after go-live | Caretaker's object; renewals, records, certificate | S | W5 |
| "Edit" on a decision card | Drawn on most cards; never specified | M | W5 |

### 4.7 Messages the owner receives (catalogue to write)

None of these is drawn except the WhatsApp brief and the on-call thread. Each needs an email and,
where noted, a WhatsApp or push version. Tag W8 unless noted.

| Message | Trigger | Channels | P |
|---|---|---|---|
| Welcome; confirm your email | Account created | Email | M |
| New device or passkey added; authenticator reset | Credential change | Email, push | M |
| Preview reminder (within consent) | Preview day 5 of 7 | Email | S |
| Claim reminder | Claim pending 24 h | Email, WhatsApp | S |
| Move started, waiting for you, stopped | Move state | Push, WhatsApp | M |
| Go-live card waiting | Card raised | Push doorbell | M |
| You're live; undo window; fallback retired in 7 days | Go-live, day 23 | Email, WhatsApp | M |
| Morning brief; weekly review | 07:00; Monday | WhatsApp, email | M |
| A decision is waiting; a card expires within the hour | Lane, expiry | Push, WhatsApp | M |
| P1 alarm (site down, payment anomaly, incident) | `AlarmRouter` P1 | Push, WhatsApp, call later | M |
| Frozen; unfrozen | Freeze | WhatsApp, email | M |
| Platform notice; platform hold with reason and appeal | Operator action | Email, console | M (W3) |
| Support grant request; access started and ended | Support | Email, console | M (W3, W6) |
| Agency invitation; role granted or revoked | Partner | Email | M (W3) |
| Plan starts today (first charge) | Go-live | Email | M (W6) |
| Invoice issued; payment received | Billing | Email | M (W6) |
| Payment failed: day 0, 3, 7; read-only at day 14; closure notice day 30 | Dunning | Email, WhatsApp | M (W6) |
| Plan changed; cancellation confirmed; site closes on date; data deleted | Billing | Email | M (W6) |
| Export ready (7-day link); erasure complete | Data rights | Email | M (W9) |
| Data incident notice within 72 h | 03 §7 | Email | M (W6) |
| Budget cap reached: WebAgents sleeping | Model Gateway | Push, WhatsApp | M |
| Mission report | Monday | Brief, email | S |
| Pack update available | Train ring 1+ | Console card, email digest | S (W3) |

### 4.8 States no concept draws

| Gap | Where it applies | P | Tags |
|---|---|---|---|
| Empty states for a new site: no visitors, no receipts, no missions, no rules | Every console page | M | W5, W7 |
| Loading and skeletons for SSE pages | Today, Move, Front of house | S | W7 |
| Offline (PWA): read cached brief and cards; approvals blocked with a clear line | Phone | M | W5, W7 |
| Models unavailable: "Resting · budget reached" or provider down; hand edits still work (07 §1.12, J13) | Shell, desks, brief | M | W5, W1 |
| Read-only console during suspension or after cancel | Whole console | M | W5, W6 |
| Platform hold in force | Shell banner, Trust centre | M | W5, W3 |
| Error pages: 404, 403 (wrong site), 500 with a reference | All | M | W7 |
| Card expired; card superseded by a newer version | Decisions | M | W5 |
| Partial failure: go-live probe failed and the pointer went back (launch/mod.rs:13-16) | Go-live | M | W4, W5 |

### 4.9 Other

| Gap | Why | P | Tags |
|---|---|---|---|
| Help and "Ask your Chief WebAgent" about the product (support line 0, 08 §4.3) | Owners need how-to answers in the console | S | W5, W3 |
| Contact support and ticket view for owners | 08 §4.3 lines 1–2 | M | W3, W5 |
| Accessibility statement and keyboard map | Gestures page has keys (gestures.html:88); the shell's keyboard parity (07 §3.8) is undrawn | S | W7 |
| Onboarding for non-WordPress sites (Wix, Squarespace, Shopify) | 01 names them; only WordPress exists | S | W4, W1 |
| Agency-led onboarding: owner receives an invite, not an assessment | 08 §8 J1 | M | W4, W3 |

---

## 5. Design system extraction

### 5.1 Where the tokens and components live today

| File | Role | Notes |
|---|---|---|
| `concept/styles.css` | Base tokens and primitives | Colour, type, space, radius, shadow, focus, ease (styles.css:4-65); buttons, chips, badge, sealed chip, meters, segmented, frames, avatars (127-223) |
| `concept/console/console.css` | The OS shell and owner components | `--os-*` surface tokens, motion `--t-fast`/`--t-med`, presence colours `--pm-wait`/`--pm-ok` (7-23); Night Time theme (66-86); shell regions, presence marks, team mark, field cards, wall, timeline, lane, command bar, overlays, passkey sheet, ladder, live floor, thread, pointer, Store, phone, Trust, fleet cards, hub (88-537) |
| `concept/control-plane/cp.css` | Scope additions | Scope markers `--op`, `--pf`, terminal tokens (5-17); KPI tiles, rows, tables, dots, operator avatars, team rows, decision card with inline sheet and signers, authority boxes, timeline, rings, canary bars, sparkline, cells, pools, WhatsApp, terminal, explainers (61-237) |
| `concept/journeys/owner.css` | Owner journey screens | Fields, providers, OK and wait lines, TOTP, plans, budget, rails, cards, money table, timeline, WhatsApp (6-112) |
| Page-local `<style>` blocks | Components never promoted | mission (`.stats`, `.contract`, `.method`), live (`.composer`, `.whisper`, `.site-mini`, `.voice-panel`), trust (`.jobrow`, `.fixed-rules`, `.earned`), phone (`.p-card`, `.p-stack`, `.p-sheet`, `.effect-box`, `.wa`), gestures (`.gbar`, `.stopwin`, `.camtile`), rollout (`.gates`, `.artefacts`, `.waves`), incident (`.lever`, `.sev`, `.draft`), billing (`.hbar`, `.payouts`, `.rail`), cli (`.diff`, `.train`) |
| `crates/agentrix-console/static/app.css` | The first release, shipping | Own copy of the tokens plus a dark theme (app.css:5-52); system fonts (app.css:3, 17-19); 44 px targets; pill buttons; static `.mark` (app.css:137); bottom/side nav (190-204) |

### 5.2 Tokens the kit must define

- **Colour, brand:** indigo `#38356d`, indigo-900/700/100/50, coral `#de4060`, coral-strong
  `#b72d50`, coral-50 (styles.css:8-15).
- **Colour, neutral:** ink, ink-2, muted, line, line-strong, paper, surface, surface-2
  (styles.css:18-25). Pick one `--muted`: concepts `#656479` (styles.css:20), the first release `#5f5e73`
  (app.css:9, darker for AA). Keep the first release's.
- **Colour, semantic:** ok, wait, risk, info, sealed, each with a background (styles.css:28-37);
  presence wait `#b26b00` and ok `#1f8a57` (console.css:21-22) must merge with ok/wait or be
  named as presence tokens.
- **Scope markers:** operator `--op` `#1f1d3d`, portfolio `--pf` `#0f6b62` with `--pf-bg`
  (cp.css:6-9). Site scope has no marker; give it one (the site's own logo tile, as drawn,
  console.css:92).
- **Themes:** light (default), system dark (the first release only, app.css:26-52), Night Time (console
  only, console.css:66-86). Decide whether Night Time is the dark theme applied by schedule or a
  third theme. Recommendation: one dark theme; Night Time = dark theme + timeline view + quiet
  lane.
- **Type:** Manrope display, DM Sans text, JetBrains Mono (styles.css:40-42); scale 12-56
  (43-52). Concepts load Google Fonts; journeys add Fraunces; the team mark uses Young Serif
  (console.css:167). The first release forbids third-party font requests (app.css:3). Decision: self-host
  Manrope, DM Sans and one mono, or ship system fonts. Never load fonts from a third party in
  the console.
- **Space, radius, shadow, focus, motion:** s1-s8, r-sm/r/r-lg/r-xl, shadow 1-3, focus ring,
  ease (styles.css:55-64); fast/med durations (console.css:19-20). The first release dropped r-sm, r-xl,
  s8 and shadow-3.
- **Hit targets:** 44 px minimum (the first release, app.css:92; WCAG 2.5.8). Concepts use 34-42 px
  (styles.css:136, 149).
- **Presence values shared with the visitor site (V1, 07 §3.5):** mark sizes 20/28/40; ring −4 px
  2 px breathing 2.2 s; working line 2 px, 6 px below, 1 s; answering .45; waiting dot 9 px;
  resting greyscale .55; reduced motion off.

### 5.3 Components the shared kit must contain

| Kit component | Source today | Used by | Notes |
|---|---|---|---|
| Button (primary, secondary, ghost, danger, sm, lg, wide) | styles.css:134-152; app.css:92-98 | all | Pick one radius (12 px concept vs 999 px the first release) and one primary colour (indigo vs indigo-900) |
| Chip (neutral, ok, wait, risk, info, coral, outline, sealed) | styles.css:155-162; app.css chip | all | |
| Badge | styles.css:163 | dock, tabs, lane | |
| Card, tight card, raised card, accent card | styles.css:127-130; console.css:196-200 (`fcard`); owner.css:65 (`o-card`) | all | Three card classes for one thing |
| Field, hint, error, ok line, wait line | owner.css:19-29; app.css field/okline/waitline | onboarding, settings | |
| Tabs | app.css tabs; journeys `j-tabs` | claim, register, billing | |
| Meter and mini meter | styles.css:179-182; console.css:113-114 | budgets, parity, pools | |
| Segmented control | styles.css:185-187 | v0.1 dial, starting level | |
| **Ladder** (five positions, locked with reason, lowered, ceiling marker) | console.css:329-335; app.css `.levels` (180-185) | WebAgents, Store, Trust, go-live | The first release renders a vertical list; concept a segmented ladder. One component, two layouts |
| **WebAgent mark** (squircle, presence states, keyline on dark) | `.avatar` letters (styles.css:214-223), `.os-mark` wrapper (console.css:131-151), `.j-mark` generator (journeys.js), `.mark` (app.css:137-142) | everywhere | Four implementations. Converge on ADR-031's living mark with one JS build golden-tested against the Rust build (07 §9.3) |
| **Team mark** (Chief WebAgent) | `.orb` (console.css:158-184) | command bar, brief, phone | Hard-codes Kilele's ink and "K" (console.css:158, 167); must take the site's tokens |
| State line | console.css:145-151 | dock, cards, monitor | |
| Status bar, site/scope chip, scope tag, pills (time, budget, trust, camera, freeze, hold) | console.css:89-114; cp.css:33-40 | shell, all scopes | |
| World pager | console.css:94-99 | site scope only | |
| Dock and app icon | console.css:117-128 | shell | Phone app grid reuses it (console.css:423-425) |
| Lane and lane card (`ldec`), quiet note, why box | console.css:246-264 | shell, missions, live | |
| **Decision card** with effect box, risk line, actions, done line | console.html `.decision`; console `ldec`; cp.css:111-137; owner.css:65-77; the first release decision.html | all scopes | Five versions. One component with sizes (lane, full) |
| **Approval sheet** (passkey, code step-up, two signers) | console.css:302-326 (overlay); cp.css:126-137 (inline); phone `.p-sheet`; the first release `.sheet` (app.css:187) | all scopes | One component, overlay on desk, bottom sheet on phone; signers row for two-person cards (cp.css:121-125) |
| Command bar and plan preview | console.css:267-284 | shell | |
| Overlay window | console.css:287-292 | monitor, consent | |
| Process table | console.css:293-300 | Activity monitor | |
| Timeline (three versions: `.tl`, `.timeline`, `.o-tl`) | console.css:235-243; cp.css:148-158; owner.css:83-87 | Night Time, incidents, cancel | Merge |
| Signal pair and wall | console.css:222-232 | Back office | |
| Receipt row with Verify expansion | console.css:522-525, 454-456; app.css receipt row macro (macros.html:41-48) | Trust, Today | |
| Grant row, fact row, freeze panel | console.css:439-453 | Trust | |
| Listing card, can/never, effect tag, stepper | console.css:373-393 | Store, pack card | |
| Thread messages (visitor, desk, owner, system) with AI and person tags | console.css:348-360 | Live | Shared with the visitor site's tags |
| Pointer ring | console.css:364-366 | Walk the site | Same geometry as the visitor site's `.pointed` |
| Phone frames, tab bar, notification, widget | styles.css:197-205; console.css:400-429 | concepts only | Not product components; the PWA is the product |
| KPI tile, rows, data table, dot | cp.css:61-89 | control plane, portfolio | Owner metrics need the same tile |
| Rings, waves, canary bars, sparkline | cp.css:161-187 | Rollouts, incident | Sparkline must be server-rendered SVG (07 §8) |
| Cells, pools | cp.css:189-205 | Fleet | |
| Authority can/never | cp.css:140-145 | portfolio, incident | Same idea as Store can/never (console.css:380-386): merge |
| WhatsApp thread (render of outbound messages) | owner.css:90-95; cp.css:208-216; phone.html:57-62 | concepts | Three copies; the product needs real message templates, not a drawing |
| Terminal block | cp.css:219-239 | CLI concept | Docs only |
| Sealed-room chip, tick counters, frame | styles.css:166-176; macros.html:15-22; app.css ticks/frame | Assess, Move | |
| Finding with evidence | macros.html:24-39; app.css finding | Assess, quarantine | |
| Plan card, budget slider, rail radio card, money table, TOTP block | owner.css:33-82 | plan, billing | |

### 5.4 Where the console and the control plane diverge

- **Theme attribute:** console uses `data-time="day|night"` (shell.html:41); control-plane pages
  set `data-shift="day"` (fleet.html:49), which no stylesheet reads. Operators get no Night Time.
  Decide whether on-call night views use the dark theme.
- **Decision card:** console lane cards are compact (`ldec`) and open an overlay sheet; control
  plane cards are full cards with an inline sheet and a signers row (cp.css:111-137). The
  operator card has two-person signing; the owner card does not need it but agencies' approval
  chains do (07 §5.6).
- **Marks:** owner WebAgents use site-coloured gradients (styles.css:215-223); operator WebAgents
  use their own gradients plus a ring (cp.css:92-103). Both are letter avatars, not ADR-031 marks.
- **Field content:** console field is cards in a grid; control plane field is container-query
  layouts with KPI tiles and tables (cp.css:268-281). The kit needs both: card grid and dense data.
- **Shell building:** console pages write the shell in HTML; control-plane pages build status bar,
  dock and command bar from a JS table (cp.js:26-86). The product should render all three scopes
  server-side from one partial with scope data.
- **Name collisions to fix in the kit:** `.sheet` (cp.css:126, app.css:187, console.html),
  `.steps` (console.css:211, assess.html player, app.css Move steps), `.room` (move.html rooms,
  console.css:339 live floor), `.rail` (move.html stage rail, billing.html rails, owner.css
  `.o-rail`), `.draft` (live.html, incident.html), `.timeline`/`.tl`/`.o-tl`, `.wa` (three files).

### 5.5 What the kit must ship

1. `tokens.css`: one file, light and dark, scope markers, presence tokens, generated for both
   the console and the visitor site's shared values.
2. Component CSS with stable class names (BEM-like prefix, e.g. `ax-`), documented with states.
3. minijinja macros for each component (the first release already has `mark`, `finding`, `receipt_row`,
   `ticks`; templates/macros.html:1-49), so pages in every scope render the same markup.
4. JS islands: approval sheet (passkey and code), SSE live regions, ladder, command bar, gesture
   HUD, verify. No framework.
5. A component gallery page in the console for review, light and dark, at 375 and 1280 px.
6. The copy lint and glossary (§6.4), owned by W7 (voice.md:77-88).

---

## 6. Copy review: "human, not AI slop"

### 6.1 Verdict

The concepts are written better than most product copy: short sentences, numbers with units,
honest gaps. The problems are a small set of habits that recur: slogan headlines and couplets,
"not X but Y" lines, cute labels that hide the function, unexplained scores, internal names on
owner screens, narrator copy that leaks into the product, and inconsistent facts across screens.
Concept page headings are presentation copy for judges; the product must not inherit them.

### 6.2 Strings to rewrite

| # | String (file:line) | Problem | Rewrite |
|---|---|---|---|
| 1 | "a website that answers every customer, sells, publishes, defends and maintains itself" (index.html:96) | Five-verb list; "maintains itself" is a vague claim | "Your site answers customers, takes orders and keeps itself up to date. It asks you before anything that matters." |
| 2 | "Earn trust in three minutes, then every morning." (index.html:139) | Slogan | "See your own site checked in three minutes. Then get a short report every morning." |
| 3 | "Printed on the pricing page. Enforced in code." (index.html:186) | Couplet | "Our promises to you, and how the platform enforces each one" |
| 4 | "Roles with mandates, limits and receipts." (index.html:200) | Rule of three, abstract | "What each WebAgent does, and what it may never do" |
| 5 | "Agentrix OS: the operating system for a website that works." (console/index.html:53) | Slogan; "works" says nothing | "The owner console: your WebAgents, your decisions and your site in one place" |
| 6 | "Nine ideas that make it an OS, not a dashboard." (console/index.html:74) | "Not X" construction | "Nine ideas behind the console" |
| 7 | "Calm by default, an OS underneath." (console/index.html:104) | Slogan | "On a phone it is a short list. On a desk it shows more." |
| 8 | "Your website is a team. This is its operating system." (shell.html:33) | Couplet, metaphor | "Your WebAgents, what they are doing, and what needs you" |
| 9 | "Proof you can check. Control you can feel." (trust.html:61) | Couplet; "feel" is vague | "Trust centre: what your WebAgents may do, what they did, and the switch to stop them" |
| 10 | "Add a WebAgent like an app. Sign what it may do." (store.html:58) | Couplet | "Add a WebAgent. Choose what it may do, then approve." |
| 11 | "A leash, not a licence" (mission.html:199) | Cute label, "not X" | "A mission never raises a WebAgent's authority" |
| 12 | "'Didn't work' is a section, not an afterthought." (mission.html:198) | "Not X" construction | "Every report lists what didn't work, with sources." |
| 13 | "The whole OS, in two minutes a day." (phone.html:89) | Slogan, unmeasured claim | "On your phone: the brief, your decisions, and who's on your site" |
| 14 | "The doorbell, not the letter." (phone.html:106) | Cute label hides meaning | "Notifications say only that something is waiting." |
| 15 | "Hands-free, safely." (gestures.html:86) | Slogan | "Hand gestures: move between cards and stop everything without touching the screen" |
| 16 | "Your hands, your light, your counter." (gestures.html:237) | Rule of three | "Calibrate gestures for your hands and your lighting" |
| 17 | "Migration as a show, not a ticket." (move.html:131) | "Not X" construction | "Watch your site move step by step." |
| 18 | "Zavora sells running it well, not keeping you." (owner-billing.html:90) | "Not X" construction | "You can take your site and run it yourself at any time." |
| 19 | "Your question matters, so I won't guess" (live.html:111) | Chatbot filler in a desk reply | "The bakery's pages don't say whether they make gluten-free tiers. I can ask the bakery now. Shall I?" |
| 20 | "All walls holding" / pill "Walls holding" (trust.html:85; shell.html:52) | Jargon that hides the function | "Protections on · last check 2 min ago" |
| 21 | "Across the wall today" (shell.html:106) | Metaphor needs a lesson first | "What visitors' conversations triggered today (no visitor text is passed on)" |
| 22 | "Earned it?" (console.html:320; trust.html:112) | Cute; doesn't say what is being asked | "Let Sales send follow-ups to consenting leads without asking?" |
| 23 | "Low risk · 0.08", "Medium risk · 0.34" (console.html:279, 294) | Unexplained score | "Low risk: returning customer, usual amount" (drop the number; keep reasons) |
| 24 | "Overnight your site welcomed 41 visitors" (console.html:245; index.html:114) | Inflated verb | "Overnight: 41 visitors, 9 questions answered (2 in Swahili)" |
| 25 | "wrote your brief from 31 recorded events. Nothing was invented." (shell.html:150) | Defensive filler | "Written from 31 recorded events (see them)" |
| 26 | "Asleep costs nothing" (console.html:319; shell.html:140) | Overclaim; hosting still costs | "Asleep: no AI spend" |
| 27 | "If anything gets worse, it rolls back by itself." (rollout.html:150) | Vague promise | "If our checks see errors rise, the update is undone automatically and we tell you here." |
| 28 | "Every client, one calm view." (portfolio.html:55) | Slogan | "Clients" |
| 29 | "The SRE WebAgent was already on it." (incident.html:49) | Story headline | "INC-0142: slow answers in ams-1, 03:12" |
| 30 | "Awareness on WhatsApp. Authority in the console." (incident.html:167) | Couplet | "WhatsApp can acknowledge and ask for evidence. Approvals happen in the console." |
| 31 | "Metered at the source. Billed only as approved." (billing.html:50) | Couplet | "Usage, invoices and payouts for November" |
| 32 | "Publishing needs a pack signed by a person and a release train. Agents propose; pipelines dispose." (cli.html:80) | Slogan inside an error message | "Blocked: publishing to stable needs a person to sign the pack. Open a pull request instead." |
| 33 | "Wanjiru watches, or goes back to the oven." (owner-move.html:28) | Narrator copy; fine in a concept, wrong in product | Product: "You can leave this page. We'll tell you when the go-live card is ready." |
| 34 | "Agentrix OS, rethought for a business: calm, phone-first, and built around one question: what needs me?" (console.html:190) | Colon chain, rule of three | "What needs you, on your phone, in a couple of minutes a day." |

### 6.3 Facts that disagree across screens

These read as invented data even when every screen is fictional. The product will make the same
mistakes unless one source feeds every surface.

| Fact | Screen A | Screen B | Fix |
|---|---|---|---|
| Sunday delivery | "Yes. On Sundays we deliver…" (assess.html:428) | "Your site doesn't say. It lists Monday to Saturday collection only" (owner-discover.html:60) | One sample site, one answer (W2) |
| Urgent findings | "2 urgent" (assess.html:210) | "1 urgent" (owner-discover.html:48) | Same |
| Plan and price | "Essential plan from KSh 3,700/month" (assess.html:328) | "$29 a month" (owner-plan.html:44) | Currency rule (below) |
| Kilele's plan | Commerce (owner-plan.html:51) | Business (portfolio.html:90); Essential (store.html:67) | One plan per sample |
| Agency role on Kilele | Editor (trust.html:161) | Approver (portfolio.html:90) | One role |
| AI budget unit | KSh 1,500 a month (console.html:234) | $30 a month (owner-day1.html:46); USD daily limit (view.rs:484) | Budget in the site's currency; monthly cap; daily limit is an internal guard |
| Go-live release | v12 (gestures.html:126; console.html:331) | v1 (owner-golive.html:38; move.html:305) | First release is v1 |
| Who holds orders | "Cashier asks" (console.html:279) | "Orders asks" (owner-order.html:35) | The Orders desk holds the request; the Cashier takes payment (07 §7.1 O8) |
| Platform version | "Platform update 3.4" (trust.html:152) | "2026.11.3" (rollout.html:62), "Platform release 0.4.3" (rollout.html:127) | One version scheme shown to owners |
| Screen count | "Six screens" (console/index.html:7, 88) | "Seven screens" (console/index.html:89); "1 of 6" (shell.html:32) | Seven |
| Sales jobs | "4 jobs" (store.html:88) | three jobs (store.html:142-150) | Three |
| Ladder words | "Do it, tell me" (store.html:143) | "Do it & tell me" (console.html:396), "Do it and tell me" (pages/map.rs) | One set (§6.4) |
| Top step of the ladder | "Within budget" (store.html:143) | "Handle it within budget" (pages/map.rs:252); "Handle it" (console.html:450) | One label |
| Quarantine word | "Left behind" (move.html:219) | "2 items quarantined" (gestures.html:129) | "Left behind" for owners |

### 6.4 Copy rules for the console and the control plane

These add to `specs/platform/voice.md`; where they overlap, voice.md wins.

1. **Headings name the thing on the page.** "Decisions", "Christmas cakes mission", "INC-0142".
   No slogans, couplets or story headlines in the product. Concept headings are not product copy.
2. **No "not X but Y", no "X. Y." couplets, no three-item lists of qualities.** (voice.md:54-57)
3. **Labels say the function.** A pill that freezes says "Freeze". A status says what was checked
   and when ("Release v14 verified 2 min ago"). Metaphors (walls, worlds, Night Time) are allowed
   only as names of features the owner has been shown, and never alone on a control.
4. **Every number has a unit, a window and a source.** Link it to the rows or receipts behind it
   (07 §8). No bare scores; replace risk numbers with reasons.
5. **One currency per site.** Show amounts in the site's currency. Show another currency only
   where money actually moves in it, and say the rate ("USD 40.70 at KSh 129.00",
   owner-order.html:40). Plans priced in USD say so and show the local amount beside them.
6. **Internal names stay behind "Details".** Fingerprint hashes, signal ids (`lead_captured`),
   charter versions, ring numbers, job ids (`nurture`) and principal URNs do not appear in the
   main line of an owner screen. The operator scope may show them.
7. **One verb per act.** Owners "approve"; the receipt says how: "with your passkey on iPhone" or
   "with an authenticator code". Use "signed" only when a passkey signature exists (07 §5.2).
8. **Glossary, fixed:** WebAgent; Chief WebAgent; desk; Front of house; Back office; quiet hours
   (feature name: Night Time); Freeze (voice: "stop everything"); platform hold; decision;
   receipt; release; left behind; standing rule (owner word; "grant" is internal); mission; AI
   budget; ladder: Watch, Suggest, Ask me first, Do it and tell me, Handle it within budget, Undo
   only. W7 publishes it beside 06 §4.8.
9. **Desks speak plainly.** State the gap and offer one action. No "great question", no "your
   question matters", no apologies (voice.md:52-53).
10. **The product talks to the owner, not about them.** Narrator copy ("Wanjiru watches") is for
    concept pages only.
11. **Promises are exact.** "Undo for 30 days", "nothing is charged until go-live". Never "it
    rolls back by itself" without the condition.
12. **Errors and blocks say what to do next**, including in the CLI and the operator console.
13. **Times in the viewer's zone, one format per surface** (voice.md:36-37). Operator views show
    the zone ("03:12 EAT"), as the concepts do.
14. **Facts come from one sample source.** Sample sites, seed data and concept copy draw from the
    same fixture data (W2), so screens never disagree.

---

## 7. Workstream index

| Tag | Items in this document |
|---|---|
| W1 WebAgent runtime | Preview build and Ask on preview (O1.5-O1.6); Move pipeline states (O5.x); presence feed, process table, plan preview, missions, takeover, voice (C2-C5, C10, C17-C21); object APIs for modules (§2.20); models-unavailable state (§4.8) |
| W2 Sample websites | Archetype modules and sample data (§2.20); one consistent sample story (§6.3); export help for non-WordPress hosts (§4.9) |
| W3 Control plane | All of §3 (X1-X37, P1-P11, S1-S3); platform notices and support access source (C31); agency-led onboarding (§4.9) |
| W4 Onboarding and owner journey | O1.1-O10.2; DNS step for go-live (O6.1); agency invitation path |
| W5 Owner console | C1-C44; module host (§2.20); empty, offline, read-only and error states (§4.8); settings home (C44) |
| W6 Identity, payments and trust | Sign-up, sign-in, recovery, devices, sessions, roles (O2.x, §4.1-4.2); approval sheet (C8); receipts (C27); standing rules (C29); Freeze/unfreeze (C30); plans, payments, invoices, dunning (O3, O8-O9, §4.4); terms and residency (§4.5) |
| W7 Design system and voice | Kit (§5); glossary and copy rules (§6.4); rewrites (§6.2); error pages and loading states; component gallery |
| W8 Channels and notifications (new) | WhatsApp, email, push, voice (C34, C39-C41); message catalogue (§4.7); notification settings and pairing (§4.3); on-call paging (X17) |
| W9 Data, metrics and data rights (new) | Metric catalogue behind every number; server-rendered charts; export and erasure (O9.4, C32, §4.5); revenue and cost views (X30) |

---

## 8. Decisions to bring to James (through the coordinator)

1. **Journey order:** keep account before plan (§1).
2. **Currency:** plans in USD with local amounts beside them, budgets and briefs in the site's
   currency (§6.4 rule 5).
3. **Night Time and dark theme:** one dark theme applied on a schedule, or a separate Night Time
   theme (§5.2).
4. **Fonts:** self-host Manrope and DM Sans, or ship system fonts as the first release does (§5.2).
5. **Button shape and primary colour:** concept (12 px radius, indigo) or the first release (pill,
   indigo-900) (§5.3).
6. **Pre-account preview:** build a public-pages preview before the claim (O1.5), or keep the first release's
   order (report, then claim, then Move).
7. **Leaving items behind:** default with no signature; only release is signed (O5.3).
8. **Recovery:** what proves identity when an owner loses both passkey and authenticator (§4.1).
