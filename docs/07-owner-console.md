# Agentrix WebAgent Platform: The owner console (Agentrix OS)

Status: design v0.1, 7 Nov 2026. It supersedes the console IA in
[01-product.md §4.1](01-product.md#41-the-owners-console-agentrix-os-rethought-for-a-business)
and keeps that section's promise of two minutes a day on a phone.

**Companion documents:**
- [02-architecture.md](02-architecture.md): approvals, budgets.
- [03-trust.md](03-trust.md).
- [05-webagents.md](05-webagents.md): the WebAgents, missions, surfaces, upstream gaps U1–U21.
- [06-website-experience.md](06-website-experience.md): the visitor side; §4.3 is shared with this console.
- [08-control-plane.md](08-control-plane.md): tenancy, agencies, the operator console.

**Concepts:** [`concept/console/`](../concept/console/index.html):
- [the shell](../concept/console/shell.html);
- [the Store](../concept/console/store.html);
- [a mission reporting in](../concept/console/mission.html);
- [live takeover and walking the site by voice](../concept/console/live.html);
- [the phone](../concept/console/phone.html);
- [the trust centre](../concept/console/trust.html);
- [gestures](../concept/console/gestures.html).

**Alignment record:** `owner-console-alignment-2026-11-07.md` in the top session's notes folder (§12).

> **Your website is a team. Agentrix OS is its operating system.** One coordinator, the
> WebAgents as apps, work that goes on in the background while you sleep, agents that are live
> while your customers are on the site, and nothing consequential happens without your
> signature.

---

## 0. The idea in one page

Agentrix OS won its hackathon on one principle: an operating system with its own UI and UX in
which **everything is an agent**. One coordinator (the Mother) turned what the person said,
typed or gestured into work for specialised agents. It kept them honest and answered in one
voice, under a permission gate the person controlled.

A website owner's version of that OS has nine ideas. The concepts show each one:

| # | Idea | What the owner gets | Concept |
|---|---|---|---|
| 1 | **The Chief WebAgent is the shell** | One command bar, by text or voice. It shows the plan, the decisions it will create and the cost before anything starts, then answers in one voice | [shell](../concept/console/shell.html) |
| 2 | **WebAgents are apps** | A dock with live presence. A Store whose listings are generated from each WebAgent's spec. Permissions are shown by effect, with authority per job and a budget per app. Adding a WebAgent is a passkey-signed charter amendment | [store](../concept/console/store.html) |
| 3 | **Two worlds are two walls** | **Front of house** (visitors and AI agents, live, untrusted) and **Back office** (your WebAgents at work). The shell shows the wall between them and the typed signals that cross it | [shell](../concept/console/shell.html) |
| 4 | **Night Time** | The shell follows opening hours. During quiet hours it turns dark, only emergencies reach you, and the field becomes a timeline. In the morning that timeline is condensed into the brief | [shell](../concept/console/shell.html) |
| 5 | **Missions** | A goal given in words, compiled into a readable contract, approved by you, run in the background, and reported every week, including what didn't work | [mission](../concept/console/mission.html) |
| 6 | **Sit in, take over, hand back** | Watch a visitor's conversation unseen, step in as yourself, then hand back. The desk whispers facts from your own pages while you type | [live](../concept/console/live.html) |
| 7 | **Walk the site by voice** | Open your own site and talk. The Chief WebAgent points at what it means, Editor drafts the change, and you sign the release. Voice can stop everything, but it can never sign | [live](../concept/console/live.html#voice) |
| 8 | **Every decision is bound to its effect** | A passkey signs the exact effect's fingerprint; without one, a single-use code from the owner's authenticator app is bound to it. It runs once and becomes a receipt that names the method. One switch freezes everything, from the app, WhatsApp or voice | [trust](../concept/console/trust.html) |
| 9 | **Hands-free, safely** | Six hand gestures, recognised on the device, each confirmed by holding it. A gesture can move through cards, open, snooze, switch worlds, stop everything or bring up the passkey sheet, but it can never approve | [gestures](../concept/console/gestures.html) |

**The tension, resolved:** *calm by default, an OS underneath.* The OS is the **model**:
- a shell agent;
- apps with permissions;
- processes that sleep and wake;
- a task manager;
- an attention policy;
- a kill switch;
- a memory the owner can read.

That model is identical on every surface because adk-surface renders every surface from one
engine. The desk view draws it spatially. The phone draws it as a calm stack the owner can
clear in two minutes. §2 sets out what carried over from Agentrix OS and what did not.

## 1. Principles

1. **Everything is an agent, and only the owner signs.** WebAgents do the work and the
   Chief WebAgent speaks for them. Approving, raising authority, adding an agent, publishing and
   moving money are human acts, approved with a passkey or, without one, a step-up code
   (A-I13; 05 §2.3; 02 §8).
2. **One model, many shapes.** Every surface renders the same decisions, threads, agents and
   brief from adk-surface's `SurfaceEngine` under one rule table (05 §7; K-I17). The desk, the
   phone, WhatsApp, voice and email differ only in **shape** and in **powers**.
3. **Calm by default.** On a phone the default is a short stack, not a canvas. Notifications
   only say that something is waiting. Quiet hours are quiet, and a typical morning ends in
   about two minutes.
4. **Show the plan before it runs; show the effect before it is approved.** The command bar
   previews the plan. Every card says "If you approve, exactly this happens" and shows the
   recipient, the text, the amount and the release diff.
5. **Make the wall visible.** The public partition and the back office are drawn as two worlds,
   and the typed signals between them are visible. Security is part of the interface, not a
   promise in a footer.
6. **Authority is per job, earned and signed.** Lowering takes effect at once. Raising is always
   a passkey-signed card with evidence. A mission or a new app never raises anything.
7. **Sleep is visible and free.** The owner can see which WebAgents are asleep, why each woke
   and what it cost. Asleep costs nothing (K-I2).
8. **Honest numbers, honest presence.** The brief and the reports come from recorded activity
   only. A presence label comes from the tool that is actually running. "Resting" is a real
   state, and the method behind every number is one tap away.
9. **Hands-free never means sign-free.** Voice and WhatsApp can ask, decline, pause and stop.
   Gestures can navigate, open, snooze and stop. Money, publishing and high-risk actions always
   end at the passkey sheet on a screen; a gesture may raise that sheet, never sign it.
10. **Interruptions are budgeted.** Every proactive item has a priority, a reason and a route:
    interrupt, lane, digest or drop. Decision cards are rationed too: trivial ones gather in one
    digest card a day, and individual ones are capped per day (§3.5).
11. **Roles, not people.** WebAgents and desks are named by what they do ("Cakes", "Editor").
    Nothing has a human name, a face or a costume. The owner is always labelled as a person.
12. **Usable by hand when the models are down.** Every WebAgent's objects (pages, leads,
    orders, reviews) can be edited by the owner directly. If the models are unavailable, the
    console says so and keeps working.

## 2. Spatial or calm: what carried over, what changed and why

01 §4.1 deliberately dropped Agentrix OS's spatial field, gestures and 3D stage as the way in,
in favour of a calm, phone-first console. That was the right call for the phone and the wrong
one for the product's identity. An owner who sits down at a laptop on Monday should feel they
are running something, and a judge or a buyer should recognise it in five seconds. The
resolution keeps one model and draws it two ways.

| Agentrix OS | Decision | In the console | Why |
|---|---|---|---|
| Mother agent: coordinator, one voice, no world tools | **Kept, renamed** | **Chief WebAgent** is the shell; it is not an app | It is the right shape for accountability. All owner text enters through it ([05 §4.2](05-webagents.md#42-how-work-moves)) |
| Work and Home worlds, isolated | **Kept, with new meaning** | **Front of house / Back office**, the two partitions | The split is now the security design itself (05 §4.1), so drawing it teaches the trust model |
| Spatial field with cards | **Kept on the desk** | The field of the selected world; cards bloom near their source | Calm doesn't have to mean flat. The phone draws the same cards as a stack |
| Dock with status rings | **Kept** | WebAgents as apps with the visitor site's presence marks | One presence grammar on both sides of the site (06) |
| ProActive lane and attention manager | **Kept** | **The lane**: Needs you, Proposals, Done for you; P1/P2/P3 routing | Already in adk-website's `AlarmRouter` |
| Command bar ("shows what will happen before it happens") | **Kept** | Ask bar with a **plan preview**, decisions to come and cost | Owners need to see what an instruction will cost and what it will touch |
| Activity strip | **Kept, grown** | **Activity monitor** over the kernel's process table | The OS feature people trust most is the one that shows what is running |
| Trust pill, Pause all | **Kept** | Trust pill with a decision count; **Freeze** | Same idea, now with site and fleet levels |
| Observe / Suggest / Automate per tool effect | **Changed** | Five steps **per job** (Watch → Within budget), plus Undo only | The kernel's ladder is per job (05 §2.3) |
| Approvals inbox | **Changed** | Decisions **approved over the effect's fingerprint**: signed with a passkey, or a step-up code without one | Passkey approval is the platform's core trust promise (02 §8) |
| Recipes ("when X do Y") | **Changed** | **Standing grants**: bounded, expiring, signed | The gateway must verify unattended effects (05 §3.4, U6) |
| Missions | **Kept** | Missions over charter goals, agenda items and triggers | 05 §4.6 |
| Memory with provenance | **Kept** | **Known / Assumed** business facts; Teach your site | Only `Known` facts are ever published (A-I18) |
| Agent Store, agents as compiled apps | **Kept** | Store of WebAgents, packs and connections | Listings are generated from `WebAgentSpec` (05 §2.1) |
| VerbUI: one grammar for every input | **Kept, narrowed** | Verbs across tap, drag, keys, voice, WhatsApp buttons (§3.8) | The router only opens views, issues surface commands or hands text to the Chief WebAgent; it never calls an effect |
| Voice (Suzy) | **Kept, de-personified** | "Talk to your Chief WebAgent", with captions and barge-in | No persona name: WebAgents are roles, never people |
| Camera hand gestures (MediaPipe, plus Gemini Live seeing frames) | **Kept, rebuilt** (James, `d-gestures-key`) | Six gestures, on the device only, each held to confirm; they can never approve (§3.9) | A baker's hands are often full of dough, so hands-free is valuable. Agentrix OS's version acted on first sight, and a model reported gestures from an empty frame. The rebuild removes the model, adds dwell, calibration and recorded-clip tests, and draws a hard safety line |
| Drag to fuse cards | **Narrowed** | Drag a card onto a dock icon = **delegate** | The useful half; fusing has no business meaning |
| Generated themes (Studio) | **Dropped** | **Night Time** is the only theme change | The site wears the owner's brand; the console wears Agentrix's |
| Greeting gate, scripted scenarios, mock cards | **Dropped** | — | Honest data: nothing is ever a sample |

## 3. The OS model for a website owner

### 3.1 The mapping

| OS concept | Agentrix OS for owners | Platform and adk-rust |
|---|---|---|
| Kernel | The site's own kernel | A TenantKernel per site (05 §8.1, ADR-015) |
| Shell | **The Chief WebAgent** | `coordinator` principal, `TeamSpec` coordinator (05 §4.1) |
| Apps | **WebAgents**, plus their objects (pages, leads, orders…) | `WebAgentSpec` → one or more `HarnessAgent`s (05 §2.1) |
| App store | **Store**: WebAgents, packs, connections | `Kernel::agents` preview and apply; signed pack bundles |
| Permissions | Effects ("can / never"), ladder per job, standing grants | `JobEffect` classes, `AutonomyLadder`, `StandingRule` |
| Processes | Jobs, awake or asleep | `ProcessMeta`, `WakeSet` (05 §3.2) |
| Task manager | **Activity monitor** | `/kernel/processes`, `/kernel/events` |
| Notifications | **The lane** and the brief | `AlarmRouter` P1/P2/P3, `ProactivityPolicy`, `PushDoorbell` |
| Partitions | **Front of house / Back office** | Public partition and back office, joined by `SignalKind` (05 §4.1) |
| Background services | **Missions**, **Night Time** | Charter goals, agenda, standing triggers; `QuietHours` |
| Kill switch | **Freeze** | `Kernel::freeze`, `JobControl::pause` (05 §3.6) |
| Files | Business records, releases, receipts | Stores, signed releases, signed audit log |
| Memory | Known / Assumed facts, Teach your site | `MemoryFact`, `FactStore` (05 §3.5) |
| Login, sudo | Passkey first; without one, email and password or Google, LinkedIn or GitHub, always with an authenticator code. Sudo is approving an effect's fingerprint, by passkey or step-up code | WebAuthn (ADR-006); adk-surface `mobile/passkey.rs` |

### 3.2 The Chief WebAgent: the shell

The Chief WebAgent is the owner's single point of contact (01 §4.2). In the console it **is** the
shell: the command bar, the voice, the brief and the explanations. It is not an app in the
dock.

**Its mark is the team mark** (ADR-031). It is drawn where Agentrix OS had an orb, and it
replaces the earlier indigo orb, which was a circle with a simulated speaking pulse:
- the body is in the site's ink, with the site's monogram, in the squircle every WebAgent uses;
- there is one dot per WebAgent in that WebAgent's colour, and a dot pulses while its WebAgent
  is working or answering;
- the amber dot shows while a **delivered** card waits in Needs you. Cards the daily cap holds
  for the brief don't light it, so the cap's quiet holds. P1 cards and decisions a waiting
  customer is blocked on are always delivered, so they always light it (C13);
- it takes the shared states: listening while the owner holds it to talk, `speaking` from the
  real voice level, and grey when frozen.

**Intake has three stages, cheapest first** (from Agentrix OS §4.2, amended by architecture C1):
1. **Verb grammar** resolves direct commands against what is on screen, without a model:
   "approve this", "pause Sales", "open the Christmas mission". It may only **open read
   views, issue surface commands** (`Decide`, `PauseAgent`, `StopAll` …) **or hand text to the
   Chief WebAgent**. It never calls an effect.
2. **The capability router** matches the request against the WebAgents' listings. When one
   job clearly owns the request, the Chief WebAgent delegates directly and still tracks it.
3. **The planner** runs only when a request is multi-agent, ambiguous or high-stakes ("Put the
   Christmas menu up and tell last year's Christmas customers"). The plan is delegated through
   the back office's `Delegate` edges; there are no handoffs, so the Chief WebAgent stays
   accountable (05 §4.1).

**The plan preview** ([shell](../concept/console/shell.html), type in the command bar)
appears before anything runs. It lists:
- each step with its WebAgent, its job and that job's level;
- the decisions that will come to the owner;
- an estimated cost against the budget;
- what will not happen without a signature ("Nobody is contacted before you sign").

"Go" starts the steps that are within authority. Everything else becomes a card.

**Rules:**
- It asks **one** question when it can't decide, never several.
- It answers with citations: an object, a receipt, "(you told me)" or "(I think)".
- It answers questions about numbers ("How did the weekend go?") **only** through the read-only
  `analytics.metric` tool over the metric catalogue (09 §6.1; §8 here). It never writes a
  query and never invents a number, so it and the console can't disagree.
- It explains itself: "Why did Sales wake at 04:10?" is answered from the process table and
  the ledger.

**It never:**
- decides a card;
- reads raw visitor text (it receives only signals, and architecture C2 forbids a one-tap
  "forward a thread to the Chief WebAgent");
- holds a tool that reaches outside the site;
- has a human name.

### 3.3 Two worlds: Front of house and Back office

| | **Front of house** | **Back office** |
|---|---|---|
| What it is | The public partition: the Concierge's section desks ("Cakes", "Orders", "Visit us"), Ask this site threads, the agent door, the site inbox | The Chief WebAgent's team: Sales, Editor, Reputation, Cashier, Guardian, Caretaker; missions; decisions; the agenda |
| Reads untrusted text | Yes, by design | Never; it reads typed signals only |
| Holds privileged tools | No | Yes, behind the governed path |
| The field shows | The **live floor**: pages as rooms, visitors and AI agents as dots, by tier, desks with presence, open conversations | Cards: the brief, missions, WebAgents at work, "Across the wall today", Teach your site |
| Owner powers | Read any thread (rendered as untrusted), sit in, **take over** (PWA only), change desk rules | Decide, delegate, pause, missions, Store |
| Controls | The Concierge has **no dial**: its controls are the capability manifest and desk rules (05 §2.3) | A ladder per job |

**Visitor text in the console is untrusted** (architecture C2), because a visitor can try to
phish the owner:
- it is escaped, with no markup and no live links;
- "Ask about this thread" sends the Chief WebAgent a `thread_ref` and the typed fields of the
  thread's signals, never the text (pasting is the owner's own act, never the easy path);
- visitor-memory items appear only where the visitor allowed `owner_visible` (06 R12).

**The Concierge app** holds the Front of house controls of 06 R20, with the words agreed with
website-experience:
- the **Front desk**, which is the visitor-facing name of the router desk behind the Ask bar;
- the **section desks** (one per menu item), each with its mandate, its **starters**, and its
  **instant starters** (answered without a model);
- the **peek** (the hover card);
- brand voice and banned phrases;
- **configurable products** and their sketch parts. Each price, option and lead time extracted
  at the Move starts as Assumed, and the owner confirms it to Known;
- presentation scripts (chapters, must-say, must-not-say);
- artifact kinds, whether they can be shared, and when they expire;
- the interactive budget share;
- the **Teach your site** list (§3.7);
- "preview as a visitor".

These follow 06 §7 (requirement `r-exp-to-console`). Device memory for visitors is off by
default for government, health and education sites.

What a change needs depends on what it can promise. Changes to a desk's **may-promise list**,
its **capability subset** or the **memory scopes** are signed with the passkey. Starters and
labels are drafts the owner approves (Ask me first). All of them ship as manifest changes in a
release.

**"Across the wall today"** pairs each signal that crossed with what it triggered:
`lead_captured` → "woke Sales, quote drafted, waiting for you"; `content_gap` ×4 → "agenda
item for Editor". A counter states "no raw text crossed". This one card explains the
architecture to a non-technical owner.

### 3.4 WebAgents as apps

**An app is a WebAgent plus the things it looks after.** Opening one shows:
- its listing (mandate, can / never, wakes);
- its presence and current process;
- its **objects**, which the owner can also edit by hand: Editor shows pages and releases;
  Sales leads and quotes; Cashier orders and payments; Reputation reviews; Concierge desks,
  conversations and visitor artifacts; Guardian incidents and probes; Caretaker domain,
  certificates, backups and bills;
- each job's dial;
- its budget meter;
- its receipts;
- its **Appearance** (below);
- Pause and Remove.

**Appearance** is the WebAgent card's editor for its living mark (ADR-031; §3.5):
- **Colour**, from the site's palette or any colour, with two live checks: the mark against the
  site's background at 3:1, and the glyph against the mark at 3:1 (WCAG 1.4.11). A colour that
  fails offers "Use the nearest shade that passes".
- **Corners**, within the squircle range. It can never become a circle.
- **Glyph**: a role icon, or a monogram in the site's display face. The first release has no uploads.
  Owner-drawn glyphs come later, reduced in a sealed room (ADR-003) to plain single-colour
  paths.
- **Name**: a role or product name, never a person's name (06 §4.3).
- **Signature**: Shuffle gives this WebAgent a new seed. The pattern family is site-wide, so it
  lives in the site settings with the site's other brand choices.
- **Preview** in every state at 20, 28 and 40 px, on the site and on the console's Day Time and
  Night Time surfaces.
- **A change is a `MarkSpec` change** in the WebAgent's spec. It publishes with the next
  release, like a page change, so the site, the console and the channels never disagree.

This carries over Agentrix OS's companion-app rule ("every agent operates on an app the person
can also use by hand"), and it is what keeps the console useful with the models down.

**The Store** ([store](../concept/console/store.html)) has three shelves:
- **WebAgents:** new roles;
- **Packs:** teach an existing WebAgent a job (Christmas orders, Swahili pages, Advertising,
  off by default);
- **Connections:** M-Pesa, WhatsApp Business, Google Business Profile, held by the Effects
  Gateway.

Listings are **generated from the `WebAgentSpec` or the pack bundle, never written by hand**
(architecture C3; 05 §2.1). So the "can" and "never" lists always follow the jobs' tool globs,
the red lines and the invariants. The console proposes two platform types for them in
`agentrix-types`: `WebAgentListing` (from a `WebAgentSpec`) and `PackListing` (from a signed
pack bundle, adk-website `packs::bundle`, `trust`, `installed`). Section desks are not Store
items: they come from the desk manifest and live in the Concierge app (06 R4, R20).

| Listing field | Source |
|---|---|
| Name, mandate, version, signer, evaluation status | `WebAgentSpec`; pack signature against the trust list; ADR-016 suite result |
| **Can**, each with its effect tag (`read`, `draft`, `send_external`, `publish_public`, `schedule`, `financial`) | The jobs' tools and `JobEffect` classes |
| **Never** | Charter red lines, invariants (A-I17, A-I18, A-I23), job exclusions ("refunds are the Cashier's") |
| Wakes | The jobs' wake sets and standing triggers; "never in quiet hours" |
| Starting level per job | `WebAgentSpec` defaults, which the owner can lower before signing |
| Budget | A slice of the site's AI budget; at the cap, the WebAgent sleeps |
| Connections | Effects Gateway drivers it needs |
| Why it's suggested | The Chief WebAgent's evidence ("3 quote requests got no reply"), with a citation |

**The Add flow** has four steps: **What → Permissions → Authority → Sign**.
- The sheet compiles into one `Kernel::agents` **preview**. Its digest covers the charter
  amendment, the pack's verification and any connections.
- The owner signs that digest with the passkey, and the console **applies** it.
- The new app drops into the dock and shows a **first look** built from real data. If it has
  nothing to work on, it says so.

The rules:
- Agents may *suggest* an app; only a person adds one.
- Adding never raises another WebAgent's authority.
- A pack that widens permissions on update asks again.
- A tool whose declaration changes is stale and asks before every call (K-I14).

**Permissions per app** are shown as effects, each with its starting level and its
**ceiling** (ADR-012 amendment 1; 05 §2.3). A job's ceiling is the ceiling of the most
consequential tool it holds, by the catalogue's `EffectClass`:

| Effect | Owner's words | Starts at | Ceiling |
|---|---|---|---|
| `read` | "Reads your leads, pages…" | Allowed | — |
| `write_local` | "Drafts", reversible | Allowed | Handle it within budget |
| `publish_public`, the site's own pages | "Changes your pages; the last signed release can undo it" | Ask me first | Handle it within budget |
| `publish_public`, off the site | "Posts, or replies to reviews, somewhere else" | Ask me first | **Ask me first** |
| `send_external` | "Emails or messages someone" | Ask me first; never without a consent record | **Ask me first** |
| `schedule_with_others` | "Books a meeting with someone" | Ask me first | **Ask me first** |
| `financial` | "Takes or returns money" | Ask me first | **Ask me first** |
| `delete` | "Deletes something for good" | Ask me first | **Ask me first** |
| `other` (not classed by the catalogue) | "Something we can't classify" | Ask me first | **Ask me first** |
| reversible only | "Rolls back, switches to read-only" | Guardian only: Undo only | Undo only |

- **The only way past a ceiling** is an owner-signed standing rule that states all four limits:
  an amount, the recipients, a frequency and an expiry (§5.4). The Effects Gateway checks the
  signature and every limit before it runs the effect unattended.
- **Value given away is money.** Discounts, refunds, store credit and free items share one
  value-given cap per site, across the Concierge, Sales and the Cashier. The console shows it
  as one meter, so curbing one channel can't quietly move the giving to another.

**Authority** is the five-step ladder per job (Watch, Suggest, Ask me first, Do it and tell me,
Within budget), plus Guardian's Undo only. The mapping to adk-kernel levels and harness modes
is in [05 §2.3](05-webagents.md#23-the-owners-dial-and-the-ladder).

The console adds these behaviours (architecture C7, with ADR-012 as amended on 7 Nov):
- **Lowering is immediate** and needs no signature.
- **Raising is always a passkey-signed card** with its evidence: 20 **reviewed** approvals
  unchanged in 30 days, no breach, and the evaluation suite passing (§5.3).
- **Positions above the ceiling are locked.** The dial shows them with a lock and the reason
  in the owner's words, for example: "Sales messages your customers, so it always asks first.
  To let it send on its own, sign a rule with an amount, who it may send to, how often, and an
  end date." The link opens the rule editor (§5.4).
- **A demotion shows as "lowered"**, with the reason and the date.
- **Asking is welcome.** At any level, a WebAgent that is unsure or meets something new raises
  a card instead of acting (§3.5).

**Budgets** sit at four levels:
- the site's monthly AI budget (the status bar);
- each app's share (its meter);
- each job's caps (the charter);
- each mission's slice;
- the site's value-given cap (discounts, refunds, credit and free items), as one meter.

At a cap, background work sleeps until `BudgetReplenished`, and raising the cap is a card.
The brief reports spend per WebAgent.

**Lifecycle:** added → awake or asleep → paused (owner) or lowered (automatic) → removed.
Removing keeps the receipts. Updates arrive as signed versions in release trains (08 §3.5).
Behaviour-changing packs need the owner's adoption.

### 3.5 Background work

**How a WebAgent looks.** Each WebAgent is a **living mark** (ADR-031, decided by James on 7
Nov). The generator and an owner-style workshop are in
[`concept/identity/marks.html`](../concept/identity/marks.html). The mark has four layers:
- **Silhouette:** a squircle, never a circle. Circles mean a person, such as the owner's chip in a
  thread they took over (§3.6).
- **Signature:** a seeded pattern, one family per site and one seed per WebAgent, so a site's
  WebAgents read as one team and each one is unique. It shows from 28 px up and moves only
  while the WebAgent works; idle stays still.
- **Glyph:** a role icon, or a monogram in the site's display face.
- **State:** the grammar below.

The console wears Agentrix tokens around the mark, but the mark keeps the site's colours
(06 §4.3), so the owner sees the same WebAgent visitors see.

**Presence and states** use one vocabulary across the site and the console. The presence enum
is the visitor site's (06, R5), with `speaking` added by ADR-031; back-office states come from
the process table.

| Owner's words | What the visitor sees (06 §4.3) | Presence or state | Mark ([`console.css`](../concept/console/console.css), mirrors `.wa-mark`) |
|---|---|---|---|
| (mark only) | Mark only | `idle`; also a desk in a thread the owner has taken over | plain |
| Listening / Watching | Mark only | `listening`; Guardian between probes | slow breath |
| *Tool label*: "Checking today's stock" | The same real label | `working(label)`, from the running tool; plain `working` shows no text | a line traces under the mark |
| Answering | Ring only, while text streams | `answering` | steady ring |
| Speaking | The ring follows the voice | `speaking`: the real voice level from the TTS stream (U7), never a simulated one | the halo's opacity and scale follow the level |
| Handing to Orders | "Cakes handed you to Orders, with the conversation so far" | `handing_over(to)` | — |
| Waiting for you | "Waiting for Wanjiru's OK" (or the business name) | `waiting_on_owner`; process parked on `Wake::Approval` | amber dot |
| Asleep · next 07:15 | — | process `Waiting` on its `WakeSet`, no lease | desaturated |
| Paused | — | `JobPauses`, with its reason | grey |
| Waiting its turn | — | `Suspension::Preempted`: a human turn took the slot | desaturated |
| Frozen | — | `ParkReason::Frozen`: the site or a platform hold froze it | grey, with the freeze banner |
| Resting · *the real reason*, e.g. "budget reached" | "Answers are paused right now. The page and the form below still work." Never the reason | Budget or models unavailable (05 §9.1) | greyscale |

- **Labels** come only from tool calls the governed path allowed (architecture C6).
- **Shared values with the visitor site (V1):**
  - mark sizes 20, 28 and 40 px. Corners follow each mark's squareness, with 6, 9 and 12 px as
    the default;
  - listening: a ring at −4 px with a 2 px border, breathing over 2.2 s;
  - working: a 2 px line 6 px under the mark, sweeping in 1 s;
  - answering: the ring at a steady .45;
  - waiting: a 9 px #b26b00 dot;
  - resting: greyscale at .55.
- **Motion:** reduced motion turns every animation off, including the signature, and each
  state still reads by its shape.
- **Two kinds of waiting.** A single WebAgent's mark shows `waiting` (the amber dot) whenever
  its process is parked on the owner, because that mark reports the agent's state. The team
  mark's amber dot reports what needs the owner now, so it follows delivered cards only (C13).
  A job whose card the cap holds for the brief shows amber on its own mark in the dock, while
  the team mark stays plain.
- **Contrast on other surfaces** (C12, ADR-031). A mark's colours are checked against its
  site's background. Wherever it falls under 3:1 against another surface, such as the dark
  Night Time field where the team mark's ink body can disappear, the host surface draws a 2 px
  keyline in its own ink around the squircle and never recolours the mark.
- **No typing indicators anywhere.**

**The Activity monitor** ([shell](../concept/console/shell.html), dock → Activity) is the
task manager. It lists every process with:
- its WebAgent and job;
- its state;
- **why it woke** (the wake key in plain words: "Your WhatsApp message, 06:58",
  "`lead_captured` 04:10", "every 5 minutes");
- its next wake;
- today's spend;
- a **Pause** control.

It reads `/kernel/processes` and `/kernel/events`, which the kernel already serves; adk-surface
does not yet project them (§9.2, F2). Kill, nice and resume are for operators; the owner
pauses, and resuming needs the owner.

**Missions** ([mission](../concept/console/mission.html)) are the agreed platform type over
charter goals, agenda items and standing triggers ([05 §4.6](05-webagents.md#46-missions)).

1. **Say it.** "Get us 60 Christmas cake preorders by 18 December. Don't spend more than
   KSh 400." This works by text, voice or WhatsApp.
2. **Read the contract.** The Chief WebAgent compiles it into:
   - goal;
   - measure (bound to a charter KPI where one exists; otherwise **measure-only** until a
     baseline exists, so it drives no promotion);
   - owners (WebAgents and jobs);
   - wakes (mission context added to the jobs' existing wakes, **not** one long `Goal` loop);
   - budget slice;
   - authority (at most the jobs' current levels; a mission may lower a level, never raise
     one);
   - end date.
3. **Approve it.** A mission that adds a KPI, a guardrail or authority is a charter commit,
   signed with the passkey. Otherwise it is an owner-approved agenda item plus triggers.
4. **It reports.** Every Monday in the brief and the lane, with:
   - progress against the measure;
   - **what worked** and **what didn't**, each with citations;
   - **what we want to do next**, each step with its owner and level;
   - the **counting method**, including what it misses.

   Steps that need the owner arrive as ordinary cards.
5. **It ends** at its expiry, when its triggers are removed and the outcome is reported.

**The lane** is Agentrix OS's ProActive lane, with three sections:
- **Needs you:** held calls, i.e. decision cards.
  - **Raised by the agent** (ADR-012 amendment 2). A card a WebAgent raised because it was
    unsure, or met something new (a new kind of request, a new recipient, an amount outside the
    job's history, a page type it hasn't handled), carries the mark "Asked by Sales · new
    recipient". Its answers are Go ahead / Not this time / Teach. The owner's answer is never
    recorded as a rejection and never lowers the job, and the weekly review counts asking in the
    job's favour ("Sales asked 4 times when it was unsure"). An agent-raised card is never
    trivial, so it is never digested.
  - **The digest card** (amendment 3). Trivial cards (low risk, reversible, inside the job's
    limits) don't arrive one by one. They gather in **one digest card a day**, delivered with
    the morning brief, one line per item, with Approve all or open an item. Approvals from the
    digest never count toward promotion, even for an item opened from it.
- **Proposals:** agenda items proposed under the `ProactivityPolicy`, with Plan it / Not now /
  "Why am I seeing this?".
- **Done for you:** acts within grants and items of an approved plan ("under plan: …", §5.4),
  with undo where it applies.

Every item carries a priority, a reason, an expiry and its source. The attention manager routes
it:

| Route | When | Mechanism |
|---|---|---|
| **Interrupt** | P1: site down, payment anomaly, incident. Rings in quiet hours until acknowledged | `AlarmRouter` P1 on the owner's P1 channels |
| **Lane** | P2: a held decision blocking a customer, a guardrail breach; other decisions, **up to the day's card cap** | Within the hour, outside quiet hours; push says only "A decision is waiting" |
| **Digest** | Done-for-you, proposals, mission progress; **the daily digest card** of trivial decisions; **cards held by the cap** | The morning brief |
| **Weekly** | P3, earned-autonomy offers, mission reports | The weekly review |
| **Drop** | Over the proactive budget, or muted ("less like this") | `ProactivityPolicy` caps, per-WebAgent feedback |

**The daily card cap** (ADR-012 amendment 3). The attention manager owns it.
- Individual decision cards are capped per site per day. The proposed default is **5 a day**,
  a starting setting rather than a finding. The owner changes it in the Trust centre, and the
  pilot measures it (§8).
- Over the cap, a card waits for the next brief, except:
  - **P1**, which always interrupts;
  - **a card near its expiry.** No card is held past its `expires_at` minus 1 hour, the same
    limit as a snooze (§3.9). It arrives then, without ringing in quiet hours, so the cap
    never turns into silent expiries;
  - **a decision a waiting customer is blocked on** (C11). It arrives individually and still
    counts against the cap. The platform knows the customer is blocked only from the
    visitor's thread parked on that approval (`Wake::Approval` with a live thread), never
    from the agent's word, so "a customer is waiting" can't become a way around the cap.
- **The cap rations what reaches the owner, never what the owner can see.** Held cards sit in
  a collapsed "Waiting for your brief (3)" row and open on demand.
- Agent-raised cards count against the cap like any other card. When a job keeps asking about
  the same kind of thing, the weekly review offers a standing rule instead.

**Day Time and Night Time.** Opening hours are Day Time. Quiet hours (`QuietHours`, in the
owner's IANA time zone) are Night Time, and they change three things in the shell:
- the theme turns to the dark field;
- the lane goes silent except for P1;
- the Back office field becomes a **timeline** of recorded work ("01:30 Guardian blocked 212
  probes"; "04:10 Sales drafted a quote and parked it for you").

At the end of quiet hours the Chief WebAgent condenses that timeline into the **morning brief**.
The brief is composed only from the ledger, the agenda, the stores and spend, then rendered per
surface. "My website worked while I slept" (01 §3, rung 3) becomes something the owner can see
in the interface.

### 3.6 Live agents

**Presence** is live:
- the dock rings;
- the "3 live" count on Front of house;
- the floor's visitor dots;
- a desk's state in each thread.

Latency budgets follow 06 R2. A presence update is a feed frame, never a model call.

**Sit in, take over, hand back** ([live](../concept/console/live.html)). This is architecture
C5 and 05 §4.5; it needs **U21** upstream.

1. **Sit in.** The owner opens a live thread. Visitor text is rendered as untrusted. The
   visitor sees only the desk.
2. **Take over** (PWA only). The thread's **speaker** becomes the owner.
   - The desk stops taking turns in that thread and its mark goes **idle**. It does not show
     as resting, which would mean the site is degraded.
   - The server labels the owner's messages "a person"; the client never does. They carry a
     person chip (an initial in a circle), never the AI chip or a desk mark.
   - The visitor sees the line "Wanjiru from Kilele joined · a person", and their input
     placeholder becomes "Message Wanjiru…". The AI never writes while the owner holds the
     thread.
   - Takeover is not offered on WhatsApp, because the owner's WhatsApp messages go to the Chief
     WebAgent.
   - If the owner leaves without handing back, the desk takes the thread back after about
     10 minutes, with the same hand-back line.
3. **Whispers.** While the owner types, the lane shows facts from the site with citations, the
   owner's rules and "not on your site" gaps. They come **from the desk**, which already reads
   the thread, never from the back office.
4. **Hand back.** The speaker returns to the desk. The visitor sees "Cakes is back · AI". The
   desk summarises, treating the owner's words as **context, not authority**: any effect still
   needs its card, which the owner can approve at once. Takeover and hand-back are audited as
   `thread.takeover` and `thread.handback`.
5. **Teach your site.** What the owner said can become a `Known` fact and an Editor draft
   ("Gluten-free tiers on request"), signed as a release like any other.

**Talk to your Chief WebAgent** (voice) runs on adk-surface's `VoiceHub`:
- push to talk, or hold the Chief WebAgent's team mark;
- barge-in and live captions;
- spoken answers go through `decide_spoken`. A spoken allow-once gets a 5-second cancel
  window; a spoken "stop everything" acts at once (adk-surface `voice/session.rs`).

By voice the owner may **ask, decline, pause and stop everything**. Voice never approves money,
publishing or high risk (`voice_cant_approve`); it puts the passkey sheet on the screen
instead. Nothing is always listening.

**Walk the site** (co-browse,
[live → voice](../concept/console/live.html#voice)):
- The owner opens their own signed release inside the console.
- The Chief WebAgent points with the **same pointer the visitor site uses**: the
  `pointer {anchor}` event over the release's anchor map (06 R5, R7; U18). The geometry is the
  site's `.pointed`, in Agentrix coral instead of the host's accent:
  - a 3 px outline at a 4 px offset, with a 10 px radius;
  - two pulses over 1.4 s;
  - a tag pinned top-right, reading "Looking here" or a short reason;
  - it clears itself after about 5.2 s;
  - it scrolls only the nearest scroll container (instantly, with reduced motion).
- The owner says what to change. The Chief WebAgent delegates to Editor, which drafts a release
  whose diff appears on the page itself: struck-through old prices, new lines highlighted,
  "Draft · not live yet".
- Prices come only from the owner's own sources. New claims become `Known` facts because the
  owner said them.
- Publishing is signed on screen.

This is safe because the Chief WebAgent reads the signed release, which is trusted content, and
never a visitor thread.

**The agent door, live.** AI agents appear on the floor as agent dots, with their tier and
receipts ("A known shopping assistant read today's stock"). A valid Web Bot Auth signature
earns only `known`: it proves who runs the agent, not who the user is. `verified` and `partner`
come only from lists the owner keeps (06 §12.3; 05). Their orders are held like anyone else's.

### 3.7 Memory

- **What your WebAgents believe about your business**
  ([trust → memory](../concept/console/trust.html#memory)): facts as **Known** (your words or
  your confirmation, with where and when) or **Assumed** (inferred, with the evidence). Each can
  be confirmed, corrected or forgotten. Only `Known` facts are published or promised (A-I18);
  an `Assumed` fact can only shape a question.
- **Teach your site:** the queue of questions the pages couldn't answer (`content_gap`), with
  counts. Answering creates a `Known` fact and an Editor draft. A visitor asking 4 times is the
  evidence.
- **Your rules:** declining a card can save a rule ("Never discount more than 10%"). Rules
  become charter red lines or standing-rule predicates, visible in the Trust centre.
- **Visitor memory:** the owner sets the scopes (06 R12; session only by default). The owner
  sees counts and policy, not profiles. A visitor's remembered items appear only inside that
  visitor's own thread.
- **Export and erase** reach every memory (05 §3.5).

### 3.8 VerbUI for owners

Every interaction is a verb on an object, as in Agentrix OS ADR-010:
- ask, approve, decline, edit, snooze;
- pause, resume, freeze;
- delegate ("give to Editor"), teach;
- sit in, take over, hand back;
- roll back, share.

Local verbs (open, snooze, pin) run in the shell. Owner verbs are surface commands. Effects go
through the governed path, and consequential verbs end at the passkey sheet.

| Input | Produces verbs by |
|---|---|
| Tap / click | Primary verb on a card; "⋯" or long-press for the verb menu |
| Drag (desk) | A card dropped on a dock icon = `delegate` ("Teach your site" → Editor) |
| Keyboard | Full parity: arrows across cards, Enter = primary verb, `/` = command bar, `Esc` = back |
| Touch gestures (phone) | Swipe a lane card = snooze. **Swiping never approves** |
| Camera (desk, opt-in) | Six held hand gestures (§3.9). The shell treats each one as a verb; a gesture never decides |
| Voice | Same verbs; "that" = the focused card or the last thing spoken about |
| WhatsApp | Quick-reply buttons map to `Decide` (everyday cards) and STOP to `StopAll` |
| Email | Links only; email decides nothing |

### 3.9 Gestures: hands-free, safely

James's decision (`d-gestures-key`) is that gesture control is a key feature of the console and
must work. The idea is Agentrix OS's; this is a rebuild for reliability and safety
([gestures](../concept/console/gestures.html)).

**What Agentrix OS taught us** (from `web/static/gesture-*.js` and its git history):

| What happened | Evidence | What the console does instead |
|---|---|---|
| A model in the loop invented gestures. With the camera frames going to Gemini Live, an empty frame and the user saying "here's my gesture" produced a gesture **4 times out of 4**, and twice a second intent as well | Commit `9e2239b`, "a gesture is not also an intent" | **No model in the gesture path.** A deterministic classifier over on-device landmarks. No frames leave the browser, ever |
| Gestures acted on first sight: a pinch fired after 2 frames, a palm after 0.8 s, a wave immediately | `gesture-classifier.js` defaults | **Hold to confirm.** A dwell ring fills and plays a sound; letting go cancels. Only reversible navigation skips the dwell |
| Two detectors reported the same gesture, so a 1.5 s dedupe was needed | `gestures.js` | **One detector.** One event per gesture |
| Camera and model failures were only logged; a dropped session turned the camera off; a restart leaked an interval and left the camera light on | Commits `ab48ea5`, and the `generation` counter in `gesture-detect.js` | **Visible states.** A camera pill that is always shown when on, failures in words, one stream with a strict lifecycle, auto-off |
| MediaPipe and its model loaded at runtime from jsDelivr and Google Storage | `gesture-detect.js` `CDN`, `MODEL` | **Self-hosted, pinned, integrity-checked** WASM and model, served from the console's origin |
| Swipe direction flipped with a mirrored feed | `mirrored` option | **Calibration** records handedness and mirroring per device |
| Tests used synthetic frames only | `scripts/test-gesture-classifier.mjs` | **Recorded landmark clips** with accuracy and false-trigger targets, run in CI |

**The pipeline.**
1. MediaPipe's hand landmarker runs in the browser at about 15 fps. It uses the GPU delegate,
   falling back to the CPU, from a pinned bundle served by the console itself.
2. Each frame's facts (21 landmarks, handedness, the canned gesture category and its score) feed
   a **pure, deterministic classifier**. It is testable in Node, like Agentrix OS's.
3. The classifier emits `armed`, `progress` and `committed` or `cancelled` events.
4. The shell turns a committed gesture into a **verb**, exactly as a key or a tap would (§3.8).
   Shell-local verbs run in the shell; owner verbs become adk-surface `Command`s.
5. The page has no API that uploads frames. The worker's content-security policy forbids
   network fetches.

**The vocabulary is small:** six gestures, chosen for MediaPipe's robust canned categories and
for distance from everyday hand movements.

| Gesture | How | Confirm | Verb | Keyboard | Touch | Voice |
|---|---|---|---|---|---|---|
| (engage) | A hand enters the gesture zone and is steady for 0.3 s | — | Shows the hand ring ("I see your hand"); nothing else | — | — | — |
| **Swipe** ← / → | Open hand moving at least a third of the zone in at most 0.6 s | None: reversible | Previous / next card in the lane or stack | ← / → | Arrows; scroll | "back" / "next" |
| **Point up** | Index finger up, held | 0.8 s | Open the focused card | Enter | Tap | "open it" |
| **Thumb up** | Held | 1.2 s | **Review and sign**: raises the passkey sheet for the focused decision; never signs | A | Tap Approve | "approve that" (also only raises the sheet) |
| **Fist** | Held | 1.2 s | Snooze the focused decision card until the next brief (rules below). On a Coming up agenda item it only opens the snooze choices, and a tap or key picks one | S | Swipe the card | "snooze it" / "not now" |
| **Two fingers (V)** | Held | 0.8 s | Switch world: Front of house ⇄ Back office | W | Tap the world pager | "show Front of house" |
| **Open palm** | Held still | 2 s, then a 5 s countdown in the shell | **Stop everything** (Freeze) | ⇧F | Freeze | "stop everything" |

**Confirmation, feedback and the rules against false triggers:**
- **The HUD shows what will happen before it happens.** A gesture bar at the bottom of the
  shell names the gesture and its verb ("Thumb up · Review and sign"), and a ring fills over the
  dwell time.
- **Sound:**
  - a soft tick when the hand is seen;
  - a rising tone while the ring fills;
  - the same ding as a click when the gesture commits;
  - a low "thunk" when it is cancelled.

  Sound follows the system mute, and there is a captions-only mode.
- **Letting go cancels.** Changing shape, moving, or lowering the hand before the ring is full
  does nothing.
- **Re-arm.** After a commit, the same gesture must be released before it can fire again. A
  1-second cool-down applies to every gesture.
- **One hand, one person.** Only the largest hand in the zone counts. With two hands (a child,
  a colleague walking past), nothing fires and the HUD says "one hand at a time".
- **Focus is explicit.** Gestures act only on the focused card, which is outlined. With no
  focus, swipe focuses the first card in Needs you.
- **Stop everything** from a palm runs a **5-second countdown in the shell**, full width. The
  shell holds the command and sends `StopAll` only when the countdown ends; the framework never
  delays a stop (S-I10). Cancel by Esc, tap or "cancel". The cancel isn't a gesture, which keeps
  the vocabulary small. Unfreezing needs the passkey on the PWA (§5.5).
- **Snoozing a held decision** (05 §7, "Snoozing a held decision") changes when the owner is
  reminded, never the decision. The held call stays held at its version; nothing is answered,
  deferred or extended.
  - It lasts until the next brief, but ends at the latest 1 hour before the request's
    `expires_at`. A card in its last hour can't be snoozed, and the shell says when it
    expires.
  - P1 cards can't be snoozed. A snooze ends early if the card's version changes, it rises to
    P1, or the approval escalates.
  - It is per person: a teammate who can decide still sees the card.
  - It is feedback on timing only. It is never an outcome, never counts toward promotion, and
    an expiry is recorded as "expired", not "rejected".
  - The fist doesn't reach adk-surface's own `snooze` verb, which moves a Coming up agenda item
    through the governed path. On such an item, the fist only opens the snooze choices.

**The safety line.** A gesture may open, navigate, snooze, switch worlds, pause one WebAgent
from its app, stop everything, and raise the passkey sheet. A gesture never approves, declines,
edits, resumes or unfreezes. It never triggers a card's effect, because approving needs the
passkey's user verification (Face ID, Touch ID, Windows Hello or a PIN) or a code typed from the
authenticator app, and no hand shape can produce either.

In adk-surface terms, gestures issue only shell-local verbs, `StopAll` and `PauseAgent`.
- **The pilot's defence in depth** (U22, G1): an `Origin::Gesture` with its own `Row`.
  `allowed_answers` gives it no decision options (like `Row::Agent`). `execute` accepts only
  `StopAll` and `PauseAgent` from it, and refuses `Decide`, `ResumeAll`, `ResumeAgent`,
  `SetMode` and `SubmitDrawn`.
- **Pausing one WebAgent:** today `PauseAgent` shares the `may_change_agents` gate, which only
  the web home and the terminal pass, so even a spoken "pause Sales" is refused. U22 gives
  pause a gate of its own, open to every human row, because pausing is in the stop direction.
- **After a thumb-up**, the `Decide` carries the origin of the confirming tap or key on the web
  row, with presence, never Gesture. The WebAuthn ceremony needs a real tap or key anyway: a
  camera gesture isn't browser user activation, and Safari in particular is strict. So the sheet
  is raised by the gesture and confirmed by a touch.
- **The record:** `surface.gesture {gesture, verb, outcome: committed|cancelled, command_id}`
  goes into the **site's audit trail, not the activity ledger**. Ledger rows can leave the site
  for fleet learning, and these would reveal when the owner is at the desk.
  - It is written for thumb-up, fist and palm, including cancelled palm countdowns.
  - Swipe, point-up and two-finger gestures are counted on the device only, for calibration.
  - The browser writes the record, so it is a report, not proof. The command's own record
    (`surface.stop_all`) carries the caller's origin.

**Camera consent and state:**
- **Off by default, opt-in per device.** The first time, a sheet explains:
  - on-device only;
  - what the camera is used for;
  - how to turn it off;
  - that nothing is recorded.

  Then the browser's own permission prompt follows.
- **Always visible while on:** a status-bar pill, "● Camera on · gestures", next to the trust
  pill, plus the browser's indicator. One tap turns it off. A small preview, local and never
  stored, is available from the pill.
- **Auto-off:**
  - after 2 minutes without a hand;
  - when the tab is hidden or the screen locks;
  - during quiet hours (Night Time);
  - on Freeze, except the palm that caused it: the camera stays on for the countdown, then
    turns off.
- **Failures in words:** permission denied, no device, device busy, the model failed to load.
  The keyboard, touch and voice equivalents keep working.

**Per-owner calibration** takes about 60 seconds, is optional, and can be repeated from the
camera pill. It is stored only on the device, in IndexedDB, and holds no images:
1. **Light and framing:** checks that the hand is detected in the zone and the image is bright
   enough. Advice is in words ("Too dark; face the window").
2. **Which hand, and mirroring:** fixes the swipe direction.
3. **Each gesture three times:** records per-owner thresholds (finger-curl angles, thumb angle,
   swipe speed). Travel is normalised by hand size, not frame width, so distance from the
   camera matters less.
4. **Ten seconds of "just talk with your hands":** a false-trigger floor. If any gesture would
   have fired, its threshold is raised and the owner is told.

**Tests and targets.** These are targets, not results; they are measured before any claim.
- **The corpus.** Recorded **landmark tracks** (21×3 coordinates per frame, labelled with
  gesture start and end), not video. The source clips are recorded with consent and kept
  private, so the tracks can be re-extracted when the MediaPipe model changes. The corpus
  covers:
  - at least 10 people, across skin tones, hand sizes and left and right hands;
  - four lighting conditions: daylight, dim, backlit, kitchen fluorescent;
  - distances from 0.4 to 1.5 m;
  - a laptop webcam and a tablet front camera;
  - cluttered and plain backgrounds.
- **Background clips:**
  - talking with hands, typing, drinking from a mug, a phone call;
  - kneading or handling goods, adjusting glasses;
  - waving at someone else, people walking behind.

| Measure | Target |
|---|---|
| Recall: the right verb commits within dwell + 300 ms | ≥ 95% calibrated, ≥ 90% uncalibrated, in normal light |
| Wrong verb committed (one gesture taken for another) | ≤ 0.5% |
| False commits on background clips, navigation (swipe) | ≤ 1 per hour of background activity |
| False commits on background clips, state-changing (snooze, stop everything, raise the sheet, open) | **0 in the corpus**; ≤ 1 per 10 hours in the pilot |
| Hand seen → ring visible | ≤ 150 ms |
| Recognition rate on a mid-range laptop | ≥ 12 fps, ≤ 15% of one CPU core |

The classifier runs over the corpus in CI on every change (as Agentrix OS's Node test does, but
over recorded tracks) and fails on any regression. A MediaPipe version bump re-extracts the
tracks and must pass again. Each release reports precision, recall and false commits per hour
in its notes.

**The concept runs on the real camera** ([gestures](../concept/console/gestures.html), with the
rules in `concept/console/gestures.js`):
- MediaPipe's hand landmarker runs on the page's main thread (the product uses a worker). It
  uses the GPU delegate, falling back to the CPU, and tracks two hands so that two in view fire
  nothing. The 21 points are drawn over a mirrored camera tile beside the ring.
- The rules are plain and pure, and the same file loads in Node:
  - a finger is out or curled by the ratio of its knuckle-to-tip distance to its length, from
    world landmarks; an unsure finger means no gesture;
  - the thumb is tucked or out by its distance from the index and middle fingers, in palm
    lengths;
  - "up" for point, two fingers, thumb and palm comes from the image;
  - a swipe is an open hand travelling 1.2 palm lengths sideways within 0.6 s, with a cooldown,
    and the return stroke is ignored.
- It doesn't use MediaPipe's canned gesture categories (step 2 of the pipeline). Whether they
  add anything over these rules is for the corpus to show.
- Holds:
  - a shape must be steady for 150 ms (the palm 300 ms), and still, before its ring starts;
  - it may drop out for 250 ms;
  - the ring fills only up to the last frame in which the shape was seen, so a hand that has
    gone can't commit;
  - after a commit, the shape must be gone for 0.5 s before it can fire again.
- Auto-off, failures in words, and pausing every gesture except the palm while a sheet is open
  all work.
- It is tested on synthetic hands built from joint angles (`gestures.test.js`), not on the
  recorded corpus, so none of the targets above is claimed. Calibration isn't built; the
  default thresholds apply.
- Unlike the product rule above, the concept loads MediaPipe Tasks Vision 0.10.14 from jsDelivr
  and the model from Google's model store, pinned by version but not self-hosted.

**Accessibility.** Gestures are optional and never the only way. Every gesture has keyboard,
touch and voice equivalents. Dwell times can be lengthened in calibration. Reduced motion keeps
the ring as a static progress bar.

**The first release (the Vultr demo) subset:**
- camera opt-in, the camera pill and auto-off;
- engage, and **swipe** through the decision cards;
- **thumb up** on the go-live card, which raises the passkey sheet; the owner then signs with
  Touch ID;
- **open palm**, which stops everything after a 5-second countdown in the shell. In the first release the
  `StopAll` goes in with the web origin, so only the `surface.gesture` record ties it to a
  gesture, and the demo does not claim a server-side gesture policy until U22 lands;
- a 20-second calibration (light and handedness);
- a first corpus: the team, 3 people, 3 lighting conditions, 10 takes of each demo gesture, and
  10 minutes of background.

The demo states what was measured and nothing more. Point, fist and two-finger gestures, and the
full calibration, come in the pilot.

## 4. Information architecture across surfaces

### 4.1 One model, five shapes

| | Desk (PWA, wide) | Phone (PWA) | WhatsApp | Voice | Email |
|---|---|---|---|---|---|
| Shape | The OS shell (§4.2) | Now · Decisions · Live · Apps (§4.3) | A thread with the Chief WebAgent | A conversation | Brief and weekly review |
| Brief | Chief WebAgent card in Back office | Top of Now | 07:00 message | "How did the night go?" | 07:00 email |
| Decisions | The lane | Decisions tab and stack | Everyday cards: Allow once / Decline | Decline only | Links to the PWA |
| Passkey, or a step-up code without one | Yes | Yes (Face ID / fingerprint, or a code) | Opens the PWA | Opens the PWA | — |
| Live | Front of house floor, takeover | Live tab, takeover | Handover notices | — | — |
| Freeze | Status bar | Apps → Freeze | Reply STOP | "Stop everything" (acts at once) | — |
| Unfreeze | Passkey or code | Passkey or code | ✗ | ✗ | ✗ |

What each surface may answer comes from adk-surface's one table (`rules::allowed_answers`,
`adk-surface/src/rules.rs:231`), and resuming is Web-only (`may_resume`, `rules.rs:454`). A
decision can't be approved on a surface with weaker checks.

### 4.2 The desk: the OS shell

| Region | Contents |
|---|---|
| **Status bar** | Site or scope chip (switcher); **world pager** (Front of house with a live count, Back office); **time pill** (Day Time · until 18:00 / Night Time · until 06:00); budget meter; **trust pill** (walls holding, decision count); **camera pill** ("● Camera on · gestures", only while on); **Freeze** |
| **Dock** | WebAgents with presence marks and badges, then the system apps: **Store, Activity, Memory, Trust**. Missions open from cards or the dock |
| **Field** | The selected world: Back office cards (day), the Night Time timeline (quiet hours), the Front of house floor (live) |
| **Lane** | Needs you · Proposals · Done for you (day); quiet-hours notice and Stop everything (night) |
| **Command bar** | The Chief WebAgent's team mark, the ask line, the mic, the plan preview |
| **Overlays** | Activity monitor, passkey sheet, app windows, the **gesture bar** (gesture, verb, dwell ring) while a hand is seen |

Wide layouts use the full grid. Below 900 px the same shell **reflows into the phone order**:
status bar, lane, field, command bar, then the dock as a bottom bar. A tablet or a narrow
laptop window gets the phone's calm order automatically.

### 4.3 The phone

| Tab | Contents |
|---|---|
| **Now** | Greeting and the Chief WebAgent's team mark (hold to talk); the brief; the **Needs you** stack (swipe = snooze, tap = open); the live strip ("3 on your site, Cakes is answering Mary"); mission widgets |
| **Decisions** | One card at a time: the exact effect, why this risk level, Approve with passkey / Edit / Decline (and teach). Approving here, with the card open, is a **reviewed** approval (§5.3) |
| **Live** | Front of house as a list: who is here, which desk, sit in, take over |
| **Apps** | The dock as a home screen: WebAgents with presence rings and badges, Store, Activity, Missions, Memory, Trust; voice; Freeze |

The lock screen is a **doorbell, not a letter**: "2 decisions are waiting" and "Your morning
brief is ready", with no arguments (`PushDoorbell`, `mobile/doorbell.rs:51`, sends empty
wake-ups that wait out quiet time). Nothing is decided from the lock screen, so the console
has no one-tap approvals there.

### 4.4 WhatsApp, voice and email

- **WhatsApp** (`WhatsAppAdapter` through the kernel's `ChannelGateway`, a paired device):
  - each brief and card shows the WebAgent's mark as a static PNG from the release build,
    padded so the squircle sits inside WhatsApp's round crop (ADR-031);
  - the brief at 07:00;
  - everyday cards with Allow once / Decline. These are one-tap approvals: they work, but
    they never count toward promotion (§5.3);
  - a consequential card answers with a link to the PWA for the passkey, or the step-up code
    without one. It never approves silently;
  - **STOP** freezes;
  - free text goes to the Chief WebAgent, so "pause Sales until Monday" works.
- **Voice:** the Chief WebAgent on any surface with a mic (§3.6). Telephone (SIP) and SMS wait on
  U7.
- **Email:** the brief and the weekly review with links. By design it approves nothing (05 §7).

### 4.5 From six tabs to an OS

| 01 §4.1 tab | Now lives in |
|---|---|
| Today | The Chief WebAgent's brief card and the Night Time timeline; **Now** on the phone |
| Decisions | **The lane**; the Decisions tab on the phone |
| WebAgents | **The dock** and its apps; the **Store**; the **Activity monitor** |
| Website | **Editor's app** (pages, versions, one-tap rollback, Teach your site, "how AI assistants see you") and **Walk the site** |
| Customers | **Front of house** (live conversations, agent door) and the **Sales** and **Cashier** apps (leads, orders) |
| Trust | **The Trust centre** app and the trust pill |

01 §4.1's tab table should point here. The control plane's §7.1 scopes reuse the same shell
(§6).

## 5. The trust centre and approvals

([trust](../concept/console/trust.html))

### 5.1 What it shows

1. **Walls**:
   - the site matches signed release vN (checked N minutes ago);
   - no WebAgent holds a credential;
   - signals crossed today, with no raw text crossed;
   - quiet hours and budget;
   - the invariant line: "Consequential actions taken without the authority you granted: 0.
     Always."
2. **Freeze** (§5.5).
3. **How much each WebAgent may do alone**: the ladder per job, with **locked positions**
   above each job's ceiling and the reason; promotions with evidence from reviewed approvals;
   lowered states with reasons; the Concierge's fixed rules; Guardian's Undo only.
   **Attention settings** sit here too: quiet hours and the daily card cap.
4. **Standing grants you signed** (§5.4).
5. **What your WebAgents believe** (§3.7).
6. **Signed decisions and receipts**, each with **Verify** (§5.2).
7. **Devices and people**: passkeys per device, the authenticator app and the devices that
   approve with codes, paired WhatsApp, agency people and their
   charter roles.
8. **Platform notices**: what Zavora as operator did or is doing that touches the site, as
   plain-words receipts: the release-train rings the site is in, incidents, platform holds
   with their reasons, incident rollbacks (08 §2.4; control-plane O4).
9. **Support access**: requests for an owner-signed support grant (time-boxed, scoped, never
   payments or customer data), with Grant and Revoke, and the agency's charter role with its
   named people.
10. **The agency's name** on every card and receipt an agency person drafted or approved.
11. **Export and erasure, with receipts** (09 §8.2–8.3; data workstream O3):
    - export progress, then the signed bundle (records, site, receipts, analytics);
    - each erasure request, with every store's answer: `Hard`, `Eventual{within}` or
      `External`;
    - the **connected tools that may hold copies** (WhatsApp Business, Google Business
      Profile, the owner's M-Pesa statements…), because Agentrix can't erase inside the
      owner's own tools (domain D10). The owner is told what to do there.
12. **Export everything**, and a reminder that the platform is MIT, so the owner can run it
    themselves.

### 5.2 Approvals: passkey first, a step-up code without one

The platform's promise (02 §8):
- each effect is canonicalised to a fingerprint `H(site, webagent, tool, canonical_args,
  amount, expiry)`;
- the approval is a WebAuthn assertion **whose challenge is that fingerprint**;
- the Effects Gateway verifies it, uses the fingerprint as the idempotency key, and records it
  in the receipt.

The console's job is to make that visible and honest:
- **What you see is what you sign.** The card's effect box and the passkey sheet are both
  rendered from the canonical arguments that are hashed, never from model text.
- **The sheet names the effect** in one line ("Order o-118 · KSh 4,500 · Amani W. · Sat 14
  Nov 10:00") and shows a short form of the fingerprint.
- **Receipts can be verified.** Verify expands to the fingerprint fields, the device and
  credential, the challenge, who checked it (the Effects Gateway), that it ran once, and the
  log chain.

**Without a passkey** (James, 7 Nov, `d-auth-passkey-first`; 02 §8). Passkeys stay primary,
and the first release demo uses them. An owner on a device without one still approves:
- The card shows the exact effect and a link: "No passkey on this device? Use a code from your
  authenticator app."
- The owner types a fresh code. Control binds that single-use code to the fingerprint, and the
  Effects Gateway verifies it against the fingerprint and runs the effect once.
- It proves the owner held the authenticator at that moment. It rests on the platform's
  attestation, not on the owner's own signature, and Verify says so.
- **The receipt names the method:** a passkey, with its device and credential, or an
  authenticator code, with the device and how it signed in.
- **Every other rule is the same.** An approval is `reviewed` only with the card open (§5.3).
  A gesture or voice can raise the sheet but can't enter a code. WhatsApp opens the PWA for
  either method.

**Throughout this document,** "sign with the passkey" and "the passkey sheet" mean this
approval ceremony: the passkey where the device has one, and the step-up code where it doesn't.

**Signing in** follows the same order (02 §8):
- passkeys first;
- without one, email and password, or Google, LinkedIn or GitHub, always with a code from an
  authenticator app, and no magic links;
- sessions bound to the device;
- roles owner, manager, viewer and agency (§5.6).

**Today's gap (F1):**
- adk-surface's passkey challenge comes from webauthn-rs, not from the decision;
- the assertion isn't kept;
- the audit records only `presence: "confirmed"` (`adk-surface/src/engine.rs:2336`).

The claim above needs F1 before any demo says "signed". adk-rust-framework has taken F1 for
the demo, with a target of Sunday morning. If it slips past Sunday midday, the first release go-live
card uses presence-only confirmation and the copy says "confirmed with your passkey", never
"signed".

### 5.3 The autonomy ladder in the console

| Action | How |
|---|---|
| Lower a job | One tap; immediate; receipt |
| Raise a job | A card with evidence → passkey → a signed grant. The evidence counts reviewed approvals only and shows the rest: "18 reviewed · 31 not counted (WhatsApp, digest)". The owner may raise without an offer (it is their authority), but the card shows the evidence and the evaluation status and warns when either is missing. A job whose cards are mostly digested rises only this way |
| Above the ceiling | Locked, with the reason. The only route is a standing rule with all four limits (§5.4) |
| An agent asks | The card is marked "Asked by …". The answer is never a rejection, and asking counts in the job's favour in the weekly review |
| Accept an offered promotion ("Earned it?") | The same card, initiated by the ladder's `offers` |
| Demotion | Automatic (breach, two rejections, incident); shown as "lowered", with the reason, the date and how to raise it again |
| Pause one WebAgent | `PauseAgent` → `JobControl::pause` for each of its jobs (U10); resuming needs the owner |

**Every approval records its kind** (ADR-012 amendment 3). The console writes the kind, the
surface and the input modality (G2) into the decision receipt. The kernel's `JobDecision`
needs a field to carry it (U26, architecture C10).

| Kind | When | Counts toward promotion |
|---|---|---|
| `reviewed` | The card's detail was open in the console (desk or PWA) when the owner approved, whatever confirmed it: a tap, a key, the passkey sheet a thumb-up raised, or a spoken "approve that" | Yes |
| `one_tap` | A notification action, or WhatsApp's Allow once | No |
| `spoken` | By voice, with no card open | No |
| `batch` | Several cards approved together (outside the digest, the console offers no multi-select approve) | No |
| `digest` | From the daily digest card | No |

An approved plan (§5.4) counts once, as `reviewed`. The items it then runs are not decisions.

A review by another WebAgent is not an owner approval at all; the kernel records it, and it
never counts either (05 §2.3).

### 5.4 Standing grants

A standing grant is the only way a WebAgent acts without asking. Each one has:
- a plain-words title;
- **bounds**: argument predicates ("at most 10 a day", "up to KSh 3,000", "returning
  customers only", "never prices");
- an expiry;
- usage counts;
- the device that signed it;
- **Revoke**.

**For capped effects** (messages, posts off the site, meetings with others, money and
deletions), the rule editor requires **all four limits**, and Sign stays disabled until they
are filled:
- **an amount**: per action and in total ("up to KSh 1,000 each, KSh 5,000 a month");
- **the recipients**: named, or a class ("customers with a paid order");
- **a frequency** ("at most 5 a day");
- **an expiry**: the editor suggests 30 days.

The Effects Gateway refuses a rule that lacks any of them. An example in the owner's words:
"The Cashier may refund up to KSh 1,000 each to customers with a paid order, at most 5 a day,
until 31 January."

They come from accepted promotions and from "Always allow" on low-risk cards where the rule
table permits it. On a capped card, "Always allow" opens the editor filled from that card
(this amount, this recipient, once a day, 30 days), never a blank cheque. The Effects Gateway
checks the grant's signature, bounds and expiry before each unattended effect (05 §3.4).
Signing them is **U6 / F9**.

**Approved plans** (ADR-012 amendment 4, decided by James on 7 Nov, `02d2f64`). A job may
propose a bounded plan, such as the Editor's content plan for a week. The owner signs it once,
and its items then run without a card each. The **plan card** arrives in Needs you and shows:
- **the job and the plan:** "Editor · content plan for the week of 9 Nov";
- **each item on one line, with the pages or recipients it touches:** "Post 'Christmas orders
  are open' → /blog", "Add mince pies → /menu", "Email the Christmas order form → the 42
  subscribers who opted in";
- **the four limits for every irreversible item**, in the rule editor's form (§5.4): "Email ·
  to the 42 opted-in subscribers · once · no money · until 15 Nov". An irreversible item
  without all four limits can't be covered. The card marks it "asks you separately", and it
  gets a normal card when its time comes;
- **the plan's expiry**;
- **what stays outside:** "Anything else the Editor wants to do this week asks you as usual";
- **Sign with passkey**, Remove an item, Decline.

After signing:
- the items appear under Done for you as "under plan: the Editor's week of 9 Nov", with undo
  where it applies, and the card shows progress ("3 of 5 done");
- anything outside the plan falls back to the job's dial and gets a normal card;
- **Stop the plan** revokes the items not yet run, like revoking a grant;
- Freeze stops plan items like everything else.

**How it counts and what runs it:**
- A plan is approved only in the console with the card open, so its approval counts once, as
  `reviewed` (§5.3). Its items are not decisions and are never promotion evidence.
- Under the hood the signature compiles to U6 signed rules: an `InSet` of the plan's items, a
  use cap equal to the item count, and the plan's expiry. The Effects Gateway checks them
  before each item runs.

### 5.5 Freeze

| Level | Who | How | What happens |
|---|---|---|---|
| Job | Owner; agency Editor | Ladder or Pause | That job's calls are denied before its next action |
| WebAgent | Owner; agency Editor | App → Pause; voice "pause Sales" | All its jobs pause; running processes park at the next checkpoint |
| **Site** | Owner; agency Editor; operator for abuse or safety, with the reason shown | **Freeze** in the status bar, Trust centre, WhatsApp **STOP**, voice "stop everything" (at once), an open palm held still (after a 5 s countdown in the shell) | `Kernel::freeze` parks every process. The edge serves the signed release and read-only capabilities. The Concierge answers from the pages, then offers the form. Nothing is sent, published or paid |
| Agency fleet | Agency, over its grants | Portfolio scope | A fan-out of site freezes |
| Platform hold | Operator, for abuse or safety (two humans for the fleet) | Operator scope (08) | The edge serves the last signed release read-only, or a signed platform notice page for abuse such as phishing. The owner is told at once, sees the reason in Platform notices, and can appeal |

**Unfreeze needs the owner's approval on the PWA**: the passkey, or the step-up code without
one. Unfreeze resumes exactly the processes the
freeze parked. A platform hold is a **separate** switch: the owner can't lift a platform hold,
and the operator can't lift the owner's freeze (control-plane O3). An operator incident
rollback is allowed **only during a platform hold** and only to the previous signed release
(05 §3.6, §6.2). It is the same reversible act the Guardian may take (A-I11), it is receipted
in the site's Trust log, and the owner rolls forward with the passkey.

### 5.6 People and roles

| Person | Charter role | Powers |
|---|---|---|
| The owner | Owner | Everything, including raising authority, adding apps, granting roles and exporting |
| A teammate | Manager / Approver | Decides the cards their role covers; can't raise authority or grant roles |
| A viewer or bookkeeper | Viewer, Billing | Reads; billing delegates pay invoices |
| An agency person | Editor, Approver or Publisher on this site, only if the owner granted it | Per 08 §2.4: drafts and approves the cards its role covers as a named person; as Editor may lower or pause a WebAgent and freeze the site; runs the agency's ring only for opted-in sites. Never raises authority, exports, deletes or grants roles |
| A Zavora operator | None on the site's charter | Platform rollouts, security fixes, billing state, platform holds, incident rollback during a hold. Never approves a card, changes a dial or reads content without an owner-signed support grant |

Agents never hold a charter role (A-I23; `is_agent_principal`). Approval chains need distinct
people (`check_chain`).

## 6. Multi-site and agency mode

**An owner with several sites:**
- The site chip in the status bar switches sites. An "All sites" home shows each site's held
  decisions, missions, budget and wall status.
- The phone's Now merges them into one stack, with each card clearly marked by site.
- **Each decision is still made on its own site's card under that site's charter.** There is
  no cross-site approval.
- Freeze is per site.

**One app, three scopes, one shell** (agreed with control-plane, O1–O4). Site, Portfolio
(agency or reseller) and Operator (Zavora Cloud) all use the shell of §4.2 and the same code:
`concept/console/console.css` for the shell, the decision card, the passkey sheet, the presence
marks, the ladder and Freeze, on the shared tokens in `concept/styles.css`.
- **The scope switcher is the status-bar chip.** Portfolio carries a teal scope mark. Operator
  carries an indigo-900 mark and an "Operator · Zavora Cloud" chip, so nobody confuses acting on
  the fleet with acting on one site.
- **Each scope's navigation is its dock.** "Today" is the field's default view, "Decisions" is
  the lane, and the scope's lead WebAgent (the site's Chief WebAgent, the agency's, or the Operator Chief WebAgent)
  is the command bar.
- **The two worlds** exist only in the Site scope.

**An agency (the Portfolio scope, 08 §7.1).** The same shell, with the agency's brand where
white-label allows (08 §2.6):
- **Dock apps:** Clients, Sites, Rollouts, Billing, Team, plus the agency's operator WebAgents
  with presence marks. On a client's site, the agency sees that site's apps within its granted
  role.
- **The lane:** the agency's own cards, plus client cards it can act on. A client card that
  needs the owner shows **"Send to client"**, which routes it to the owner's passkey with the
  agency's note.
- **Rollouts:** packs and themes reach a client's site only if the owner opted it into the
  agency's ring. Behaviour-changing packs still need the owner's adoption.
- **Fleet freeze** over the agency's grants. Agency operator WebAgents only prepare and propose
  (08 C5).
- **Never white-labelled:** the AI disclosure, the passkey ceremony and what it shows, the
  receipts and the operator chain, the export button (08 §2.6).

**The operator console** (Zavora) is the third scope on the same shell. Its dock holds Fleet,
Rollouts, Incidents, Customers, Money, Partners and Trust, plus the operator WebAgents.

**Where owner authority ends** (the same line as 08 §2.4):

| | Owner | Agency (only with a granted charter role) | Operator (Zavora) |
|---|---|---|---|
| Approve a site's cards | ✅ passkey or step-up code | ✅ the cards its role covers, as a named person | ❌ never |
| Raise autonomy, add a WebAgent or pack, sign a charter-changing mission | ✅ passkey or step-up code | ❌ | ❌ |
| Lower or pause a WebAgent | ✅ | ✅ as Editor | ❌ |
| Freeze the site | ✅ (only the owner unfreezes it) | ✅ as Editor | Platform hold, for abuse or safety, with the reason shown; a separate switch |
| Roll back a release | ✅ any time | ✅ as Publisher | Only during a platform hold, to the previous signed release |
| Platform version, adk-rust pin, security fixes, billing state | Told, in Platform notices | Told | ✅ through release trains and notice policy |
| Read content, conversations, leads | ✅ | ✅ within the role's scope | Only with an owner-signed support grant (time-boxed, scoped, receipted) |
| Export or delete everything; grant or revoke roles and support access | ✅ | ❌ | Executes the owner's request |

Every operator or agency action that touches a site shows in that site's Trust centre, naming
the person and the organisation.

**Framework need:** one `SurfaceEngine` reads one `Source` today, so an "All sites" home needs
a multi-site source (U15 / F10).

## 7. Key journeys

### 7.1 Buying Agentrix and moving your own site

This is the owner's half of the dual journey; the site customer's half is in 06. The map is
[`concept/journeys/index.html`](../concept/journeys/index.html), and it shows where the two
meet. The story is the same in both lanes. Wanjiru owns Kilele Bakery
(`kilele-bakery.example`). Her first online customer is Njeri, ordering Amina's 2-tier forest
cake from London for KSh 5,250, paid by PayPal in USD at the rate in Kilele's price list.

| # | Stage | What the owner does and sees | What is signed or proved | Surface | Concept |
|---|---|---|---|---|---|
| O1 | **Discover, then assess** (01 rung 1) | Arrives from a "Made with Agentrix" footer, a friend's shared preview, an agency or the Vultr Marketplace. Types one address and watches the site read, rebuilt and answering in about three minutes. Talks to it | Nothing: no account. The preview lasts 7 days and is never indexed | Web | [owner-discover](../concept/journeys/owner-discover.html), [assess](../concept/assess.html) |
| O2 | **"Keep it": create the account** | A passkey first. Without one: email and password, or Google, LinkedIn or GitHub, always with an authenticator app set up on the spot, and no magic links. The preview becomes a site in setup, the old site is untouched, and the Chief WebAgent introduces itself | The passkey, or the authenticator enrolment; a device-bound session (02 §8) | Web, then the PWA | [owner-account](../concept/journeys/owner-account.html) |
| O3 | **Choose a plan and pay** | Three plans with what each includes. The Chief WebAgent suggests Commerce because the site takes paid custom orders, and says why. The AI budget is shown as a boundary the owner sets, beside an honest cost line ("about $4–12 a month to run a site like yours", 01 §6.2). Migration is free. Rails: **PayPal Subscriptions**, a card on Paystack's hosted page, or a monthly M-Pesa prompt | The owner approves the subscription on the provider's own page; no card data touches the platform. **The first charge is on go-live day.** An owner who never goes live is never charged (08 §4.1). **The fleet release** | PWA, provider page | [owner-plan](../concept/journeys/owner-plan.html) |
| O4 | **Claim the domain** | Proves control with one uploaded file or one DNS TXT record, guided for the owner's registrar. The Caretaker checks and says what it found. Nothing changes on the live site yet | Domain proof, recorded. **Agentrix never asks for the registrar password** | PWA | [owner-domain](../concept/journeys/owner-domain.html) |
| O5 | **The Move** (rung 2) | Replay in a sealed room, cleaning with quarantine in plain words, and the parity proof of at least 95%, watched in the Activity monitor | Sandbox receipts; parity report; quarantine list | PWA | [owner-move](../concept/journeys/owner-move.html), [move](../concept/move.html) |
| O6 | **Go live** | One card: release, parity, what was left behind, the DNS switch, and "your Commerce plan starts today". Approve, then the site is live in seconds. Undo is one tap for 30 days | **Passkey, or the step-up code without one** (§5.2); the receipt names the method | PWA, phone | [owner-golive](../concept/journeys/owner-golive.html) |
| O7 | **Day 1 and the first brief** (rung 3) | The first Night Time timeline, then the 07:00 brief on WhatsApp, email and the console, built only from recorded activity, with what needs her | — | WhatsApp, phone, desk | [owner-day1](../concept/journeys/owner-day1.html) |
| O8 | **The first customer order** | Njeri's custom order arrives as a card (the Concierge held `request_custom_order`). Wanjiru approves it with the card open. Njeri pays by PayPal, and the card turns "Paid". The payout and PayPal's fee then appear in the books, with gross, fee, exchange rate and net. A refund under the Cashier's signed rule runs alone and shows in the brief; above it, a card. **The payment, payout and refunds are the fleet release** | The order approval (`reviewed`); the payment through the Effects Gateway; ledger postings | Phone, desk | [owner-order](../concept/journeys/owner-order.html) |
| O9 | **Billing and ownership** (the fleet release) | Invoices and receipts (eTIMS), AI spend against the budget, changing plan, cancelling, and **exporting everything** (the platform is MIT, so the owner can run it herself) | Plan changes and cancellation as cards; a signed export bundle | PWA | [owner-billing](../concept/journeys/owner-billing.html) |

**Where the two journeys meet** (the map's meeting points): **the order card** (O8, Njeri's
request held for Wanjiru), **the payment** (Njeri pays; Wanjiru sees it settle), **the payout**
(PayPal or M-Pesa to Wanjiru, fees in the books) and **the refund** (under the signed limit it
runs alone; above it, a card).

**Billing, as agreed with control-plane** (08 §4.1–4.2, `07455e9`; the policies are
hypotheses, like dunning):
- **PayPal Subscriptions carry the plan fee.** The owner approves on PayPal's own page. The
  Effects Gateway creates, revises and cancels the subscription as effects with receipts.
  Verified webhooks post each payment against its invoice. Overage, other one-off charges and
  payouts take Steward's governed path, never the subscription.
- **Plan changes.** An upgrade applies at once, and the prorated difference is a one-off charge
  through the rail's one-off path (Steward for PayPal, Paystack's page for cards, an STK prompt
  for M-Pesa). A downgrade applies at the next period, and its card lists what stops then.
- **Cancelling.** Full service to the end of the paid period. Then 30 days read-only: the last
  signed release serves, the WebAgents stop and nothing is billed, so the owner can move DNS.
  Then the site closes, and the data is kept 90 days, then deleted with notice and a receipt.
  Undo works until closure, and there is no refund for the unused part of a period.
- **The first charge is on go-live day.** The plan and subscription are approved at "Keep it".

**The first release and PayPal** (`d-paypal-first-scope`): this weekend PayPal is in the journeys and
mockups only. The first release build and video end the custom order at Wanjiru's approval. The plan
payment (O3), the payment, payout and refunds (O8), and billing (O9) are labelled the fleet release.

### 7.2 Running the site

| # | Journey | Surfaces | Proves |
|---|---|---|---|
| J1 | 07:00 brief on WhatsApp → open the order → passkey → done; the price update next; phone down in about two minutes | WhatsApp, phone | Two minutes a day; the passkey on effects |
| J2 | "Put the Christmas menu up and tell last year's Christmas customers" → plan preview → Go → two cards arrive → signed | Desk command bar | Chief WebAgent planning; nothing contacted before signing |
| J3 | Add Sales from the Store: read can / never, lower a job, set a budget, sign the preview digest → the icon drops into the dock → first look from real data | Desk or phone | Apps, generated listings, `Kernel::agents` |
| J4 | Give a mission in words → read the contract → approve → Monday report with what didn't work → approve the next step | Voice or text, then the lane | Missions; honest reporting; no authority raised |
| J5 | A visitor asks for a person → sit in → take over → reply → hand back → "Teach your site" → sign the FAQ release | PWA | Live takeover (U21); the wall holds; owner words become `Known` |
| J6 | Walk the site by voice: "these prices are wrong", the Chief WebAgent points, Editor drafts, "publish it" → the passkey sheet appears → signed | Desk with mic | Co-browse; voice can't sign |
| J7 | After closing, the shell turns to Night Time; nothing rings; the morning timeline matches the brief | Desk, phone | Quiet hours; honest brief |
| J8 | A guardrail breach lowers Reputation automatically; the owner sees "lowered", the reason and how to raise it | Trust centre | Demotion is visible; raising is signed |
| J9 | "Earned it?" → evidence (reviewed approvals only) → Allow and tell me → signed grant with bounds → the next brief lists what Sales did under it | Trust centre, brief | Earned autonomy; signed standing grants |
| J10 | Reply STOP on WhatsApp → every WebAgent parks; the site serves the release; the owner unfreezes later with the passkey in the PWA | WhatsApp, PWA | Freeze from anywhere; unfreeze only with a passkey |
| J11 | Activity monitor: "why did Sales wake at 04:10?" → wake key, spend, parked on approval → pause Sales | Desk | Task manager; sleep is free |
| J12 | Agency: a client card → "Send to client" → the owner signs on their phone → both see the receipt | Portfolio scope, phone | Agency roles; per-site charter |
| J13 | Models unavailable: the brief says so, the desks rest, the owner edits a price by hand in Editor's app and signs the release | Desk | Honest degradation; apps usable by hand |
| J14 | Hands floury at the counter: turn on the camera → swipe to the go-live card → hold a thumb up → the passkey sheet appears → a tap on "Use passkey" and Touch ID with a clean knuckle → live. Later, an open palm held still → the shell's 5 s countdown → cancel with Esc | Desk, camera | Gestures navigate and raise the sheet; only the passkey approves; held to confirm |
| J15 | Monday: the Editor's plan card for the week (two posts, a menu change, one email to the opted-in subscribers with its four limits) → open → remove one post → sign once with the passkey → the items appear under Done for you as "under plan" through the week; a price change the Editor wants on Wednesday is outside the plan and gets a normal card | Desk or phone | One signature for bounded recurring work; nothing outside the plan rides on it |

## 8. Metrics

Targets are pilot hypotheses, to be measured weekly. None of them is a result.

**Every metric is an entry in the metric catalogue** (09 §6.1, data workstream O1), which is
served by `agentrix-analytics` as `metric(id, site, window, breakdown)` and versioned and
reviewed like a playbook. Each entry has:
- an id;
- a plain-words definition;
- its source: the site's records, its lake slice, or both;
- a minimum sample, below which the console says "not enough data yet";
- the words used to show it.

**Provenance is shown on every number:** its window, its method in one line, and a link to the
rows or receipts behind it. The owner sees the same numbers in the console, in the brief and in
the Chief WebAgent's answers, because all three call the same catalogue (§3.2).

**Charts** are small inline SVGs (a sparkline, a bar, a funnel), rendered by the console server
in `console.css` tokens, so no chart library ships to the phone (data workstream O2). adk-ui's
chart tools come in only after a port to adk-rust v3, and only for charts inside the Chief
WebAgent's replies.

The ids below are proposed catalogue ids.

| Kind | Catalogue id | Metric | Source | Pilot target (hypothesis) |
|---|---|---|---|---|
| Owner effort | `console.owner_minutes_week` | Minutes in the console per week (all surfaces) | Lake (surface sessions) | < 15 (01 §8) |
| Journey | `journey.owner_funnel` | Assessments → Keep it → plan chosen → domain proved → live; median time from Keep it to live | Records (accounts, subscriptions, releases) | Measured; a stage that loses owners is a design bug |
| Owner effort | `console.time_to_clear` | Median time from the morning notification to "nothing needs you" | Lake (`fact_decision`, surface events) | ≈ 2 minutes |
| Reach | `console.decisions_by_surface` | Share of decisions made from the phone or WhatsApp | Lake (`fact_decision`) | Measured, not targeted: shows the phone-first bet |
| Trust | `trust.corrections_per_100_actions` | Corrections per 100 WebAgent actions | Lake (`fact_webagent_action`) | Falls month over month (01 §8) |
| Trust | `trust.approved_unchanged_by_job` | Reviewed approvals unchanged, per job (the promotion evidence), with the share of approvals by kind | Lake (`fact_decision`) | Rises; feeds promotion offers |
| Trust | `trust.asked_when_unsure` | Agent-raised cards per job, and the owner's answers | Records (decisions) | Measured; asking counts in the job's favour |
| Trust | `trust.promotions_demotions` | Promotions offered vs accepted; demotions | Records (ladder) | Both exist, so the ladder is used |
| Trust | `trust.unauthorised_consequential` | Consequential actions without the authority granted | Records (receipts) | **0, always** (invariant; shown only if not 0) |
| Live | `live.takeovers_per_100` | Takeovers per 100 conversations; time to take over after "wants a person" | Records (threads) | Measured; takeover should be rare and fast |
| Live | `site.gaps_closed_week` | Teach-your-site gaps closed per week | Records + lake (`content_gap`) | Grows in month 1, then falls as the site learns |
| Background | `missions.on_time_and_met` | Missions that report on time; share reaching their measure | Records (missions) | Measured; honesty over vanity |
| Attention | `attention.p1_week`, `attention.dropped` | P1 interrupts per week; lane items dropped by budget | Records (`AlarmRouter`, attention) | Low and explainable |
| Attention | `attention.cards_per_day`, `attention.held_by_cap` | Individual cards per day; cards held for the brief by the cap; digest items | Records (attention) | Under the cap on most days; tunes the default |
| Calm | `attention.quiet_nights` | Share of Night Time periods with no interrupt | Records (`AlarmRouter`) | High; owners should sleep |
| Gestures | `gestures.corpus_scores` | Recall, wrong-verb rate and false commits per hour, on the corpus | CI results (not a site metric) | §3.9 targets; 0 false state-changing commits in the corpus |
| Gestures | `gestures.cancelled_or_undone` | Palm countdowns cancelled, and gesture commits undone within 30 s | Records (`surface.gesture` in the site's audit trail) | Low; a rising rate means a threshold is too eager |
| Performance | `perf.console_slos` | Console interaction < 100 ms and approval → effect started < 2 s (02 §9); desk-to-desk handover shown < 300 ms (06 R2); plan preview first step < 1.5 s (proposed) | Telemetry (operator SLOs, not the owner's catalogue) | Enforced as SLOs |

The **North Star** stays the product's: customer moments handled well, per site per week
(01 §8), as catalogue entry "Customer moments handled well" (09 §6.1).

## 9. What the console needs

### 9.1 From the architecture (agreed with webagent-architecture)

| # | Need | Status |
|---|---|---|
| C1 | The Chief WebAgent is the shell; all owner text enters through it; the verb router never calls an effect | Accepted, amended (05 §4.2) |
| C2 | Two worlds = two partitions. The owner reads public threads rendered as untrusted; no one-tap forward to the Chief WebAgent | Accepted, amended |
| C3 | Store listings generated from `WebAgentSpec` or a pack bundle; adding = a passkey signature over a `Kernel::agents` preview digest | Accepted, amended (05 §2.1) |
| C4 | Missions as a platform type over charter goals, agenda and triggers; measure-only until a baseline; never raise autonomy; not one long `Goal` loop | Accepted, amended (05 §4.6) |
| C5 | Live takeover: a per-thread speaker, server-labelled, PWA only (U21) | Accepted, amended (05 §4.5) |
| C6 | One presence vocabulary: 06's R5 enum, plus process-table states. ADR-031 adds `speaking`, driven by the real TTS level | Accepted, amended (`508828a`) |
| C7 | Five-step ladder per job, "lowered" state; lowering immediate, raising a signed card. Since ADR-012's 7 Nov amendments: ceilings by effect class, shown as locked positions; promotion counts reviewed approvals only; agent-raised cards | Accepted, amended (`dce2496`) |
| C8 | Attention: interrupt / lane / digest / drop = P1 / P2 / brief / P3; quiet hours drive Night Time. Since 7 Nov: a daily digest card for trivial cards, and a per-site daily card cap that the attention manager owns (proposed default 5); over the cap, cards wait for the next brief, except P1 | Accepted, amended |
| C9 | Agency: an "All sites" home over many TenantKernels (U15); per-site cards; "Send to client" | Accepted |
| C10 | Decision provenance: each approval's kind (`reviewed`, `one_tap`, `spoken`, `batch`, `digest`) is recorded in the receipt and carried by `JobDecision` (U26). "Reviewed" means the card's detail was open, however it was confirmed | Accepted (`46d8fa4`) |
| C11 | Two exceptions to the card cap: never hold a card past `expires_at` minus 1 hour; deliver individually a decision that a live visitor thread is parked on (`Wake::Approval`), never on the agent's say-so. Held cards stay visible in a collapsed row | Accepted (`46d8fa4`) |
| C12 | Marks on other surfaces: wherever a mark falls under 3:1 against a surface that isn't its site's background, the host surface draws a 2 px keyline in its own ink and never recolours the mark | Accepted (`b079f19`) |
| C13 | The team mark's amber dot follows delivered cards in Needs you (cards the cap holds don't light it; P1 and customer-blocking decisions always do). A WebAgent's own mark shows `waiting` whenever its process is parked on the owner | Accepted (`b079f19`) |

**Gestures (sent to webagent-architecture, A1–A5):**
- A1: gesture verbs are VerbUI verbs, either shell-local or adk-surface `Command`s; C1's router
  rule is unchanged.
- A2: the semantics of snooze for a held decision (05 §7).
- A3: an `Origin::Gesture` row whose `execute` accepts only `StopAll` and `PauseAgent` (U22).
- A4: a `surface.gesture` record in the site's audit trail, not the ledger.

All five were accepted (A1 and A5 as written, A2–A4 amended; 05 at `9a95385`).
- A5: a gesture row in 05 §7's surfaces table.

**Platform types the console reads** (in `agentrix-types`):
- `WebAgentSpec` and pack listings;
- `Mission` (contract, report);
- a **plan preview**: the steps, their jobs and levels, the decisions to come, the cost
  estimate;
- **wall counts** per day: signals by kind, raw-text crossings (always 0);
- a **brief** and a **mission report**, each built from recorded activity, with citations to
  ledger rows and receipts.

### 9.2 From the framework (sent to adk-rust-framework)

Paths are relative to the adk-rust checkout.

| # | Crate | Gap today | Ask | Urgency |
|---|---|---|---|---|
| F1 | `adk-surface` | The WebAuthn challenge comes from webauthn-rs (`mobile/webauthn.rs:104-110`), not from the decision. The assertion isn't kept, and the audit records only presence (`engine.rs:2336`) | Derive the challenge from (decision id, version, answer, args digest); keep the assertion; write its reference into `OwnerDecision.signed_note` / `CommitDecision.signed_note` (`adk-kernel/src/charter/store.rs:80`) | **Demo**: taken; target Sun morning |
| F2 | `adk-surface` | The kernel serves `/kernel/processes` (`ProcessView`: spend, burn rate, goal, tool calls), `/kernel/events` and per-process suspend and resume, but `SurfaceEngine` projects none of it | `SurfaceEngine::processes()` plus pause and resume commands | **Demo**: taken; target Sat night |
| F3 | `adk-surface` | `AgentStatus` (`model.rs:44`) is coarse and refreshed by polling | A pushed `FeedFrame::Agents` from `KernelEvent::Process`/`Held`, with a working label from the running tool's name | **Demo**: taken; target Sat night |
| F4 | `adk-server` (`awp`), `adk-website` | No person path into a public thread (= U21); lead handoff is an async alarm | A per-thread speaker, server-authenticated takeover and hand-back, audit records, visitor notices | Pilot |
| F5 | `adk-kernel` | No entity links goal, agenda items, triggers and KPI (`Charter.mission` is a string) | A mission projection over them, with progress and reports for the surface | Pilot (platform type first) |
| F6 | `adk-surface` | Partial text is dropped (`project.rs:603`) | `FeedFrame::Partial`, for a desk answering live | Pilot |
| F7 | `adk-website` | `VoiceHub` exists, with barge-in, the cancel window and `voice_cant_approve`, but the Steward/Chief WebAgent isn't wired to it | Build the Chief WebAgent harness with `Pack::Voice` | Pilot |
| F8 | `adk-surface`, `adk-website` | UI words are English constants (= U5) | A locale catalogue; Swahili first | Pilot (Kenya) |
| F9 | `adk-harness` | `StandingRule` records its creator but isn't signed (= U6) | Signed standing rules, verifiable by the gateway | Pilot |
| F10 | `adk-surface` | One `Source` per engine (= U15) | A fan-out source over many TenantKernels for "All sites" | Agency |
| F11 | — | No co-browsing primitive | Pointer events over the release anchor map (U18) are enough. Later, voice `VisualInputs` for "look at this" | Later |
| F12 | `adk-kernel` | No app manifest lifecycle; listings are platform data | Keep listings in the platform (`WebAgentSpec`); converge on signed pack bundles, `DriverManifest` and `AgentRecord` fields | Later |
| G1 (= U22) | `adk-surface` (`origin.rs`, `rules.rs`, `engine.rs`) | No gesture origin; `PauseAgent` shares `may_change_agents` (web and terminal only); `DecisionCard` lacks `expires_at` | `Origin::Gesture` with its own `Row`: no `allowed_answers`; `execute` accepts only `StopAll` and `PauseAgent`. A pause gate of its own, open to every human row. `expires_at` on `DecisionCard` for snooze limits. No framework delay on stops: the countdown is the shell's | Pilot (the first release uses the web origin) |
| G2 | `adk-surface` | Commands don't record the input modality | A content-free modality on commands and audit (web, voice, chat, gesture), for receipts and the ledger | Pilot |
| G3 | — | Deliberately nothing camera-side: gestures never use `VisualInputs` or the voice camera channel | If adk-surface's web module serves the console, it must serve the pinned MediaPipe WASM and model from the same origin, with a CSP allowing `wasm-unsafe-eval` and no third-party fetches | Pilot |

Existing upstream items the console also depends on: U1 (roster as data), U8 (time-zone cron
for Night Time), U10 (pause per agent), U17 (several desks per edge), U18 (anchors for the
pointer).

### 9.3 Built in the platform

- **`agentrix-console`**: the PWA shell (status bar, dock, worlds, field, lane, command bar,
  overlays) over adk-surface's web and mobile surfaces.
- The plan preview, the wall view and the Night Time timeline.
- The Store, rendered from `WebAgentSpec`s and pack bundles.
- The mission projection and reports.
- Rendering decision effects from canonical arguments (§5.2).
- The JavaScript build of the mark generator, for the Appearance preview, golden-tested against
  the Rust release build (ADR-031).
- The `console.css` tokens and the shared presence marks, kept in step with
  `concept/experience/experience.css`.

## 10. Build order

| Stage | Console scope |
|---|---|
| **The first release, Vultr demo (target Sun 8 Nov, 20:00 EAT; lablab closes Mon 9 Nov, 02:00 EAT)** | One site. The go-live card and the quarantine decisions **signed with the passkey (F1)**, the Activity monitor (F2), presence in the dock (F3), the brief, Freeze. The shell layout from the concepts, with the Back office world and the lane. Front of house shows a read-only list of conversations. Gestures: camera opt-in and pill, swipe, thumb up → passkey sheet, open palm → stop everything (§3.9). Missions, Store, takeover and voice appear only as labelled concepts. If F1 slips past Sunday midday, the go-live card says "confirmed with your passkey" |
| **Pilot (Dec–Jan)** | The phone (Now, Decisions, Live, Apps); WhatsApp brief, cards and STOP; the Store with Sales, Reputation and Cashier; missions; takeover (U21); Chief WebAgent voice (F7); Swahili (F8); signed grants (F9) |
| **Fleet (Q1 2027)** | Portfolio and Operator scopes (08 §7.1); "All sites" (F10); packs through release trains; walking the site with richer pointing |

## 11. Decisions and open questions for James

1. **Name.** Call the owner console "Agentrix OS" in the product, as 01 §4.1 already implies,
   with the Chief WebAgent as its shell? (Proposed.)
2. **The Night Time theme.** Should the shell switch to the dark field during quiet hours, or
   only when the owner chooses it? (Proposed: automatic, with an override.)
3. **Desk-first or phone-first default.** The phone order stays the default below 900 px.
   Should the wide shell open on Back office or on the brief alone? (Proposed: Back office with
   the brief card first.)
4. **Missions approval.** A passkey for every mission, or only for missions that change the
   charter (05 §4.6)? (Proposed: only charter-changing ones; others are an approved agenda
   item.)
5. **Gestures** (decided: they stay, `d-gestures-key`). Still open:
   - Should a **wave** read the brief aloud? It is left out of v1 because people wave while talking.
   - Should the phone's front camera offer gestures too, or only desk and tablet? (Proposed:
     desk and tablet only.)
6. **The first release copy.** Until F1 lands, say "confirmed with your passkey", not "signed".
7. **The daily card cap.** Proposed default: 5 individual cards per site per day, changeable by
   the owner. webagent-architecture accepts it as the pilot's starting point.

## 12. Alignment record

**Naming:** James decided on 7 Nov that the lead WebAgent's owner-facing name is **Chief
WebAgent** (hub decision `d-coordinator-name`). Its runtime principal stays
`urn:adk:agent:{site}/coordinator`, so receipts, rules and stores are unchanged. This document
and the concepts use the new name throughout; "Mother" is never used owner-facing.

Agreements and open items are recorded in `owner-console-alignment-2026-11-07.md` in the top
session's notes folder. In summary:

- **webagent-architecture:** C1–C9 accepted (C1–C5 and C7 with amendments). Folded into 05
  §2.1, §4.2, §4.5 and §4.6.
- **website-experience:** V1–V6 agreed. The console uses:
  - the same mark values and pointer geometry (§3.5, §3.6);
  - per-audience presence words, with the real reason shown to the owner only;
  - "Front of house" for the owner and operator, "Front desk" for visitors;
  - the takeover lines ("Wanjiru from Kilele joined · a person", "Cakes is back · AI"), an idle
    desk mark while the owner holds the thread, and an automatic hand-back after about 10
    minutes;
  - passkey signing for may-promise lists, capability subsets and memory scopes.

  06 §4.3 records what is shared with the console.
- **adk-rust-framework:** F1–F3 taken for the demo (F2 and F3 by Saturday night, F1 by Sunday
  morning with a presence-only fallback), F4–F9 for the pilot, F10–F12 later.
- **data-architecture (09):** O1–O3 accepted. §8 is the metric catalogue with ids,
  provenance and server-rendered SVG charts. The Chief WebAgent answers numbers only through
  `analytics.metric` (§3.2). The Trust centre shows export and erasure progress with per-store
  receipts and the connected tools that may hold copies (§5.1).
- **Autonomy-dial amendments** (ADR-012 and 05 §2.3, `dce2496`): amendments 1–3 are folded in.
  They cover locked positions and the four-limit rule editor (§3.4, §5.4), agent-raised cards
  and the digest card (§3.5), the daily card cap (§3.5), and reviewed approvals with their
  recorded kind (§5.3). webagent-architecture accepted the two cap exceptions (C11, on the
  condition that a blocked customer is known from the parked thread) and provenance (C10) in
  `46d8fa4`, and took the default of 5 cards a day as the pilot's starting point. Amendment 4,
  plan approval, was decided by James on 7 Nov (`02d2f64`) and is in §5.4 as the plan card.
- **Living marks** (ADR-031, `508828a`): the four layers and `speaking` are in §3.5, the team
  mark is in §3.2, the Appearance editor is in §3.4, and WhatsApp images are in §4.4. C12
  (keylines on other surfaces) and C13 (the team mark's amber dot follows delivered cards) were
  accepted in `b079f19`.
- **Sign-in and approvals** (`d-auth-passkey-first`, 02 §8 at `44caae1`, hub row
  `r-platform-auth-to-console`): passkeys first. Without one, sign-in is email and password or
  Google, LinkedIn or GitHub, always with an authenticator code, and approval is a single-use
  step-up code bound to the fingerprint. The receipt names the method (§5.2). The concepts' approval
  sheets offer the code, and the Trust centre shows a receipt approved with one.
- **The owner journey** (coordinator, hub `r-platform-journey-owner`): §7.1 maps buying
  Agentrix through running the site, O1–O9, with pages in `concept/journeys/owner-*.html`.
  website-experience owns the map, the chrome and the customer lane; the story and the
  meeting points are shared. control-plane answered the billing questions in 08 at `07455e9`.
- **Gestures:** James decided they stay (`d-gestures-key`, requirement `r-gestures-work`).
  §3.9 is the redesign. A1–A5 have gone to webagent-architecture. G1–G3 for adk-rust-framework
  are recorded in the hub, because the session's message limit stopped the direct message.
- **control-plane:** O1–O4 settled and confirmed.
  - All five control-plane concepts now run on this shell: they link `console.css` and use the
    scope chip, the dock with operator WebAgents, the field, the lane and the command bar.
  - Portfolio uses a teal chip; Operator uses indigo-900 with "Operator · Zavora Cloud".
  - 08 §2.4 matches §6 here: separate freeze and hold, rollback only during a hold.
  - 08 says "Trust centre", "Chief WebAgent" and "Operator Chief WebAgent".
  - 08 §7.1 points to `concept/console/`.

  Its confirmation of the amendments is pending.
