# Agentrix WebApp Platform: the plan

Owner: the coordinator session, "Agentrix WebApp Platform" (W0). Started Sat 7 Nov 2026.
Live board: (an internal board, not public)

## What we're building

The whole Agentrix WebAgent Platform as a product anyone can use: a small business pastes its
address, sees its site assessed, moves it, goes live with WebAgents, and runs it from the owner
console; its customers browse, ask, register and order; Zavora runs the fleet from the control
plane. The design is done (docs 01 to 09, 31 ADRs, about 60 concept screens in `concept/`). The first
release built the first slice and runs it at https://agentrix.website.

James, Sat 7 Nov, 23:50 EAT: the platform work is part of the Vultr Agent Rush submission,
because the first release shows only a subset of the product, and it may ship to agentrix.website.

## Read first

| File | What it holds |
|---|---|
| [how-we-work.md](how-we-work.md) | Branches, commits, the shared build cache, the first release's file owners, migration ranges, deploy rules, the board protocol |
| [voice.md](voice.md) | How every word on screen is written, and the copy lint |
| [01a-ui-website.md](01a-ui-website.md) | Visitor and customer UI, page by page; the three sample sites |
| [01b-ui-owner-and-operator.md](01b-ui-owner-and-operator.md) | Onboarding, owner console and control plane UI, page by page |
| [02-architecture.md](02-architecture.md) | Production architecture, built vs designed, the WebAgent runtime, UI stack, crate ownership, contracts C1 to C11, order |
| [03-data.md](03-data.md) | The entity model, built vs needed, stores, seeds for the samples, erasure and residency |

## Two horizons

**Horizon 1: live for the submission.** From when the first release spec hands over `main` (after its
live run and the Vultr switch, tonight) until the freeze at **Sun 8 Nov, 17:00 EAT**. Each
workstream ships slices that work on their own, on real data, beside the first release's code rather than
rewriting it. The coordinator merges and deploys them outside the first release's windows (the board's
"When main can move"). Target: by 16:00 Sunday a judge can open the three sample sites, sign up as
a customer and place an order, see the owner admin those orders, follow onboarding from Assess to
a live site, and open the operator's view of the fleet.

**Horizon 2: production.** After the hackathon closes (Mon 9 Nov, 02:00 EAT). The refactor
02-architecture.md lays out: per-site stores, the WebAgent kernel on adk-rust, the Effects Gateway
and key holders, tenancy above the site, the fleet profile, and the production readiness
checklist (02 §8). Moving the first release code into new crates (site/ to `agentrix-edge`, auth/ to
`agentrix-identity` and so on) waits for Horizon 2, because the first release lanes still own those files.

Each session plans both horizons, delivers Horizon 1 first, and keeps going into Horizon 2.

## Workstreams and sessions

Every build session runs in its own worktree, on a branch cut from `platform`, in the sidebar group
"Agentrix WebApp Platform". Ports for local previews follow 02 §5.3 (W1 18100, W2 18200 …).

| | Workstream | Session | Horizon 1 (live by Sun 16:00) | Owns (new code goes here) |
|---|---|---|---|---|
| W0 | Coordination and integration | Agentrix WebApp Platform | Merges, wiring into node, deploys, the board | Root files, `agentrix-node` composition, `agentrix-host` |
| W1 | The individual WebAgent | WebAgent runtime | The roster of nine with living marks, mandates, budgets and per-job dials that the policy path enforces; section desks on the sample sites; the Chief's brief as a WebAgent job | `agentrix-kernel` (new) |
| W2 | Three sample websites | Sample websites | Three sample sites live on `*-sample.agentrix.website`: visitor pages from the concepts, Ask and the agent door, customer sign-up and accounts, orders or bookings, and the owner's admin apps for them | `agentrix-records`, `agentrix-themes` (new), `fixtures/samples/` |
| W3 | Control plane for the platform admin | Control plane | The operator console on its own host with fleet, sites, Moves, spend, receipts and incidents from real data; tenants and plans | `agentrix-operator`, `agentrix-billing` (new) |
| W4 | Onboarding and the owner journey | Onboarding | Discover and Assess through plan, sign-up, claim, Move, go-live and the first brief, every page matching the concepts | `agentrix-onboarding` (new); `agentrix-move` only with the Move builder's agreement |
| W5 | Owner console | Owner console | The Agentrix OS shell from `concept/console/` with Today, Decisions, WebAgents, Website, Customers and Trust on real data, phone first | `agentrix-console` after handover; `agentrix-decisions` (new) |
| W6 | Identity, payments and trust | Identity and payments | Customer accounts on site hosts (passkey first, then email with a one-time code), staff roles for owners, PayPal checkout in sandbox mode, consent and data export for customers | `agentrix-identity`, `agentrix-effects` (new) |
| W7 | Design system and words | Design system and voice | `agentrix-ui` with tokens, layouts, components and the mark generator; `scripts/copy-lint`; a copy pass over every shipped page | `agentrix-ui` (new), `concept/`, `voice.md` |
| W8 | Platform core and data | Platform core | Per-site stores behind traits, the Model Gateway with tokens, metering and the metric catalogue the console and control plane read, export and erasure (ADR-028), health and status | `agentrix-store` (new); `agentrix-models`, `-sandbox`, `-egress`, `-gauntlet` after handover |
| W10 | Home page and brand | Home page and brand | The production home page at https://agentrix.website/, branded "Upgrade your website into WebAgents" as the submission deck tells it: the problem, the Move's seven steps, containment and the live gauntlet score, go-live on the owner's passkey, the site that answers people and their agents, the console on a phone, the architecture on Vultr, the three samples, plans in USD, trust promises; live numbers from receipts; the paste box that starts Assess | `agentrix-home` (new); the home page's copy and imagery |
| W11 | Images and voice on Vultr | Images and voice | Image generation at the centre of every site (hero and product images for the samples, images the Move fills in where an old site has none, sketches in the cake studio, the Editor's images for posts), each screened, stored by digest, labelled as generated and approved by the owner; and the speech cascade for the owner console (push-to-talk to the Chief WebAgent, the spoken brief, once-or-deny by voice for everyday decisions) | `agentrix-media` (new); the gateway's `image` and `voice`/`speak` purposes, with W8 |
| W9 | Messages and notifications | Messages and notifications | Email to owners and customers (sign-up codes, order receipts, the morning brief, decision alerts), web push "a decision is waiting", WhatsApp drawn and ready behind a setting; the catalogue of every message (01b §4.7), each written to voice.md | `agentrix-messages` (new); channel drivers sit behind W6's Effects Gateway |

## James's answers (board, Sun 8 Nov, about 00:00 EAT)

These override the defaults below where they differ.

- **"This is what we will present in the hackathon so align everything properly. The first release was
  foundation, but users expect the UIs we drafted."** Every page people see on agentrix.website
  (start, Assess, sign-up, claim, the Move, the console, the sample sites, the operator console)
  must look and behave like its concept. The first release's pages are brought up to the concepts, not
  replaced by something new, and keep showing the first release's real data.
- **Samples: "look at the UI concepts we had drafted."** Use the businesses the concepts draw:
  Kilele Bakery, Classy Perfumes and Tana Valley Times (the three `concept/identity/marks.html`
  names), with the news pages from `concept/experience/news.html`. Each runs on its own `-sample`
  host with a "Sample" label, so nobody mistakes it for the live classyperfumes.shop.
- **Payments: "PayPal only."** PayPal checkout (sandbox) on the samples and PayPal Subscriptions
  for plans. No M-Pesa or card rails on screen.
- **Currency: "USD"** for plans and platform billing. Site prices stay in each site's own
  currency as the concepts draw them (Kilele in KSh).
- **Recovery: must have, including email one-time codes**, for owners and customers, alongside
  recovery codes.
- **Look: "follow"** the concepts exactly (buttons, fonts, Night Time).
- **One binary: "refer to the architecture"**: one server binary with roles (02 §1).
- **Disk: "manage space as you work."** The coordinator manages disk for the program.

**Added by James, Sun 8 Nov, about 00:35 EAT:** "image generation as central to the solution: a
site without images looks very dull"; "an end to end speech pipeline using the Vultr models,
especially for the owner console, in addition to the gestures"; and "a production ready home
page for agentrix website with the agreed branding, Upgrade your website into WebAgents, as per
the artifact [the deck], with architecture and UI." These became W10 and W11.

- **Images** use Vultr's `qwen-image-2.1` (text or image in, image out, edits) and
  `z-image-turbo` (fast drafts) through the Model Gateway's `image` purpose (02 §6, the first release
  design §4.3). Generated images are labelled as generated wherever a visitor might take them
  for a photograph, never show real people or invented news events, pass the content screen,
  live in the content store by digest, and reach a live site only through a release the owner
  approves.
- **Voice** is the cascade in 02 §6: realtime transcription (`voxtral-mini-4b-realtime` over a
  server-only WebSocket the gateway relays; `whisper-large-v3-turbo` for batch), a streamed chat
  completion, then `qwen3-tts` speech streamed back. adk-surface's voice rules hold: voice can
  only allow once or deny everyday decisions, serious ones need a screen, and the recording is
  never kept. The mark's `speaking` state follows the real audio level.

## Decisions taken so we can move, and what James should confirm

These are on the board under "Waiting on James", each with the default we're using now. Build
on the default; when James answers on the board, the coordinator tells the sessions it affects.

1. **One server binary with roles** (edge, control, kernel), not three. Default: yes, as the first release
   runs today. W8 and W9 are separate workstreams (02 §5.1, 01b §7).
2. **The three sample sites.** Default: Kilele Bakery (local services), a perfume shop under a
   new invented name (classyperfumes.shop is a real store, so its name and products stay out),
   and Tana Valley Times (local news, built with The Kestrel Post's drawn patterns; 01a §1.6,
   03 §5). Each is marked "Sample", runs on a `-sample` host, and uses only invented people,
   reserved contact details and sandbox payments. W2 adopts 01a §8's recommendations D1 to D10
   as defaults. zavora.ai stays out: it's a real site.
3. **Payments for the samples.** Default: PayPal sandbox only (lines up with the fleet release, the PayPal
   Checkout Steward due 12 Nov); M-Pesa drawn but switched off until there's a Daraja sandbox
   key.
4. **The first release's live data.** Default: leave `atl-0`'s data as it is; import it into the new store
   layout in Horizon 2.
5. **Disk.** About 32 GB free. Sessions build only their own crates (`-p`). If James frees space
   (the 6.4 GB cache in `~/Developer/projects/adk-rust-wt-webagent/target` is the biggest),
   more can build at once. Nobody deletes it without his yes.
6. **Console look.** Default: follow the concepts exactly (12 px indigo buttons, self-hosted
   Manrope and DM Sans, one dark theme that Night Time switches on by schedule). The first release's pill
   buttons and system fonts give way (01b §5).
7. **Journey order and currency.** Default: account before plan; plan prices in USD with the
   local amount beside them; budgets and briefs in the site's currency. The preview before the
   claim (O1.5) comes in Horizon 2.
8. **Owner recovery** when both passkey and authenticator are lost. Default: recovery codes at
   sign-up, plus email recovery with a 24-hour hold and a notice on every other channel.

## Integration cadence

- Sessions ask W0 to merge a slice when it is done and tested (board: `merges`).
- W0 merges `main` into `platform`, then the slice, runs the whole workspace check from a clean
  export, wires the slice into node, and fast-forwards `main` outside the first release's windows.
- W0 or the control-plane session deploys with `deploy/deploy.sh <commit>`. Every deploy re-runs
  the gauntlet on live sites and must stay at 10/10.
- Freeze from Sun 17:00 to the 20:00 submission: only fixes, through the first release spec.
