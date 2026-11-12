# Agentrix WebAgent Platform: Product

> **Move your website. Meet your WebAgents.**
> Agentrix turns a small business's website (WordPress, Wix, Squarespace or Shopify) into a
> WebAgent: a website that answers every customer, sells, publishes, defends and maintains
> itself, and asks the owner before anything that matters.

Status: product definition v0.1, 7 Nov 2026. It supersedes the Agentrix OS and Agentrix Website
proofs of concept, keeps what they proved, and drops the shortcuts they took. Companion
documents: [02-architecture.md](02-architecture.md), [03-trust.md](03-trust.md),
[04-vultr-agent-rush.md](04-vultr-agent-rush.md), [05-webagents.md](05-webagents.md), and the UI
concepts in [`/concept`](../concept/).

---

## 1. Why now

Three changes make the brochure website obsolete for a small business:

1. **The website became a liability.** WordPress runs 39.4% of all websites (W3Techs, 7 Nov 2026). For
   a small business that means plugins nobody updates, a theme nobody can edit, and a
   hosting bill for a page that waits. Security is infrastructure the owner does not control
   and cannot see.
2. **Customers expect an answer now.** Questions arrive at night, in different languages,
   through forms that land in an inbox nobody watches. Every unanswered question is a sale
   that went to whoever answered first.
3. **Half the audience is not human.** About half of web traffic is now automated (Imperva),
   and more of it each month is AI assistants that shop, compare and book for their users. A
   site that cannot speak to them clearly loses them.

AI site builders generate a site and then leave the owner alone with it. Chat widgets bolt a
bot onto a passive site. Agencies are slow and priced for larger budgets. **Nobody moves the
existing business onto a website that does the work, under the owner's authority.** That is
the gap.

## 2. Who it is for

| | Owner | Agency | Self-hoster |
|---|---|---|---|
| Example | A bakery, salon, clinic, consultancy or boutique with a WordPress site built years ago | A studio maintaining 20–200 client WordPress sites on care plans | A developer or business running WordPress on Vultr's Marketplace |
| Today | Leads arrive on WhatsApp; the site is out of date; they fear it was hacked; they pay a "web guy" when something breaks | Maintenance margins are falling; clients ask "what about AI?" | Patch, back up, repeat |
| Wants | Customers answered, bookings made, the site true and safe, with two minutes a day of attention | A product to sell that keeps clients and raises margin | A modern stack they can own and run |
| Buys | A plan from Zavora Cloud | Per-site wholesale, white-label | The open-source platform (free), with paid support |

The examples are illustrative. Validating them is the first job of the pilot programme (§10).

**Jobs to be done, in the owner's words:**
1. "Answer every customer instantly, in their language, with the right facts."
2. "Turn visitors into bookings, orders and leads without me doing the routine."
3. "Keep my site true, fresh, fast and safe, without me touching WordPress."
4. "Get found by people and by the AI assistants they ask."
5. "Know what happened and stay in charge, in two minutes a day."

## 3. The experience: a ladder of "wow"

The product has to earn a "wow" in the first three minutes, before signup and without
credentials, and then again every morning. Each rung is designed to be shown to someone else.

### Rung 1: "That's my business" (0:00–3:00, no account)

| Time | What the owner sees | What is actually happening |
|---|---|---|
| 0:00 | One field: *"Your website address"* | Nothing yet |
| 0:10 | **"Reading your site"**: pages appear as they are found, each opened in a sealed room shown on screen (no network beyond your site, no keys, destroyed after) | A polite, identified, rate-limited public crawl in sandboxes |
| 0:45 | **Your site's health, honestly:** speed, mobile, how AI assistants see it, and security signals visible from outside (e.g. outdated core, exposed admin paths, hidden spam links). Every finding has evidence; nothing is guessed | Assessor agent + detectors over public evidence only |
| 1:30 | **Your new website, alive:** the same content, faster, cleaner, perfect on a phone, on a private preview link | The Mover rebuilds from the crawl, and the parity score is shown |
| 2:00 | **Talk to it.** "Do you deliver on Sundays?" It answers from *your* pages, with the page it used. When it doesn't know, it says so: *"Your site doesn't say. Want to teach it?"* | The Concierge is grounded on the crawl. "I don't know" is a feature |
| 2:30 | **Meet your WebAgents:** cards for the Concierge, Sales, Editor and Guardian, each with a first suggestion drawn from your real site: *"Your price list says 2023. Confirm the prices?"*, *"Three pages link to pages that no longer exist."* | Real findings, not templates |
| 3:00 | **"Keep it"** → create the account with a passkey; without one, email and password or Google, LinkedIn or GitHub, with MFA | Preview becomes a site in setup |

The preview link is **shareable** ("look what it made of my site"). That is the first referral
moment.

### Rung 2: "Nothing broke" (day 1, the Move)

The owner proves the domain (one DNS record or one uploaded file, guided). The Move then runs:
- **Replay:** for WordPress, the site's own export (or a database dump) is opened in a sealed
  room with no route out.
- **Clean:** anything suspicious goes to **quarantine** with a plain explanation: *"This hidden
  script sends your visitors to a casino site. We left it behind."*
- **Prove:** a side-by-side check against the old site must reach at least 95%.
- **Go live:** one card in the console. The old site stays as it is, kept as a fallback, and
  **undo is one tap** for 30 days.

Finding and removing a hidden infection the owner never knew about is a powerful moment. It is
also the moment trust is established.

### Rung 3: "My website worked while I slept" (day 2, 07:00)

The first **Morning Brief**, delivered where the owner already is (WhatsApp, email or the
console):

> **Good morning, Wanjiru.** Overnight your site welcomed 41 visitors and answered 9 questions
> (2 in Swahili). **1 cake order** is waiting for your OK (KSh 4,500, collect Saturday). The
> Guardian blocked 212 bot requests. Editor drafted a post about this weekend's offer.
> Spend: KSh 38 of your KSh 1,500 monthly AI budget.
> **[Approve order]** **[See the post]** **[Ask me anything]**

The numbers above are an example layout. Real briefs show only recorded activity, never
filler. The brief has a **share** button that makes a privacy-safe card ("my website did this
overnight"). That is the second referral moment.

### Rung 4: "It's getting better at my business" (week 1 → month 1)

- The weekly review shows customer moments handled, hours saved, revenue the site influenced
  (attributed honestly, with the method shown) and spend against budget.
- Unanswered questions become a **"teach your site"** list, which the owner clears in seconds.
- **Earned autonomy:** *"Your Concierge answered 212 questions with no corrections this month.
  Let it handle booking changes without asking?"* The owner grants autonomy step by step,
  with evidence. It is never assumed.

## 4. The three surfaces

### 4.1 The owner's console (Agentrix OS, rethought for a business)

> **Superseded in depth by [07-owner-console.md](07-owner-console.md)**, which reinvents the
> console as Agentrix OS for website owners: everything is an agent, WebAgents as apps,
> background missions and live agents, still in charge in two minutes a day on a phone. The
> summary below is the v0.1 baseline; 07 governs where they differ.

The Agentrix OS proof of concept showed that one coordinator, a decision inbox and per-agent
authority make agents governable. The product keeps those ideas and drops the spatial field,
gestures and 3D stage as primary navigation. A business owner needs **calm, two-minute,
phone-first** control.

| Tab | Purpose | Key elements |
|---|---|---|
| **Today** | What happened, what needs me, what's next | Morning brief; live pulse (visitors now, conversations open); next scheduled work; spend against budget; the **Ask** bar ("Ask your Chief WebAgent") |
| **Decisions** | Everything waiting on the owner, one card each | The context, the WebAgent's recommendation, a risk score, a **preview of the exact effect** (diff, message, amount), then **Approve** (confirmed with a passkey, or an authenticator code without one), Edit, or Decline. Declining can create a standing rule: *"Never offer discounts over 10%"* |
| **WebAgents** | The team, by name | Each WebAgent's mandate, recent work, budget used, and an **autonomy dial** per job (Ask me first, Do it and tell me, Handle it within budget, plus Watch and Suggest after a demotion; [05 §2.3](05-webagents.md#23-the-owners-dial-and-the-ladder)); pause one or all; add more through capability packs |
| **Website** | The site itself | Live preview; versions as a timeline with one-tap rollback; pages; the **"teach your site"** gaps; how AI assistants see it |
| **Customers** | People and agents the site served | Conversations (human and AI-agent visitors), leads, bookings, orders; one-tap handover to WhatsApp |
| **Trust** | Proof and control | Security status; every action as a plain-words receipt; what each WebAgent can and cannot do; devices and sessions; **export everything**; freeze switch |

**Channels:** the console is a phone-first web app installable as a PWA. The brief and one-tap
decisions also arrive on WhatsApp and email. Voice ("talk to your Chief WebAgent") is optional
and never required.

### 4.2 The WebAgents

Owner-facing names, defaults and hard limits. The machinery behind them is adk-rust's kernel
charters and autonomy ladder; how each WebAgent is built is in
[05-webagents.md §2](05-webagents.md#2-the-roster). Starting levels are per job, and the owner
signs them in charter version 1 at go-live.

| WebAgent | Mandate | Starts at | Never, without the owner |
|---|---|---|---|
| **Chief WebAgent** | Your single point of contact: briefs you, routes decisions, answers your questions about the business | Do it and tell me | Decides a card for you |
| **Concierge** | Front desk for every visitor and AI agent, 24/7, in their language | Your rules: a fixed policy; orders and bookings over your amount threshold ask you | Promises prices, dates or refunds the site doesn't state |
| **Sales** | Qualifies leads, follows up, quotes, books | Ask me first | Contacts anyone without a consent basis; commits to terms |
| **Editor** | Keeps every page true and fresh; drafts posts in your voice | Ask me first (new pages); within budget (fixes) | Publishes new pages |
| **Reputation** | Replies to reviews and mentions in your voice | Ask me first | Posts publicly; writes fake reviews (ever) |
| **Cashier** | Orders, payments, refunds through your providers (M-Pesa, PayPal, Stripe) | Ask me first; refunds under your signed limit run alone | Moves money beyond your signed limit; sees card or wallet credentials |
| **Guardian** | Security, abuse, tamper detection, instant rollback | Handle it (rollback and blocking only) | Publishes or spends anything |
| **Caretaker** | Domains, certificates, backups, updates, costs | Ask me first; renewals under your signed limit run alone | Changes infrastructure unwatched |

Irreversible or outward-facing work starts at "Ask me first". It runs alone only under a
standing rule you sign that states an amount, the recipients, a frequency and an expiry
([ADR-012](adr/012-autonomy-per-job.md), amendment 1).
| **Mover** | The migration, then hands over | Ask me first (go-live) | Goes live |

### 4.3 The website (Agentrix Website, the product)

What the owner's customers meet:
- **Instant pages.** Static, signed releases served from the edge. No agent on the page-load
  path; target under 1 s to a usable page on a mid-range phone on 4G.
- **"Ask this site".** A first-class part of the page, not a bubble: answers cite the page they
  came from, actions (book, order, request a quote) are real forms the WebAgents operate, and
  handover to the owner happens on WhatsApp when needed. Visitors are told they are speaking
  with AI.
- **A door for AI agents.** The same truth for machines, served through the Agentic Web
  Protocol, MCP and A2A, with verified access, the same prices and stock, and receipts. No
  scraping required.
- **Languages.** The Concierge answers in the visitor's language. Pages can be published in
  more than one, for example English and Swahili, each on approval.

### 4.4 The Move (migration as a show)

Migration is a product moment, not a ticket. The owner watches their site move room by room:
**Read → Claim → Replay → Clean → Rebuild → Prove → Go live → Watch.** Each sealed room shows
what it may touch (network: *your site only*; keys: *none*), its time limit, and *destroyed ✓*
when done. The quarantine list explains every item in plain words, with evidence, and needs
the owner's decision to release anything.

## 5. Trust promises

Printed on the pricing page, enforced in code ([03-trust.md](03-trust.md)):

1. **Nothing changes until you say go.** Your old site is untouched until go-live, and undo is
   one tap for 30 days.
2. **Your WebAgents ask before anything that matters,** and you decide what matters.
3. **Your keys stay yours.** No WebAgent ever sees a password, card or wallet credential.
4. **Every action has a receipt,** in plain words, that anyone can verify.
5. **Your site and data are yours.** Export everything (content, customers, the built site,
   the code) at any time. The platform is open source.
6. **Visitors always know** when they are talking to AI.
7. **Honest numbers.** We never invent activity or results.

## 6. Business model

### 6.1 Open core plus managed cloud

The platform is **MIT open source** (this repository). Zavora sells running it well:

| Offer | Price (hypothesis) | Includes |
|---|---|---|
| **Free assessment** | $0 | Health report and a 7-day living preview you can talk to (not indexed) |
| **Essential** | $29/mo | The site, Chief WebAgent, Concierge, Editor, Guardian, Caretaker, Morning Brief; **$5/mo AI budget included** |
| **Business** | $79/mo | Plus Sales, Reputation, bookings and forms, WhatsApp channel, outreach within budget; $15 AI budget |
| **Commerce** | $149/mo | Plus Cashier, product agents, agent-to-agent commerce; $30 AI budget; payment fees pass through |
| **Agency** | From $15/site/mo wholesale (10-site minimum) | White-label console, client approval routing, a fleet view |
| **Self-host** | Free (MIT) | One-click on Vultr Marketplace; paid support and updates subscription |

- **Migration is free** on every plan. White-glove migration for complex sites is a fixed fee.
- **The AI budget is a boundary, not a meter:** the owner sets a monthly cap, and anything
  above it needs a decision card. This is zavora.ai's "a budget with boundaries" applied.
- **Local pricing:** KES, NGN and ZAR price points, and M-Pesa billing in Kenya.

### 6.2 Unit economics (estimates to validate)

Cost to serve a small site on Vultr is about **$4–12 per month**: Serverless Inference about
$3–9, compute about $1, storage and CDN under $0.50, platform overhead about $1. The range is
wide because Vultr publishes two inference prices: per-model in the Inference API's live
catalog, and a flat $0.55 / $2.75 per 1M tokens in its support docs. The first real bill
settles it. A migration costs about **$5–30** once. At $29–149 per month the gross margin is
roughly 60–90% (lowest on Essential under flat pricing). The derivation is in
[02-architecture.md §10](02-architecture.md#10-cost-model).

### 6.3 Vultr partnership

Every migrated site adds steady Vultr Serverless Inference, compute, storage and CDN spend.
Vultr's Marketplace already offers WordPress, so we offer Vultr's WordPress customers a path
to stay on Vultr while upgrading what their site does. The routes are a verified-vendor
Marketplace listing, the service-provider partner track and Cloud Alliance co-marketing.

## 7. Growth loops

| Loop | Mechanism |
|---|---|
| **Preview share** | Every assessment yields a living preview link owners send to partners and friends |
| **Brief share** | "My website did this overnight": a privacy-safe card from the Morning Brief |
| **Made with Agentrix** | A footer on Essential sites: *"Move your site in 2 minutes"* (removable on paid tiers) |
| **Give a month, get a month** | Credits on both sides of a referral |
| **Agencies** | Agencies migrate their whole book; 20% revenue share on managed plans |
| **The agent network** | AI assistants prefer sites that answer with verified data and receipts; Agentrix sites are listed for agents through AWP. More sites make agents send more customers, and that brings more sites. This is the long-term moat |
| **Marketplace** | One-click self-host on Vultr, then an upgrade path to managed |

## 8. Metrics

| Kind | Metric | Target (pilot) |
|---|---|---|
| Activation | Preview talked to within the first session | ≥ 60% |
| Conversion | Preview → kept | ≥ 15% |
| North star | **Customer moments handled well, per site per week** (answers not corrected, bookings, orders, qualified leads) | Grows weekly |
| Owner effort | Minutes in the console per week | < 15 |
| Trust | Corrections per 100 WebAgent actions | Falls month over month |
| Invariant | Consequential actions taken without the authority the owner granted | **0, always** |
| Referral | Share of new sites from preview, brief and footer loops | ≥ 30% by month 3 |

## 9. What we will not do

- Run a page load through a model. Pages are static and signed.
- Hold the owner's passwords, or ask for WordPress admin access to migrate.
- Let any WebAgent act outside the authority the owner granted, for any reason.
- Show invented numbers, fake reviews, or AI that claims to be a person.
- Lock anyone in. Export and self-host are first-class.

## 10. Plan to prove it

1. **The first release (9 Nov):** the Move on Vultr, end to end for one WordPress site, built for the
   Vultr Agent Rush ([04-vultr-agent-rush.md](04-vultr-agent-rush.md)).
2. **The fleet release (12 Nov):** the Cashier, via Steward, for the PayPal AI Hackathon.
3. **Pilot (Dec–Jan):** 10 sites, starting with zavora.ai (site one, "company number one"),
   classyperfumes.shop and Kenyan SMEs. Measure every metric in §8 weekly.
4. **Launch (Q1 2027):** Agency tier, Vultr Marketplace listing, billing, multi-region.

## 11. Open questions

1. ~~Open core~~: **decided 7 Nov.** The whole platform is MIT; Zavora sells the managed service, packs and support.
2. Brand: "Agentrix" as the product name and Zavora as the company, with zavora.ai's
   AI-startup story as the umbrella?
3. Data residency: Vultr does not publish where Serverless Inference runs. Until Vultr
   confirms the region in writing (James to ask), owners are told it is unconfirmed, and EU and
   African customers get that disclosure at signup. **Decided 7 Nov:** every model call
   (chat, embeddings, rerank, speech and images) goes through Vultr's Inference API; there are
   no self-hosted GPU models (ADR-008).
4. WhatsApp: Business API provider and pricing for the brief and approvals channel.
5. ~~The lead WebAgent's name~~: **decided 7 Nov.** It is the **Chief WebAgent**, in place of
   "Chief of Staff" ([ADR-009](adr/009-webagents-terminology.md)).
6. Fleet learning: whether contributing aggregates to cross-site learning is opt-in or opt-out
   ([ADR-013](adr/013-no-swarm-fleet-learning.md)).
