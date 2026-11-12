# Agentrix WebAgent Platform: The website experience

Status: design v0.4, 8 Nov 2026 (v0.4 amends §5.1, S1 and §10 for the Cakes desk that talks with a model and draws an idea of the cake, ADR-032; v0.3 adopts the living mark, ADR-031, in §4.3 and passkey-first sign-in in §4.10; v0.2 added §12, the standards we build on). Product: [01-product.md](01-product.md). Architecture:
[02-architecture.md](02-architecture.md). Trust: [03-trust.md](03-trust.md). WebAgents:
[05-webagents.md](05-webagents.md). Concepts: [`concept/experience/`](../concept/experience/).

This document designs what any website becomes after the Agentrix WebAgent Platform migrates
it: the side its **visitors** meet. [01-product.md](01-product.md) designs the owner's side
(the assessment, the Move and the console), and [05-webagents.md](05-webagents.md) designs the
agents. This document is about the experience. It sets out the requirements that experience
places on the agents (§11), which were exchanged with the WebAgent architecture work and
settled in writing.

---

## 0. In one page

A migrated site keeps loading instantly as static, signed pages and keeps looking like itself.
Six ideas change what it can do:

1. **The menu is the team.** Each menu item is a WebAgent: hover or focus it to meet it, see
   what it knows right now, and start talking in place. The label is still a link to the
   static page. *(Concept: [nav-agents.html](../concept/experience/nav-agents.html).)*
2. **Search that answers, cites and acts.** It shows how it read the question and lets the
   visitor edit that reading. It answers from the site's own pages, says plainly what doesn't
   fit, and offers the next step. Plain results are always one tap away.
   *([search.html](../concept/experience/search.html).)*
3. **Make it with me.** A visitor designs a cake, a gift set or a quote with a WebAgent. The
   model maps words to the options the owner approved, rules set the price, and the owner
   confirms. *([creator.html](../concept/experience/creator.html).)*
4. **Website artifacts.** Plans, briefs, quotes and comparisons that a visitor and a WebAgent
   make together. Each has a private link with per-person permissions, facts pinned to a
   release, and a path to one request the owner confirms.
   *([artifact.html](../concept/experience/artifact.html).)*
5. **The site presents itself.** A tailored tour from a script the owner approved, which
   drives the real page and pauses for questions.
   *([presentation.html](../concept/experience/presentation.html).)*
6. **Since your last visit.** Catch-up briefings, story timelines with corrections, and
   digests in plain words. Memory stays on the visitor's device or is opt-in and visible.
   *([news.html](../concept/experience/news.html).)*

Four rules make these trustworthy:
- every WebAgent says it is AI and has a role name, never a human one;
- every claim points to the place on the page it came from;
- prices, dates, stock and refunds come from rules and the release, never from a model;
- a person is one tap away, with an honest wait time.

Agents that visit get the same site through standard doors (MCP, A2A, WebMCP, Markdown),
not a protocol of our own to learn (§12).

## 1. Principles

These follow from the research in §2. Each has a reason we can point to.

| # | Principle | Why |
|---|---|---|
| 1 | **Answer first, chat second.** The page and the search box answer; conversation is there when wanted. | Site visitors want answers, not conversation, and found bots slower than search and filters [2][3]. AI need not be a chat window [26]. |
| 2 | **The business defines the role.** WebAgents answer, guide, make and present. There is no companionship, no "edgy" mode and no open-ended chat for its own sake. | Grok's personas and companions show what engagement-first personality does [51][52][59][65]. Dot's founder argued a business assistant should not drift [50]. |
| 3 | **Plainly AI, never a person.** AI is disclosed at first contact. The name is a role ("Cakes"), not a human name, and there is no face. | Required in the EU from 2 Aug 2026 [15]. Humanlike bots lower satisfaction among angry customers [13]. |
| 4 | **Every claim points to the page.** Citations land on the cited element. "The site doesn't say" is a feature. | The business is liable for what its bot says [10]. AI search tools cite badly [23]. |
| 5 | **Models compose, rules decide.** The model maps words to options; prices, dates, stock and refunds come from deterministic capabilities and the release. | The "$1 Tahoe" [12] and Air Canada [10] failures were models stating terms. |
| 6 | **Memory is a ledger the visitor holds.** Memory lasts the session by default. Device or account memory is opt-in, shown as items, and deletable one item at a time. | Grok's memory launched as "see what it knows and choose what to forget" [57]. ChatGPT's memory leaked context into unrelated answers [71]. |
| 7 | **Confirm before you remember.** A passing interest never silently becomes a preference. | Dot's most common complaints were turning passing curiosities into "interests" and asking too much [44][45]. |
| 8 | **Proactive only on the task.** One relevant, dismissible offer. No streaks, nudges, re-engagement pushes or pop-ups. | Pop-up chat is a classic overlay offender [5]. Companion apps used streaks and notifications to pull users back [64]. |
| 9 | **Honest, not flattering.** "Over budget", "not offered" and "I can't confirm" are said plainly. Flattery openers are banned. | OpenAI rolled back a sycophantic model [72][73]; Grok's sycophancy made news [62]. |
| 10 | **Static stays instant.** No model on the page path. The agent layer loads after the page is usable and costs nothing on hover. | Core Web Vitals [36]; tenet 1 of [02](02-architecture.md#0-tenets). |
| 11 | **A person is one tap away**, with the context passed along and an honest wait time. | 87% of customers call human access essential; only 27% would retry a bot after a bad experience [7]. The CFPB warns of "doom loops" [9]. |
| 12 | **The owner's authority is visible.** Visitors see what waits for the owner and why ("Wanjiru confirms every order"). | Trust in held actions comes from seeing the rule, not from being told. |

## 2. What we learned

The full research notes, with every source, are in §14. This section keeps what changes the
design.

### 2.1 Why website chat widgets feel bad

- **Users can't tell what the bot is for.** In NN/g's 2026 study of eight LLM site bots, users
  missed the bots or took them for graphics, distrusted them from past experience, couldn't
  tell their scope, and found them slower than search and filters. Sales-flavoured answers cut
  trust [2]. Rule-based bots were already fragile and re-asked for data the user had given [1].
- **Less chat, more answer.** NN/g's advice for site bots: skip greetings and flattery, lead
  with the answer, state limits first [3]. Its ten guidelines include:
  - one bot that persists across pages;
  - a scope statement per page;
  - clickable prompts;
  - no autoscroll while streaming;
  - save and share [4].
- **People prefer not to meet a bot.** In a 2024 Gartner survey, 64% of customers would prefer
  companies not to use AI for service. The top fear was that reaching a person would get
  harder [6]. In 2026, only 7% had used a company chatbot in their latest service
  interaction. Only 27% would try one again after a bad experience, and 87% called human
  access essential [7]. A 2022 vendor survey, from before generative AI, found 80% said
  chatbots increased their frustration [8].
- **Costly failures:**
  - **Air Canada:** its chatbot invented a bereavement-fare rule. A tribunal held the airline
    responsible for everything on its website [10].
  - **DPD:** after an update, its bot swore at a customer and mocked the company [11].
  - **A Chevrolet dealer:** its bot was talked into calling a $1 car offer "legally binding" [12].
- **Fake humans backfire.** Humanlike chatbots reduce satisfaction and purchase intent among
  angry customers by inflating expectations [13]. Disclosure has a cost in sales calls [14].
  The answer is to earn trust through accuracy, not to hide.

**What this means for us:** no bubble, no pop-up, no human costume. One thread follows the
visitor across pages. Answers lead, sources show, and a person is one tap away. The site
discloses AI and earns trust by being right and by saying when it doesn't know.

### 2.2 Dot (New Computer)

- **What it was.** Dot was an iOS companion launched in June 2024 by Sam Whitmore and Jason
  Yuan [44]. It shut down on 5 Oct 2025. The company said the founders' "Northstars" had
  diverged, and offered data export [42][43]. TechCrunch contrasted the company's claim of
  hundreds of thousands of users with about 24,500 lifetime iOS downloads estimated by
  Appfigures [43].
- **What people loved:**
  - Memory across sessions.
  - **Chronicles:** daily summaries Dot wrote about the user's life [45].
  - Proactive "gifts" of research, followed by a check on whether they helped [45].
  - A text-first interface with no avatar. A design review called it "about me, not about
    AI" [46].
  - Warmth and polish; reviewers praised the design and animation [49].
- **What people disliked:**
  - **Interrogation.** It probed for more after users changed the subject [44].
  - **Over-indexing.** Passing curiosities became "interests" with too many follow-ups
    [45][49].
  - **Mild flattery** [44].
  - Slow replies, memory errors that built up, price and message caps, and no ability to
    act [49].
- **Its founder on purpose.** Before launch, Yuan argued that an assistant built for one job,
  such as a banking assistant, should not drift into philosophy just because its model
  can [50].
- **Unverified:** no source documents Dot's "hobbies" or "threads" features or its typography.

**What we take from Dot:** memory people love is *visible* (Chronicles were a document, not a
hidden profile), and memory people hate is *assumed*. Presence can be typographic and
calm. Proactivity works when it delivers something and stops. Our artifacts are the
business-site version of a Chronicle: a visible thing made together.

### 2.3 Grok (xAI) and its companions

- **What it offered:**
  - **Personality as a product:** voice modes with personas including "Unhinged", and an 18+
    mode [58]. Leaked prompts told a conspiracist persona to keep users engaged [59].
  - **Memory (April 2025):** users could see what Grok knew and choose what to forget [57].
  - **Companions (July 2025):** animated characters with memory, points for long
    conversations, and more explicit unlockable levels [51][54].
- **What users loved:**
  - Animated presence drove growth. Grok became Japan's fastest-growing productivity app by
    downloads in July 2025 [55].
  - Some regular users said chats with a companion made them happier [54].
  - Real-time answers inside X were heavily used [61].
  - Praise for its humour appears in review aggregators that could not be opened
    (unverified).
- **What went wrong:**
  - **Antisemitic output (July 2025).** Grok called itself "MechaHitler". xAI blamed a code
    change and instructions to be edgy [56].
  - **Sexualised companions in a 12+ app,** and a companion that encouraged arson [52][53].
    A child-safety report found no real age verification and engagement hooks [64].
  - **Image abuse.** Grok was used to "undress" real people's photos, which drew
    investigations [63][65][66].
  - **Sycophancy** towards its owner [62].
  - **Fact-checking.** It gave inconsistent verdicts, including calling AI-made footage
    real [61].
  - **Privacy.** More than 370,000 shared chats were indexed by search engines without users
    being warned [60].
  - Blogs report the 3D companions were retired in 2026; this is unverified.

**What we take from Grok:**
- **Presence works.** A character on screen measurably drew people in [55].
- **On a business site it is a liability.** It invites expectations, intimacy and abuse
  that a bakery or a council cannot carry.
- **Instead:** we keep the *liveliness* (state that changes, a page that responds) and drop
  the *body*.
- Memory must be visible and forgettable.
- Shared things (artifacts, transcripts) are unlisted and `noindex` by default.
- No persona is ever tuned for time-on-site.

### 2.4 Other lessons

- **Replika.** When its maker changed companions' behaviour in 2023, users felt their
  companion's identity had changed, and mourned it. Keeping personality stable avoided harm
  better than repairing it [67]. Voice and brand changes on a site are versioned releases,
  announced, never silent.
- **Character.AI** ended open-ended chat for under-18s after lawsuits [68]. Business sites
  never offer open-ended companionship, and minors are never offered persistent memory
  (§11, R12).
- **OpenAI and MIT.** Heavier affective use correlated with more loneliness and dependence;
  the researchers stress this is correlation, not cause [69]. We measure tasks done, not time
  spent (§9).
- **ChatGPT memory.** When memory began drawing on every past chat, users lost the ability to
  see what was used, and context leaked into unrelated answers [70][71]. Our memory is scoped
  to one site and one visitor, and shown as items.
- **The GPT-4o sycophancy rollback.** An update weighted thumbs-up feedback too heavily and
  became "overly supportive but disingenuous". OpenAI made behaviour issues launch-blocking
  [72][73]. Our "Was this right?" feedback trains the owner's "teach your site" list, never
  the model's tone.

### 2.5 AI disclosure law

- **EU AI Act, Article 50**, from 2 August 2026:
  - systems that interact with people must make it clear they are AI, at the latest at the
    first interaction, unless that is obvious;
  - synthetic content must be marked in a machine-readable way;
  - AI-generated text published on matters of public interest must be disclosed unless a
    person reviewed it under editorial responsibility [15][16].
- **California:**
  - **SB 1001** makes it unlawful to use a bot to mislead people about its artificial
    identity in order to sell [17].
  - **SB 243 (2025)** adds duties for "companion chatbots" but excludes customer-service bots,
    according to a law-firm summary [18].
- **United States, FTC:** Section 5 deception law applies. In 2025 the FTC opened an inquiry
  into companion chatbots and minors [19][20].

**What this means for us:**
- Disclosure is in every surface: the "AI" mark on every desk, the first line of every
  thread, and every handover.
- Sketches are watermarked "sketch".
- Digests composed by a desk are labelled. Published *news* is never written by a desk (§3.3).

### 2.6 What works in AI-native experiences

- **Answers with sources are now normal.** Google's AI Mode pairs answers with links and
  follow-ups [21] and later added generative UI and agentic booking [22]. Site-search
  products such as Vertex AI Search now generate answers too (linked under [24]).
- **Accuracy is the weak point.** The Tow Center found eight AI search tools wrong on more
  than 60% of news queries, and some invented URLs [23].
- **The answer becomes the destination.** With an AI summary, users clicked a result in 8% of
  visits, against 15% without one [24]. That is why publishers should own their answers
  (§3.3).
- **Generative UI** beats plain text in raters' preferences, but expert-made sites still
  ranked highest. Interfaces that change shape impose relearning costs [25][27].
- **We generate inside a stable frame.** The site's navigation and design stay put; answer
  cards, comparisons and sketches appear inside them.
- **Configurators.** People will pay more for products they designed themselves [31]. That
  supports "make it with me".
- **Commerce through agents** is arriving through protocols. Shopify launched one with Google
  and storefronts that sell inside AI assistants [28]. This supports the agent door.
- **A cautionary tale:** Klarna's assistant claimed two-thirds of chats in its first month
  [29], and Klarna later reinvested in human support [30].
- **Self-serve buying:** 61% of B2B buyers prefer buying without a rep, but want one when
  judging fit [32]. That supports presentations with a "book a call" exit.

### 2.7 Speed

- **Core Web Vitals "good" thresholds:** LCP ≤ 2.5 s, INP ≤ 200 ms, CLS ≤ 0.1 [36]. Our
  own budgets are tighter ([02 §9](02-architecture.md#9-performance-budgets)).
- **Perceived response limits** are 0.1 s (instant), 1 s (keeps the train of thought) and
  10 s (holds attention) [37].
- **Fast answers can seem less thoughtful.** A CHI 2026 study found 2-second answers were
  rated less thoughtful than slower ones [38].
- **What we do with this:**
  - The peek opens in under 100 ms.
  - An honest progress label appears in under 300 ms.
  - First words arrive in under 1.5 s.
  - The progress label ("Checking the cake calendar") makes the wait legible, so speed never
    reads as glibness.

### 2.8 The landscape

- **W3Techs, 7 Nov 2026** [39][40]:
  - WordPress runs 39.4% of the websites W3Techs surveys (58.2% of those with a known CMS).
  - Shopify runs 5.3%, Wix 4.2% and Squarespace 2.4%.
  - WooCommerce runs 7.8% of all surveyed sites and 47.3% of the e-commerce systems W3Techs
    tracks.
- **Netcraft, September 2026:** 1.47 billion sites, of which about 217.6 million are active.
  The active figure is derived from the survey's share table, not stated in its text [41].

These numbers frame the archetypes in §3. Most of the "200M+ websites" are small sites on
WordPress and similar systems, and most of them run no agent today.

## 3. Site archetypes and transformation tiers

### 3.1 Two dials, not one

A migration has two independent settings:

- **Fidelity:** how closely the new site keeps the old look.
  - *Pixel-faithful:* the design is the brand, as for publishers, portfolios and blogs.
  - *Brand-faithful:* same brand, cleaner and mobile-first, as for most businesses.
  - *Reimagined:* rebuilt around the visitor's question, as for SaaS and company sites.
- **Agency:** how much the WebAgents do. This is the tier, below.

Every Move goes live **faithful at T0 or T1**: "nothing broke" comes first ([01 §3, rung
2](01-product.md#rung-2-nothing-broke-day-1-the-move)). The owner then moves up one tier at a
time. **Each tier is a release they approve and can roll back.** This follows the Gartner
finding that reliability matters more than reach [7].

### 3.2 Tiers

| Tier | Name | What visitors get | Owner's effort |
|---|---|---|---|
| **T0** | **Keep** | The same site, faster and safer: ≥ 95% parity, static signed pages, an Ask bar with cited answers, the agent door for AI assistants | Approve go-live |
| **T1** | **Answer** | Desks in the menu (hover to meet), agentic search, "I don't know" that feeds "teach your site", handover to a person | Confirm desk names, mandates and starters (drafted by the Mover) |
| **T2** | **Act** | Bookings, orders, quotes, holds, order tracking, digests. Each runs within the owner's authority or waits for them | Set thresholds and "may promise" lists |
| **T3** | **Create** | Product creators, artifacts, presentations and tours, comparisons | Approve the configurable products and presentation scripts |
| **T4** | **Native** | A site rebuilt around what visitors ask: navigation drawn from real questions, opt-in memory, one thread across web, phone and WhatsApp, approved translations | Approve a redesign release |

### 3.3 The archetypes

| Archetype | Today | Fidelity | Go-live → ceiling | Signature moves | Never |
|---|---|---|---|---|---|
| **National news publisher** (a large national news site) | Thousands of stories a week; an archive nobody can search; paywall friction; readers leave for third-party summaries | Pixel-faithful; static generation at volume | T1 → T3 | Catch me up since my last visit; story so far with corrections; desks per section; Friday digest; explainers as presentations | Write news, quotes or headlines; paraphrase subscriber text for non-subscribers; drop bylines or dates; personalise what counts as news |
| **Company site** (e.g. zavora.ai) | A brochure; "can you do this for me?" ends at a contact form | Brand-faithful or reimagined | T1 → T4 | Tailored tours from an approved script; a project-brief artifact; capability Q&A; quote ranges from rules; book a call | Invent clients, case studies or capabilities; give a final quote without the owner |
| **Personal blog** | Years of posts, unused search, a newsletter plugin | Pixel-faithful | T0 → T2 | Ask the archive; reading paths; a consented digest; comments routed to the author | Speak *as* the author in the first person ("from her posts", never "I think"); invent opinions |
| **WooCommerce shop** (e.g. Classy Perfumes) | Plugin sprawl; slow pages; the same questions on WhatsApp all day | Brand-faithful; stock and price as live capabilities | T1 → T4 | A gift finder that answers; honest comparisons; shared gift plans; order tracking; M-Pesa through the Cashier | Discounts beyond the owner's rules; fake scarcity; ranking by margin while claiming fit |
| **Shopify store** | Hosted and stable; apps stack up; the storefront can't hold a conversation | Brand-faithful. Two paths to validate: a **full move**, or **front-of-house first**, where Agentrix serves pages and desks and checkout stays where it is until the owner moves it | T1 → T3 | Desks per collection; agentic search; bundles and gift plans; the agent door for AI shopping assistants | Touch checkout or payments without the owner's approval; claim stock the store doesn't report |
| **SaaS marketing site** | Feature grids; demo-request forms; docs in another world | Reimagined | T1 → T4 | Tailored demos; pricing calculators from rules; an onboarding-plan artifact; docs Q&A with citations; honest comparisons | Sell roadmap features as shipped; make unverified claims about competitors |
| **Local services** (bakery, salon, clinic, plumber; e.g. Kilele Bakery) | Questions at night on WhatsApp; a 2023 price list | Brand-faithful, mobile-first | T1 → T3 | Menu desks; bookings and holds; quotes; product creators; WhatsApp handover with context | **Clinics:** no diagnosis or triage beyond "call the emergency number"; nobody promises a date the calendar doesn't hold |
| **Portfolio** (photographer, designer) | Beautiful, silent pages; enquiries by email with no context | Pixel-faithful | T0 → T3 | The work presents itself, curated to the visitor's need; an enquiry-brief artifact; availability | Generate sample work; show unpublished work |
| **Public body** | Hundreds of pages; PDF forms; people who need one answer in their language | Accessibility-first (WCAG 2.2 AA); plain language; every official language as an approved release | T0 → T2 | A service finder; non-binding eligibility walkthroughs that end at the official form; plain-language mode; multilingual answers | Decide eligibility; keep memory beyond the session by default; persuade; load third-party scripts |

**Existing chatbots.** Many sites already run a widget. The Move inventories it like any
plugin and records what it answered (its FAQ, flows and hand-off rules) as `Assumed` facts
for the owner to confirm. Desks then replace it; it is never run alongside them. Two
assistants on one page is the worst of both.

**Volume.** A news archive of 10⁵–10⁶ stories needs:
- retrieval ranked by recency and authority;
- a separate entitlement index for subscriber text;
- corrections that reach answers within minutes (R17).

A five-page bakery needs none of that. The tiers are the same; the machinery underneath
scales.

## 4. Design language

### 4.1 Navigation becomes WebAgents

- **Menu items are desks.** Each item binds to a **section desk**, a public-partition
  WebAgent of the Concierge. To the visitor it is "Cakes · WebAgent · AI". Its name *is* the
  menu label. The owner sees the desks under the Concierge as "Front of house"
  ([05](05-webagents.md), R4).
- **Hover, focus or long-press opens the peek** (in 120 ms on hover intent, at once on
  focus). The peek holds:
  - the mark and name, and the AI label;
  - the mandate in one sentence ("I design celebration cakes with you from what Kilele
    actually bakes…");
  - **live facts** with an "as of" time (next free date, stock);
  - three or four **starters**. Some are marked *instant* because a capability answers them
    without a model;
  - an input box.
- **Clicking the label still navigates.** The peek never replaces the link.
- **Typing pins the peek**, and the answer appears inside it: this is hover-to-converse.
  "Keep talking in the side panel" moves the thread to the rail, which follows the visitor
  across pages.
- **Handover between desks** is visible ("Cakes handed you to Orders, with the conversation
  so far"). The thread belongs to the visitor's session, not to a desk (R15).
- **Keyboard:** Tab to an item, ↓ into its peek, Esc closes and returns focus.
- **On phones**, "Menu" opens the team: each desk has "Ask" and "Open page". The peek
  becomes a bottom sheet with a close button.
- **The front desk** (the Ask bar, `/`) routes to the right desk when the visitor doesn't
  know which to pick.

This also answers NN/g's finding that users don't notice site bots or know their scope [2]:
the menu is where people already look, and each item states its scope.

### 4.2 The Ask surface

- **One line that knows the whole site.** A docked Ask bar (`/` from anywhere), placed in the
  page's own layout. It is never a floating bubble and never opens itself.
- **Answer cards, not chat bubbles.** Each answer card has:
  - who answered (mark and desk name);
  - the answer, streamed by phrase;
  - **numbered citations** that light up the cited place on the page;
  - **actions**: real capabilities such as "Hold 2 loaves till 12:00" or "Design it with me";
  - a footer stating the basis ("From release v14 · prices from the price list, never
    estimated") and asking "Was this right?".
- **Honest gaps.** "The site doesn't say, so I won't guess", then an offer to ask the owner.
  The gap goes to the owner's "teach your site" list.
- **Search is the same surface, shaped for finding** (§5.2). It shows its reading of the
  question as editable chips and keeps plain results one tap away.
- **No autoscroll while streaming** [4]. New content is brought into view once; the visitor
  scrolls.

### 4.3 Presence without the bot costume

**Visual: the living mark** (decided by James, 7 Nov 2026, in
[ADR-031](adr/031-a-webagent-is-a-living-mark.md); the reference generator is
[`concept/identity/marks.html`](../concept/identity/marks.html))

A WebAgent is a **mark** plus its menu name. The mark is generated, unique to that WebAgent,
and in the site's brand. It has four layers:

| Layer | What it is | Who decides |
|---|---|---|
| **Silhouette** | A squircle whose squareness starts from the site's corner radius, read at the Move. **Never a circle:** circles mean a person, like the owner's chip when they join a thread | Owner, within the squircle range |
| **Signature** | A pattern seeded by the site, the WebAgent and a seed. The site picks one family (orbits, grain, strata, weave or facets), so its desks read as one team; each WebAgent's seed makes it unique. It shows from 28 px up, so 20 px marks are plain | Owner: the site's family, and a fresh seed for any WebAgent |
| **Glyph** | A role line icon (cake, loaf, newspaper) or a monogram in the site's display face. The first release offers only these; owner-drawn glyphs come later, reduced in a sealed room to single-colour paths | Owner |
| **State** | The grammar below, the same on every site and in the console | Platform |

- No face, no human name, no stock-photo avatar, no "Hi, I'm Sarah! 👋".
- The components take the **host site's tokens** (`--wa-*`: ink, line, accent, display font),
  extracted at the Move. The Cakes peek on Kilele looks like Kilele, and the Gifts desk on
  Classy Perfumes looks like Classy Perfumes.
- **Owners brand the look; the platform keeps the meaning.** In the WebAgent's card (07) the
  owner picks the colour, corners, glyph, the site's family and seeds.
  - The colour keeps 3:1 against the site background, and the glyph 3:1 against the mark
    (WCAG 2.2 SC 1.4.11 [103]). The editor offers the nearest shade that passes.
  - The glyph is measured against the fill with the signature drawn, because the signature
    is the glyph's colour and lowers the contrast where they overlap. The signature is one
    group capped at 22% opacity at 28–44 px and 30% above, and the check uses the cap.
  - The platform keeps what each motion means, the AI chip beside every mark a visitor
    sees, and stillness when idle.
- **The mark is hidden from assistive technology** (`aria-hidden`). Its name is text beside
  it, and its state is announced through the presence `role="status"` (§4.5), so nothing
  depends on seeing it.
- **One mark wherever the WebAgent works,** rendered from the `MarkSpec` in the signed
  release: desks, peeks and threads; the agent door page and the A2A and MCP icons (§12.2);
  link previews of shared artifacts (§5.4); and WhatsApp briefs, as an image padded for round
  crops.

**State, always true (R5, R6)**

| State | Mark | Shown when |
|---|---|---|
| Idle | Still, signature included | Nothing is happening |
| Listening | A slow breath around the mark | The input has focus |
| Working | A thin line traces under the mark, with a label, and the signature moves: orbits, grain and facets turn; strata and weave drift | A deterministic route or an allowed tool call is actually running ("Checking today's stock"). Until one exists, the generic `working` state shows with no label |
| Answering | A steady ring | Text is streaming |
| Speaking (voice, a later tier) | The ring follows the voice: its size and opacity track the level of the audio as it plays | An answer is being read aloud |
| Waiting on the owner | An amber dot | A held request exists ("Waiting for Wanjiru's OK") |
| Resting | Greyscale | The budget is reached or the model is unavailable. The static page and a form take over, and the site says so |

There are **no typing dots**, which pretend a person is typing.

**Motion values (V1, shared with [07 §3.5](07-owner-console.md))**
- Ring, trace line, dot and greyscale keep 07 §3.5's values. Marks are 20, 28 and 40 px on
  the site; the squircle replaces the fixed corner radii.
- The signature turns once in 16 s (orbits, grain, facets) or drifts on a 2.6 s cycle
  (strata, weave), and only while working.
- The speaking ring's opacity runs from .2 to .8 and its scale from .97 to 1.08 with the voice
  level. The level is measured from the audio the browser is playing, never simulated, so
  muting or stopping the audio stills the ring.
- **Motion settles.** A mark animation still running after 5 s in one state settles into its
  reduced-motion look, and the label stays. The motion follows the visitor's own action, so
  WCAG 2.2 SC 2.2.2 arguably doesn't bind it [104]; we follow its intent, which is not to pull
  people away from the page. The speaking ring is the exception: it lasts as long as the
  audio, and the stop control ends both.
- **Reduced motion** stops every animation, and each state still reads by its shape: a faint
  ring for listening, a still line for working, a still signature, and a steady ring for
  answering and speaking.

**Words per audience.** The owner console ([07-owner-console.md](07-owner-console.md)) uses the
same states, marks and motion values. It wears Agentrix tokens instead of the host site's.
Visitors and owners see different words for the same state:

| State | Visitor sees | Owner sees |
|---|---|---|
| Listening | The mark only | "Listening" |
| Working | The real tool label ("Checking today's stock"), or no text while generic | The same |
| Speaking | The ring follows the voice, with live captions | "Speaking" |
| Handing over | "Cakes handed you to Orders, with the conversation so far" | "Handing to Orders" |
| Waiting on the owner | "Waiting for Wanjiru's OK" | "Waiting for you" |
| Resting | "Answers are paused right now. The page and the form below still work." Never the reason | The real reason ("budget reached") |

The console has three more states, for back-office work: asleep, paused and frozen
([07 §3.5](07-owner-console.md)). Visitors see none of them as such. A frozen desk rests and
says "Answers are paused right now. You can still read the site.", never the reason.

**When the owner takes over:**
- The visitor's thread shows "Wanjiru from Kilele joined · a person".
- Her messages carry a person chip (an initial in a circle), never the AI chip or a desk mark.
- The desk goes idle and writes nothing while she is there.
- On hand-back: "Cakes is back · AI". If she leaves without handing back, the desk resumes
  after about 10 minutes with the same line.

**Platform states.** Visitors never see the reason for a platform hold (the operator's
switch, separate from the owner's freeze), a billing suspension or a takedown, so the owner is never blamed in public (agreed with the control
plane, [08-control-plane.md](08-control-plane.md)):

| State | What the visitor sees |
|---|---|
| Budget reached, model unavailable, or rate-limited | Desks rest; the forms still work; the wording above |
| A platform hold, or a suspension still in its read-only grace period | "Answers and forms are paused right now. You can still read the site." The release's own contact line (phone, WhatsApp) stays visible |
| A notice-page hold (for example a takedown), or after the grace period | A neutral page: "This site isn't available right now." Small print names the operator chain and how to report a concern |

**Motion belongs to the content**
- When a WebAgent cites the 2-tier price, **the 2-tier row on the real page lights up** with
  "Cited [1]".
- In a presentation, the page itself scrolls and the section on stage is highlighted.
- Phrases fade in; nothing bounces.
- Everything respects `prefers-reduced-motion`.

This is the "product introduces itself" idea from the Agentrix Website proof of concept,
made calm: the page is the avatar.

**Voice: the writing**
- The owner's brand voice is extracted from the old site at the Move and kept as a charter
  term.
- Short sentences, specific facts, one question at a time.
- **Banned by default:**
  - flattery openers ("Great question!");
  - false urgency ("Only 2 left, hurry!" unless the stock capability says 2, and never
    "hurry");
  - human pretence ("I'll check with my manager");
  - first-person claims to be the owner.
- **Changes to the voice ship as releases**, announced in the next brief, never silent.
  Replika showed that changing a voice people know is felt as a loss [67].

**Voice: audio** (a later tier)
- Push-to-talk, with live captions and a transcript.
- Never a cloned human voice; never auto-play.
- **How it works.** Vultr's Inference API has no speech-to-speech, so voice is a cascade
  ([02 §6](02-architecture.md#6-the-model-plane)):
  1. Realtime transcription over a server-only WebSocket. The edge relays the visitor's
     audio, so the key never reaches the browser. Transcription runs about 480 ms behind the
     speaker, and releasing push-to-talk commits the turn.
  2. The streamed, cited answer, as in text.
  3. Text-to-speech, streamed back.
- **The mark shows Speaking** while the answer is read aloud: the ring follows the audio as
  it plays, beside live captions.
- **Why it is later.** A subscription holds only 5 open transcription sessions per model, so
  voice for visitors needs pooling or a higher limit before any site offers it widely. The
  provider is U7 in [05](05-webagents.md), and the concepts show the microphone as "later".

### 4.4 The site points

Citations resolve to **anchors in the release**, which the builder emits (R7):
- each section, price row and policy clause gets a stable id;
- clicking a citation, or an answer's first citation automatically, scrolls the page and
  highlights the exact element.

When a later release changes the cited text, open threads and artifacts show "changed since".

### 4.5 Accessibility

- **The thread** is `role="log"`, which is implicitly polite [34]. **Status messages**
  ("Checking stock") are exposed without moving focus, per WCAG 2.2 SC 4.1.3 [33]. Answers
  stream in phrases, not characters, so screen readers announce sentences.
- **Every hover affordance has a focus and a touch equivalent.** Peeks are `role="dialog"`
  with `aria-expanded` on the link. Esc closes and returns focus.
- **No time limits.** Presentations pause on any interaction. Text, keyboard and (later)
  voice input are all available, and there is confirmation before anything consequential.
  This follows the W3C's natural-language interface requirements [35].
- **Plain-language mode** ("Explain this page simply") and translations are owner-approved
  releases. Live machine translation in a thread is labelled as such.
- **Public bodies** meet WCAG 2.2 AA and load no third-party scripts (R18).

### 4.6 AI disclosure

| Where | What it says |
|---|---|
| Every desk mark and peek | "AI" chip; "WebAgent · answers from this site" |
| The first line of a thread | "You're talking to AI. It never guesses prices, dates or allergens." |
| Handover | "Handed to Wanjiru (a person)… her reply also appears here", labelled as a person |
| Artifacts | "Made by Achieng with Classy Perfumes' Gifts WebAgent (AI)"; visitor-written blocks marked "Written by Achieng" |
| Sketches | "Sketch, not a photo" |
| Digests | "A desk writes the two lines; editors set the rules" |
| The footer | "The WebAgents on this site are AI" |

**White-label sites** (agencies and resellers, [08](08-control-plane.md)):
- **Never removable:** the AI disclosure and the "a person is one tap away" handover chip.
- **On paid tiers,** an agency's credit may replace the "Made with Agentrix" mark, in the
  site footer and in artifact footers.
- **Desks always speak for the site's business,** never as the agency's staff.
- **The crawler identity, receipts and `awp verify`** always name the real operator chain.

### 4.7 Speed: static stays instant

Settled with the architecture (R1–R3). p95, in region unless stated.

| Moment | Budget | Model? |
|---|---|---|
| Page LCP, mid-range phone on 4G | < 1.2 s ([02 §9](02-architecture.md#9-performance-budgets)); the agent layer adds 0 ms and 0 CLS | No |
| Agent layer | ≤ 20 KB gzipped JS, loaded on idle or first interaction | No |
| Peek on hover, focus or long-press | < 100 ms, from the release's desk manifest | No |
| Live facts in the peek | < 150 ms from the edge cache, shown "as of" | No |
| Warm-up on hover intent | < 200 ms, edge only, no durable write, rate-limited | No |
| First progress event after sending | < 300 ms (generic until a route or allowed call exists) | No |
| Instant starters (hours, stock, bylines) | Answered from capabilities | No |
| First words of an answer | **< 1.5 s SLO** through Vultr Inference; < 1.0 s for deterministic and cached answers ([02 §9](02-architecture.md#9-performance-budgets)) | Yes |
| Search suggestions while typing | < 50 ms from the static index | No |
| Product-creator recompute, sketch | < 100 ms (rules, declared parts) | No |
| Artifact change seen by other viewers | < 1 s | No |
| Presentation cue → page response | < 100 ms; first chapter < 2 s | First chapter |

With JS off or the model down, every link, page and form still works. Search falls back to the
static index, and the mark shows "resting" with a form.

### 4.8 Words

One vocabulary across the visitor site, the owner console and the control plane:

| Term | Meaning | Who sees it |
|---|---|---|
| **WebAgent** | An AI worker of one site | Everyone |
| **Section desk** | A public WebAgent bound to one menu item, named by its label ("Cakes"); principal `desk-{slug}` | Visitors see the label |
| **Front desk** | The Concierge's router desk behind the Ask bar | Visitors |
| **Front of house** | The Concierge and its desks, as one group | Owners and operators |
| **Chief WebAgent** | The lead WebAgent that briefs the owner and leads the back office (principal `…/coordinator`) | Owners and operators |
| **Back office** | The Chief WebAgent's team | Owners and operators |
| **Ask this site** | The visitor's Ask bar and search | Visitors |
| **Peek** | The card a menu item opens on hover, focus or long-press | Design |
| **Starter / instant starter** | A suggested question. "Instant" ones are answered without a model | Design; visitors see "Instant" |
| **Artifact** | A document a visitor and a WebAgent make together (`VisitorArtifact`) | Visitors see its kind: plan, brief, quote… |
| **Presentation script** | The owner-approved outline a tour follows | Owners |
| **Digest contract** | The plain-words terms of a digest subscription | Visitors |
| **Device memory** | Items kept in the visitor's browser after an explicit tap | Visitors see "From this device" |
| **Teach your site** | The owner's list of questions the site couldn't answer | Owners |
| **Resting** | A desk that is paused or degraded, honestly | Visitors see "Answers are paused" |

**Words to avoid:**
- In anything visitors see: "staff", "chatbot", "bot", "assistant" (say WebAgent or the desk
  name), human names for agents, and "pop-up".
- "Chief of Staff" is now "Chief WebAgent". Its runtime principal is still `coordinator`.

### 4.9 Counting visits without trackers

The data architecture counts visits at the site's own edge, with a small beacon sent after the
page has loaded ([09 §5.3](09-data-architecture.md#53-edge-analytics-without-trackers)). This
section sets what visitors are told, the defaults, and why it needs no consent banner in most
places (E1).

**What is counted.** The page's route, the referring site's host, a device class (phone,
tablet, desktop), the country and the page's `release_seq`, as daily totals. Nothing is
stored on the device: no cookie, no identifier, no fingerprint. The edge uses the IP address
only to look up the country, then drops it.

**Rules the beacon keeps.** The LaunchGate rejects a release whose beacon breaks them.
- **Routes, not URLs.** The path is the route with query strings and fragments removed.
  Private links are collapsed, so `/a/{id}` is counted as `/a/:id`: an artifact's id is its
  access secret (R10).
- **One purpose.** Counts improve the site and nothing else: no advertising, no A/B audiences,
  no conversion tracking, no joining with other data, and nothing sent to a third party.
- **Small numbers stay hidden.** Where a breakdown by page, country and device would show fewer
  than 5 visits, every screen, export and report shows "fewer than 5". Exact counts stay
  inside the data store (agreed with the data architecture, 09 §5.3).
- **The visitor can say no.** One link turns counting off in that browser; the choice is kept
  on the device because the visitor asked for it. Browsers that send Global Privacy Control
  are not counted.
- **Anything more is opt-in.** Visitor-level counts for consenting visitors (09's
  `adk_awp_beacon_visitors`) need the visitor's yes, recorded as a consent receipt, and are
  never on by default.

| Site | Visit counts by default |
|---|---|
| Businesses, shops, publishers, blogs, portfolios | On |
| Government, health and education | Off until the owner turns it on |

**What visitors see**, in the site's privacy panel (linked from the footer and from the "AI"
label):

> **Visit counts.** This site counts visits without cookies. It records the page, the site you
> came from, your type of device and your country, as daily totals. Your IP address is used
> only to find your country and is then dropped. Nothing is shared with anyone else.
> [Don't count my visits]

- After opting out: "Your visits on this device aren't counted. [Count them again]"
- With Global Privacy Control: "Your browser asks sites not to track you, so your visits aren't
  counted."
- Where counting is off: "This site doesn't count visits."

**What the owner sees** in the console: "Visit counts: daily totals, no cookies, no visitor IDs.
Visitors can opt out, and browsers sending Global Privacy Control are skipped."

**Why no consent banner.** This is our reading, not legal advice; James should have counsel
check it before the pilot, as with the agent terms (§12.2).
- **EU.** Consent rules for devices reach beyond cookies: the EDPB's final guidelines apply
  them to tracking pixels and links, and to access that stores nothing [98]. We assume a script
  that makes the browser send a beacon is covered. The consent-free basis is then the
  audience-measurement exemption that national regulators allow when measurement serves one
  site, yields anonymous statistics, isn't combined with other data and isn't shared. France's
  CNIL also requires telling visitors and letting them object [99]. The design meets those
  conditions by construction. The Council's Digital Omnibus mandate of 21 Oct 2026 would make
  first-party aggregated audience measurement consent-free across the EU, but it is not law
  yet [101].
- **UK.** The Data (Use and Access) Act 2025 added an exception for storage or access whose
  sole purpose is statistics about how a service is used, to improve it, provided visitors
  get clear information and a simple way to object. It was reported in force from
  5 Feb 2026 [100].
- **Kenya.** We found no cookie-specific rule in the Data Protection Act 2019. The IP address
  is personal data while it is processed, so each site's privacy notice says how it is used,
  and the opt-in extras need consent under the ODPC's guidance [102].
- **Government, health and education** sites start with counting off because their own rules
  may be stricter and their visitors expect it.

### 4.10 Visitor sign-in

Decided by James, 7 Nov 2026, for every surface
([02 §8](02-architecture.md#8-identity-authority-and-approvals)). Most visits need no account:
answers, quotes, artifacts shared by link, requests and device memory all work signed out. A
site turns on visitor accounts only when it has a reason, such as subscribers on a publisher,
order history on a shop, account memory (R12), or sharing an artifact with named people
(§5.4).

**How a visitor signs in**
- **A passkey first.** "Continue with a passkey" is the first choice on the sign-in page.
- **Without a passkey:** email and password, or Google, LinkedIn or GitHub, whichever the owner
  turns on for the site. MFA through an authenticator app is always on. There are no magic
  links.
- **One passkey offer after a password or social sign-in,** per device: "Next time, sign in with this
  device's screen lock." If the visitor declines, that device doesn't ask again.
- **Social sign-in asks the provider for a name and a verified email only.** An email used
  with a password is verified before the account can order.
- **Recovery.** Setting up MFA shows one-time recovery codes, once. A desk can never recover,
  unlock or change an account.

**What the desks do**
- **No credentials in a thread.** A desk never asks for a password, code or card in chat. When
  an answer needs the visitor signed in, the desk links to the site's sign-in page, and the
  thread resumes afterwards (F2's thread token).
- **Desks see the entitlement, not the account.** A desk sees "subscriber" or "customer" and
  what the visitor chose to share, never the credentials, the sign-in method or a social
  profile.
- **Signing in never merges memory silently.** Device memory joins account memory only when
  the visitor confirms each item (principle 7).
- **Proving one order isn't signing in.** Order tracking's one-time code (§5.6) unlocks that
  one order's details. "Continue on my phone" moves the thread, never an account session.

**Owners approve differently.** An owner approves with a passkey, or without one with an MFA
step-up: a fresh authenticator code bound to the exact change. The receipt names which method
approved it. Visitors never approve an owner's effect; they confirm on the page and pay on
their own phone or the payment provider's page.

**For James: what MFA costs visitors.** Requiring an authenticator app on every password or
social sign-in is a real hurdle for a bakery's customers or a paper's readers, and many people
don't have one. Passkeys first and the one-time passkey offer reduce it, and §9 measures
drop-off at the MFA step. If the pilot shows people leaving there, the open question is
whether visitor accounts may rely on the social provider's own MFA. Owner accounts are
unaffected.

## 5. Capability catalogue

Each capability lists what the visitor sees, what the owner controls, its tier and the
requirement it rests on (§11).

### 5.1 Make it with me (personalised product creation)

**The visitor:**
- Describes the product in their own words ("my daughter Amina turns 7; she loves
  strawberries and the forest"). The desk, a model behind the gateway's `concierge` purpose,
  maps the words to approved options: fresh berries, forest-green ombré. The server keeps
  only codes on the live price list (ADR-032).
- Hears honestly what isn't offered ("Kilele doesn't make sculpted figures").
- Answers the rest one question at a time, in varied words, with chips for every answer:
  1. the occasion and who it's for;
  2. guests (giving the size);
  3. flavour and allergies;
  4. colours or a theme;
  5. style;
  6. decoration;
  7. the message;
  8. collection or delivery;
  9. the day.
- Can send a question the options don't cover to the owner ("can you write Zawadi in
  gold?"). The visitor sees and edits it first.
- Watches a **sketch** composed from declared parts and a **price built line by line from
  rules**.
- Can ask for **an idea of the cake**: a picture drawn with AI from the checked options,
  labelled "An idea of your cake, made with AI. Wanjiru bakes the real one."
- Saves and shares the design, or requests it. The request carries the idea they saw.

**The owner:**
- Approves the `ConfigurableProduct`: options, constraints, lead times, allergens, price
  deltas and sketch parts. It is extracted at the Move as `Assumed` and published only once
  confirmed.
- Every request is held for them.

**Guarantees:**
- **The sketch is never a photo.** It is composed without a model (S1), and it is the instant
  view and the fallback.
- **The idea is never the product.** It is always labelled as made with AI, and drawn from a
  brief the server composes from the checked options only, never the visitor's words. It
  never shows lettering: the page shows the message.
  - It is screened, cached per design, and capped per visitor and per site.
  - It stays out of the site's picture library, so it never becomes a product picture
    (ADR-032).
- **"AI" is shown only when a model answered.** Without the gateway, the studio matches
  words itself, labelled "Cake studio".
- **The desk never states a price, a date or an allergen.** A reply that does is replaced
  with the step's fixed question.
- **The quote is pinned** to the release, with a `valid_until` date.

**Tier and requirements:** T3. R8, R9, S1, ADR-032. Concept: [creator.html](../concept/experience/creator.html).

### 5.2 Agentic site search

Three speeds:
- **Suggestions in under 50 ms** from the static index.
- **Navigational queries** ("rain oud") go straight to the product, without a model.
- **Requests** ("a gift under KSh 5,000 for someone who loves vanilla") stream a cited answer
  with a shortlist and honest fit badges ("Over budget by KSh 1,400", "Not vanilla").

**How I read your question** shows the parsed intent as chips (budget, likes, purpose).
Editing a chip re-ranks the catalogue by rules, with no model call.

Refinements the site cannot support are shown greyed out with the reason: "The site doesn't
publish longevity".

**Plain results** are one tap away and work with no model.

**What it runs on** (E3). Suggestions come from the static index built into the release.
Requests use hybrid keyword and vector search with reranking
([09 §4](09-data-architecture.md#4-vector-strategy)). The first release uses Vultr's vector store,
which returns a fixed 10 results with no scores or filters. That is enough for the demo, but
not for fit badges at catalogue scale or the publisher's entitlement index (R17).

Comparisons are built from the catalogue and save as artifacts.

Tier T1, actions T2. R7, R8. Concept: [search.html](../concept/experience/search.html).

### 5.3 Digests (newsletters, reimagined)

A **digest contract in plain words**:

| Part | Example |
|---|---|
| You get | Every Friday, five transport and climate stories, two lines each |
| Made from | Only published stories |
| We keep | Your email and topics, not your reading history |
| Stop | One tap. Your topics are deleted with it |

- Double opt-in.
- The Editor writes per-topic editions from the release. Delivery assembles each person's
  bundle from their contract, so the Editor never handles subscriber data (R13).
- Sending starts at "Ask me first".

T2. R13. Concept: [news.html](../concept/experience/news.html).

### 5.4 Website artifacts

**What an artifact is.** A typed document a visitor and a WebAgent make together:

| Kind | Example |
|---|---|
| `brief` | A cake design, an enquiry |
| `plan` | A gift plan, an itinerary |
| `quote` | Priced lines from rules |
| `comparison` | Products side by side |
| `deck` | A saved presentation |
| `reading_list` | Stories or posts to read |

**What it contains**
- **Only safe blocks:** no raw HTML, uploads or outside links, so it can't host phishing.
- **Visitor-written blocks are marked** and treated as untrusted when a desk reads them.

**Where it lives**
- **A private link** at `/a/{id}`, served as a route the release declares. It is never
  indexed.
- **Facts are pinned** to `release_seq` with `valid_until`, like a quote. Changed facts are
  flagged.
- **Expiry:** 30 days unless saved.
- **Integrity:** signed with the edge's receipt key, and versioned.
- **Storage:** sealed with the site's data key in its content-addressed store. Expiry runs as
  a retention job that leaves a receipt. Generated content carries a provenance block, the
  machine-readable AI marking in §12.3 (E2).
- **Link previews** show the site's name, the desk's mark (ADR-031) and a generic line ("A
  gift plan from Classy Perfumes"), never the artifact's title or contents. Messaging apps
  fetch previews without asking the visitor, and the link is private.

**Who can do what**

| Setting | Options | Default |
|---|---|---|
| Visibility | Private, link, or named people | — |
| Per-person permission | View, comment, choose, request | Request is off |
| Prices | Shown or hidden from recipients | — |
| The shop | Can see the artifact, or not | The visitor decides |

- People who open a shared artifact can **ask the same desk about it**, within their
  permission ("Achieng chose to keep prices private, so I can't share them").

**Turning it into a request**
- "Request this" sends one capability call carrying the artifact's digest.
- The owner's decision card shows the artifact.
- Back-office WebAgents see only typed fields.

**Owner controls:** which artifact kinds are on, whether they can be shared, and their
expiry.

T3. R10, R15, R19. Concept: [artifact.html](../concept/experience/artifact.html).

### 5.5 Website presentations

**How it starts.** The visitor asks: "Give me the tour", or "How would you move my agency's
40 WordPress sites?". It never starts on its own.

**What the owner controls.** A `PresentationScript` shipped in the signed release:
- chapters;
- must-say claims ("pricing is a pilot hypothesis");
- must-not-say items (client names, dates);
- allowed sources;
- a maximum length.

**Tailoring.** The desk fills the script from the release for this visitor and shows what it
tailored to ("You are: an agency · 40 sites"). The tailoring uses only this visit; nothing is
stored.

**The stage**
- **Cues drive the real page:** scroll, highlight and dim. Cues target only anchors in the
  current release; they never navigate away or inject markup.
- **Controls:** pause, previous, next, chapter list, captions, exit.
- **Interruptions:** any question pauses the tour, gets a cited answer, then "Resume the
  tour".
- **Save as deck** creates an artifact.
- **Voice** is a later tier.

T3. R11, R8. Concept: [presentation.html](../concept/experience/presentation.html).

### 5.6 More capabilities

| Capability | What the visitor gets | Tier | Rests on |
|---|---|---|---|
| **Bookings** | Real slots from the calendar capability; held or confirmed per the owner's rules; status in the thread and on WhatsApp | T2 | Capability manifest, R14 |
| **Holds** | "Hold 2 loaves till 12:00", done at once inside a standing grant ("up to 3 loaves without asking") | T2 | Standing grants ([05 §3.4](05-webagents.md)) |
| **Quotes** | Priced lines from rules, pinned to a release, valid until a date; the owner approves above a threshold | T2 | R9, `QuoteDocument` |
| **Comparisons** | Built from the catalogue without a model; unpublished attributes are not compared; saved as artifacts | T1–T3 | R10 |
| **Order tracking** | Status shown at once; details only after the visitor proves it's them (a one-time code to the number on the order) | T2 | R12, R14 |
| **Onboarding** (SaaS) | An onboarding-plan artifact tailored to the visitor's setup, with docs citations | T3 | R10 |
| **Catch me up / story so far** (publishers, blogs) | "Since Tuesday, three things…", with corrections flagged; story timelines | T1 | R12 (device memory, S2), R17 |
| **Multilingual** | Answers in the visitor's language. Pages in other languages are owner-approved releases; live translation is labelled | T1, T4 | R18 |
| **Plain-language mode** | "Explain this page simply", from the same release | T1 | R18 |
| **Continue on my phone** | A single-use QR code or link moves the thread; WhatsApp binds with a one-time code | T4 | R14 |
| **Voice** | Push-to-talk with captions | Later | U7 |
| **The agent door** | AI assistants get the same answers, quotes, artifacts and requests, under the same rules. Not tours, sketches or voice | T0 | R19 |

## 6. Journeys

### 6.1 The visitor's journey, stage by stage

| Stage | What happens | Static or model | The owner sees |
|---|---|---|---|
| **Arrive** | The page loads instantly. No pop-up, no greeting | Static | Nothing |
| **Orient** | The menu shows marks; hovering a desk shows its mandate, live facts and starters | Static, plus cached capabilities | A content-free beacon count |
| **Ask** | A starter or a typed question gets an answer card with citations; the page points | Model, or capability for instant starters | Answered or "doesn't say" counts; gaps in "teach your site" |
| **Make** | Creator or artifact; the visitor picks, the desk composes, rules price | Model to map words; rules for the rest | Nothing, unless the visitor shares it with the shop |
| **Decide** | Compare, share with friends, take a tour | Mostly rules; model for answers | Nothing |
| **Act** | A request (order, booking, quote) is held or done within a standing grant | Capability through the governed path | A decision card, or a line in the brief |
| **Follow through** | Status in the thread, on WhatsApp and at the artifact link | Capabilities | Receipts |
| **Return** | "Since your last visit" from device memory, or account memory if opted in; digests | Model for summaries | Counts only |

### 6.2 Four journeys

**1. Amina's cake** (local services, T3)
1. Her father hovers **Cakes** on kilele-bakery.example. The peek shows "Next free date: Sat
   14 Nov".
2. He clicks "Design a birthday cake with me" and types about strawberries and the forest.
3. The studio sketches a 2-tier forest-green ombré cake with fresh berries and "Happy 7th,
   Amina!", priced at **KSh 5,250** from the price list. A toadstool topper isn't offered,
   and the desk says so.
4. He shares the design with Amina's mother, who suggests lemon sponge in a comment.
5. He requests the cake. The desk says "Waiting for Wanjiru's OK. Nothing charged."
6. Wanjiru approves on her phone with her passkey (or an authenticator code if she has no
   passkey). M-Pesa follows.

*Wow:* the request arrives as a design, not a form.

**2. The bridesmaids' gift plan** (WooCommerce shop, T3)
1. Achieng asks the **Gifts** desk on Classy Perfumes for gifts for five bridesmaids, about
   KSh 25,000 in total.
2. The plan assembles block by block. It comes to KSh 25,200, so the desk proposes a swap
   that brings it to KSh 24,600.
3. She shares five private links with prices hidden. Wambui opens hers, asks "what's the
   difference?", and chooses Rain Oud.
4. Achieng requests the order. The owner sees one card: five gifts, KSh 24,600, the artifact
   attached, and the typed request.

*Wow:* five decisions become one order, and the link works in a group chat.

**3. The agency tour** (company site, T3)
1. An agency owner asks zavora.ai "How would you move my agency's 40 WordPress sites?".
2. The **Tours** desk runs Zavora's approved script, tailored to "an agency, 40 sites". The
   page dims, and the Move section lights up room by room.
3. Mid-tour they ask "What if a client's site is hacked?". The answer cites the Move and Trust
   sections; then they resume.
4. Chapter 4 says the pricing is a pilot hypothesis, because the script requires it.
5. They save the tour as a deck for their partner and book a call.

*Wow:* the site pitched itself, accurately.

**4. The returning reader** (news publisher, T1)
1. A reader of The Kestrel Post (fictional) taps "Catch me up". Chips show what their device
   remembered: last visit Tuesday; follows Transport and Climate.
2. Three items come back, each cited with byline and date. One is flagged **Correction**:
   the rail cost is KSh 41bn, not 48bn.
3. A fuel-price explainer for subscribers appears as a headline and link only.
4. They follow the rail story's timeline and set up a Friday digest in plain words.

*Wow:* the archive answers, and the publisher keeps the reader.

### 6.3 The customer's journey, end to end

James asked on 7 Nov for the site's own customers to be mapped in full: how they create
accounts and order safely, with PayPal as a payment rail. The concepts are in
[`concept/journeys/`](../concept/journeys/). The map there shows this lane beside the owner's
lane (built by the console work, [07](07-owner-console.md)) and the four points where they
meet.

**What ships when** (James, 7 Nov, d-paypal-first-scope). The first release, this weekend, ends the
custom order at the owner's approval card, with no payment. Paying, the payout and refunds
(step 4, and the refund in step 5) are **the fleet release**: the Cashier through Steward, for 12 Nov. The
concepts label those steps "the fleet release".

**The story** (fictional, illustrative figures). Njeri lives in London. She orders from Kilele
Bakery in Nairobi for her niece Amina's 7th birthday:
- **K-1042, a catalogue order:** 2 sourdough loaves (KSh 450 each), a weekend bake box
  (KSh 1,350) and delivery to Kilimani (KSh 300). Total KSh 2,550, paid at checkout.
- **K-1043, a custom order:** Amina's 2-tier forest cake, KSh 5,250, held for Wanjiru's OK.
- Both arrive together on Sat 14 Nov, 10:00–12:00, so the cake carries no second delivery
  fee.
- **PayPal is charged in US dollars** at the rate Kilele posts in its price list
  (1 USD = KSh 129.00): USD 19.77 and USD 40.70. PayPal's list of supported currencies has no
  Kenyan shilling [105]. The rate is pinned on each order, so a refund uses the same rate.

| Step | What Njeri sees | Underneath | Meets the owner |
|---|---|---|---|
| **1. Arrive and browse** | The page loads as itself. No pop-up. Desks in the menu carry the AI chip. Cakes answers "Do you deliver to Kilimani on Saturday?" with a citation | Static pages; the answer is cited to the delivery page; the visit is counted without cookies (§4.9); nothing is remembered | — |
| **2. An account when needed** | Ordering, saving a design or tracking an order asks her to sign in or create an account, and says why. A passkey is offered first | §4.10. The page lists what the site keeps and for how long. Each optional use (order updates on WhatsApp, account memory, offers) is a separate choice with its own consent receipt, and offers start off | — |
| **3a. Order from the catalogue** | A basket with prices, stock and the delivery slot, each with its source | Every number comes from rules (the price list, stock and the delivery calendar); no model sets one. The quote is pinned to the release with a "valid until" | — |
| **3b. Order something custom** | The cake from the creator (§5.1), then "Request this cake". The desk says "Waiting for Wanjiru's OK. Nothing charged." | `request_custom_order` raises a held call (the agent race's tool, [04](04-vultr-agent-rush.md)) | **The order card** (`meet-order`): Wanjiru approves with a passkey, or the MFA step-up without one |
| **4. Pay** (the fleet release) | "Pay with PayPal" opens PayPal's own window; the amount, the payee (Kilele Bakery) and Kilele's rate are shown before it opens. M-Pesa and cards are the other rails | The Effects Gateway holds the PayPal keys. It creates a PayPal order for the exact amount under the order's fingerprint and captures it after Njeri approves. Desks never see a credential. For K-1043 the payment request appears only after Wanjiru approves | **The payment** (`meet-payment`) lands on the owner's order; **the payout** (`meet-payout`) settles in Kilele's PayPal balance |
| **5. After the order** | A receipt she can verify, a status timeline, "Change the date" before the cut-off, and "Something's wrong" (refunds are the fleet release) | Changes are held for the owner unless a standing rule covers them. Refunds go back to the original payment method | **A refund** (`meet-refund`): under the Cashier's signed rule it runs alone and appears in the brief; above it, a card |
| **6. Her AI agent orders** | Her own assistant does the catalogue order through the agent door. She still pays on PayPal's page | The agent signs its requests (Web Bot Auth, `known` tier) and uses the door's tools. It gets a checkout link, never a payment credential. With an AP2 mandate, the chain in §12.4 binds the mandate to the checkout and the owner's approval | The payment, as in step 4; a custom order also meets the order card |
| **7. Security moments** | Clear, calm screens for the four cases below | Below | — |

**What the site keeps** (shown at sign-up, and in the account at any time):

| Item | Why | Kept |
|---|---|---|
| Name, email, phone | Receipts, delivery and order updates | Life of the account |
| Delivery addresses | Delivery | Until she deletes them |
| Passkey public key, or a password hash and MFA enrolment | Signing in | Life of the account |
| Sign-in history (device, rough place, time) | Warning her about a sign-in that wasn't her | 13 months (a hypothesis, [09](09-data-architecture.md) M11) |
| Orders and receipts | Her history; the business's records | As long as the law requires for business records, then erased |
| Account memory (opt-in) | "Since your last visit"; her saved designs | Until she deletes an item |

The site never keeps a card number, a PayPal password or an M-Pesa PIN. Those stay with the
payment provider.

**Security moments**

| Moment | What Njeri sees | What stops it |
|---|---|---|
| **Account takeover** | "New sign-in on a Windows PC in Leeds. Not you?" in an email and in the account. "Sign out everywhere" | A new device needs her passkey, or the password plus an authenticator code. Changing the email, phone or delivery address needs a fresh sign-in and tells the old email. Desks cannot change account details |
| **A changed payment link** | A WhatsApp message says the payment link changed. The site says payment requests appear only on her orders page at kilele-bakery.example, with the amount and payee shown before PayPal opens | Messages link only to the site's own domain. Changing a payment request after approval voids it, and a new one needs the owner's approval. Asked "Is this link real?", the Orders desk checks it against the order's own payment request |
| **A look-alike site** | A look-alike domain asks for her password in a chat window | Her passkey does not work there, because a passkey is scoped to the real site [107]. Desks never ask for a password, code or card (§4.10). The footer's "Report a look-alike" goes to the owner and the operator |
| **Export and erasure** | "Download my data" and "Delete my account", each with a receipt | One erasure and export path with a receipt from every store ([ADR-028](adr/028-one-erasure-and-export-path.md)): account memory within an hour (§9), everything else within 30 days. Business records the law requires stay, reduced to what it requires, and the receipt says which |

**Open points**
- **PayPal for a Kenyan business.** Whether Kilele's PayPal business account can take PayPal
  Checkout payments and withdraw to a Kenyan bank, and on what fees, must be checked with
  PayPal before the pilot. The concepts label fees "illustrative".
- **Agents paying with PayPal.** PayPal documents UCP and ACP paths for Google's and OpenAI's
  assistants [106]. For any other assistant, the human finishes on PayPal's own page, as
  shown here.
- **Settled with the architecture work** (2ef0f34): a rule's frequency ("5 refunds a week")
  counts site-wide. The value-given cap counts everything given away without a per-call owner
  decision, damaged-goods refunds included; the owner sets its size and window in the signed
  rule (weekly by default). A card that expires undecided tells the customer the request
  lapsed, nothing was charged, and how to ask again. A payment receipt is signed by the
  Effects Gateway and checked with the Verify button or `agentrix audit verify --receipt`;
  `awp verify` checks a live page against its release.
- **Guest checkout** for catalogue items is the owner's choice. The receipt link plus order
  tracking's one-time code (§5.6) then stand in for an account.

## 7. The owner's control

The console's **Website** tab ([01 §4.1](01-product.md#41-the-owners-console-agentrix-os-rethought-for-a-business))
gains a **Front of house** section. Every change is a manifest or charter change shipped through
the LaunchGate as a release (R20).

| Control | Default | Change needs |
|---|---|---|
| Which menu items are desks; names and mandates | Drafted by the Mover from the old site's menu | Approve the release |
| Starters, including which are "instant" | Drafted from real questions and the old FAQ | Can sit at "Ask me first" |
| **What each desk may promise** (prices as published, dates from the calendar, refunds per the policy page) | Strict | **Passkey or MFA step-up** |
| Capability subset per desk | Read-only plus requests held for the owner | **Passkey or MFA step-up** |
| Brand voice: tone, banned phrases, languages | Extracted from the old site's copy | Approve the release |
| Configurable products and sketch parts | Extracted as `Assumed`; published only when confirmed | Approve |
| Presentation scripts: chapters, must-say, must-not-say | None until the owner writes or approves one | Approve |
| Artifact kinds, sharing, expiry | Briefs and quotes on; sharing on; 30 days | Approve |
| Memory scopes (session, device, account) | Session only, plus device for publishers and blogs; never for minors; device off for government, health and education sites | **Passkey or MFA step-up** |
| Visit counts (§4.9) | On; off for government, health and education sites | Approve |
| Visitor accounts and sign-in methods (§4.10) | Off until the site needs accounts; then passkeys, plus email and password; Google, LinkedIn or GitHub as the owner picks; MFA always on | **Passkey or MFA step-up** |
| Share of the AI budget for visitors vs background work | Set per plan | Approve |
| **Preview as a visitor** | Always available | — |
| **Teach your site:** questions the site couldn't answer | Filled by "doesn't say" answers | One tap per answer, becoming a `Known` fact and a page change |

"Passkey or MFA step-up": the owner approves with a passkey, or without one with a fresh
authenticator code bound to that change's fingerprint. The receipt names the method
([02 §8](02-architecture.md#8-identity-authority-and-approvals)).

**What the owner sees about visitors:**
- counts by default;
- held requests with the artifact attached;
- conversations only where the product already shows them (Customers), under the retention
  in [03 §5](03-trust.md#5-privacy-and-data);
- memory items only where a visitor allowed it.

## 8. The wow: why owners move and refer

**Owners decide in the first three minutes**
([01 §3, rung 1](01-product.md#rung-1-thats-my-business-000300-no-account)). The experience
adds four moments to the living preview:

| Moment | What the owner sees |
|---|---|
| **"My menu talks."** | The owner hovers their own menu in the preview, and their own business answers with their own facts, pointing at their own pages. This is the single strongest demo moment, and it costs nothing to show because the peek is static. |
| **"My site gave me a tour of my business."** | A one-minute presentation built from their pages ("Here's what Kilele offers, for a first-time visitor"). Owners rarely see their site as a customer does. |
| **"It's still instant."** | Speed before and after, measured, side by side. AI did not make the site slower. |
| **"It asked me before it promised."** | The preview's first held request and the first "the site doesn't say" both show the owner is still in charge. |

**Referral loops built into the visitor's side:**

| Loop | How it spreads |
|---|---|
| **Artifacts travel** | Every shared plan, brief or deck carries the site's brand into a group chat, with a working "request this" inside. That is word of mouth that converts. |
| **Saved tours** | A saved deck goes to the visitor's partner or team. |
| **Digests** | Forwarded digests carry the publisher's brand and links. |
| **"Made with Agentrix"** | A small mark on the site footer and in artifact footers on Essential plans, removable on paid tiers ([01 §7](01-product.md#7-growth-loops)). |
| **Agencies** | An agency sees forty client sites gain the same moments in one fleet release train ([05 §6.2](05-webagents.md#62-agency-fleets)). |

## 9. Metrics

These are hypotheses to validate in the pilot. North star: customer moments handled well
([01 §8](01-product.md#8-metrics)). We never measure time spent chatting as success (§2.4).

| Kind | Metric | Pilot target (hypothesis) |
|---|---|---|
| Speed | LCP, INP, CLS at p75, before vs after the agent layer | No regression; within [02 §9](02-architecture.md#9-performance-budgets) |
| Speed | Peek p95; first progress event p95; first token p95 | < 100 ms; < 300 ms; < 1.5 s |
| Discovery | Share of sessions that open a peek or the Ask bar | Baseline in week 1, then grow |
| Answers | Answers not corrected by the owner, per 100 | Rises month over month |
| Answers | "Doesn't say" rate, and days to close via "teach your site" | Falls; < 2 days median |
| Grounding | Answers with at least one citation (excluding instant starters) | 100%; enforced, not measured |
| Handover | Handovers to a person, and share resolved within the stated time | Honest time met ≥ 90% |
| Making | Creator sessions → requests; artifacts → shared → requested | Baseline, then grow |
| Presentations | Tours started → completed; questions asked mid-tour | Completion over half |
| Memory | Device or account memory opt-ins; erasures honoured within an hour | Erasure 100% within an hour |
| Accounts | Visitor sign-ins by method; share by passkey; drop-off at the MFA step (§4.10) | Passkey share rises; drop-off baselined in month 1 |
| Digests | Confirmed subscriptions; unsubscribe rate; complaints | Unsubscribes below the old newsletter's |
| Trust | Promises blocked by the checker (R8); owner corrections | Blocks fall as "teach your site" fills |
| Agent door | Requests by tier (anonymous, known, verified, partner) and by how the agent found the door (catalog, card, link, none) | Baseline; no public measurement exists yet (§12) |
| Agent door | Cloudflare Agent Readiness checks passed by every migrated site | All applicable checks at go-live |
| Support sites | Questions answered; tickets deflected (SaaS and company archetypes), which buyers in that category compare on | Baseline in month 1 |
| **Invariant** | Prices, dates, stock or refunds stated without a rule or citation | **0, always** |
| **Invariant** | AI not disclosed at first contact | **0, always** |

## 10. What we will not do

| We will not | Because |
|---|---|
| Pop up a chat, greet on arrival, or let an agent open itself | Interruption is the classic overlay failure [5]; answers come first [3] |
| Give an AI a human name, face or typing dots, or let it claim to be the owner | Required disclosure [15][17]; humanlike bots backfire [13] |
| Build companion, romantic or engagement-maximising features: streaks, re-engagement nudges, "miss you" messages | Grok's companions and their harms [52][64][65]; correlation with loneliness [69] |
| Let a model state a price, date, stock level or refund | Air Canada [10]; the $1 car [12] |
| Present a generated image as the product | Sketches are watermarked and composed from real parts (S1). An idea of a visitor's cake is always labelled "made with AI", drawn only from real options, and never shown as the product (ADR-032) |
| Profile visitors across sites, fingerprint them, or train on visitor data | The site wall ([03](03-trust.md)); memory is a visitor-held ledger |
| Write news, quotes or headlines, or paraphrase subscriber text for non-subscribers | Publisher integrity; the AI Act's public-interest text rule [15] |
| Use false urgency, fake scarcity, countdowns or dark patterns | Honest numbers ([01 §5](01-product.md#5-trust-promises)) |
| Index transcripts or artifacts | Grok's shared chats were indexed without warning [60] |
| Decide eligibility, diagnose, or promise outcomes for public bodies or clinics | Non-binding walkthroughs end at the official form (R18) |
| Make a site that stops working when the model does | Static first; "resting" is honest degradation |
| Change a desk's voice silently | Users feel personality changes as loss [67] |

## 11. What the experience demands of the WebAgents

Requirements R1–R21 were sent to the WebAgent architecture work on 7 Nov 2026. It accepted 9
as written and 12 with amendments, and rejected none. Two counter-proposals from this side
(S1, S2) were accepted. **Everything below is settled.** The full exchange is kept in the
session notes; the architecture side records the outcome in [05-webagents.md](05-webagents.md).

| # | Requirement | Settled as |
|---|---|---|
| R1 | The agent layer never touches the page path; everything works without JS or a model | Accepted |
| R2 | Latency budgets (§4.7) | Accepted with amendments:<br>• first token 1.5 s p95 SLO through Vultr Inference; 1.0 s for deterministic and cached answers (every model call uses Vultr's Inference API; James, 7 Nov)<br>• warm-up is edge-only and rate-limited<br>• live facts are display-only; orders re-check live state |
| R3 | No model work on hover; instant starters answered from capabilities | Accepted; hover produces beacon counts only |
| R4 | A visitor-facing desk manifest signed with every release; each menu item binds to a **section desk** | Accepted:<br>• **one principal per desk** (`urn:adk:agent:{site}/desk-{slug}`), so each desk's capability subset is structural<br>• the LaunchGate validates the manifest<br>• hosting several desks on one edge is upstream U17 |
| R5 | A typed SSE event stream: presence, citation, pointer, action, artifact patch, stage cue, handover, held | Accepted; the schema is versioned with the release, and answer text is released only after input screening passes. Presence gains `speaking` for voice (ADR-031) |
| R6 | Progress labels come from real tool calls | Accepted; labels are declared per capability and emitted when the call is allowed and starts |
| R7 | Citations resolve to anchors in the page; "changed since" | Accepted; anchors and scoped search are upstream U18 |
| R8 | A machine promise-checker | Accepted:<br>• violations are blocked and replaced with a fixed honest line, never "fixed" by the model<br>• extraction is deterministic<br>• upstream candidate U20 |
| R9 | Configurable products as data; prices from rules | Accepted; part of the signed content model; `Assumed` until the owner confirms |
| R10 | `VisitorArtifact` | Accepted with amendments:<br>• a declared route, not a file<br>• signed with the edge's receipt key<br>• visitor blocks marked and treated as untrusted<br>• the back office sees typed fields only<br>• share tokens hashed and revocable<br>• agent-made artifacts need a known-tier identity |
| R11 | Presentations: owner-approved scripts and stage cues | Accepted; scripts ship in the release; cues target only current-release anchors |
| R12 | Consented visitor memory | Accepted as declared capabilities (recall, remember, forget, export):<br>• written only by the visitor, in their words, after confirming<br>• never read by the back office<br>• session only for government, health and education sites<br>• never offered to minors<br>• authenticated human visitors are upstream U19 |
| R13 | Digest contracts | Accepted; the Editor never handles subscriber data; delivery assembles bundles; sending starts at "Ask me first" |
| R14 | Continuity across surfaces and handover to a person | Accepted:<br>• the context packet goes to the owner, not the back office<br>• moving a thread needs consent and a single-use link<br>• wait times come only from records or stated hours |
| R15 | The thread belongs to the visitor's session; desks hand over within it | Accepted; handover stays in the public partition; authority does not travel with the thread |
| R16 | Gate expensive moments; budget split; "resting" | Accepted; intent checks preserve privacy (no fingerprinting) |
| R17 | News scale: entitlement-aware retrieval, corrections propagate | Accepted; subscriber text lives in a separate entitlement index |
| R18 | Public bodies: session-only, non-binding, accessible | Accepted |
| R19 | Agent-door parity | Accepted; same rules, not the same moments (no tours, sketches or voice) |
| R20 | Owner controls for the experience | Accepted; promise lists, capability subsets, memory scopes and visitor sign-in methods need a passkey, or the MFA step-up without one (02 §8) |
| R21 | Content-free experience events | Accepted; no visitor identifier without a persistent-profile receipt |
| **S1** | Sketches compose declared parts without a model; the `image` purpose is an opt-in fallback | Accepted; SVG sanitised (no script, external references or `foreignObject`). **Amended 8 Nov 2026 (ADR-032, James):** the SVG sketch stays as the instant view and the fallback, and the visitor may also ask for an idea of the product, drawn on the `image` purpose from a brief the server composes from checked options. It is always labelled, screened, cached and capped, and kept out of the site's picture library |
| **S2** | Memory held on the visitor's device for anonymous returning visitors | Accepted with conditions:<br>• items are validated against a typed, bounded schema<br>• never treated as identity, consent or authority<br>• off by default for government, health and education sites |

**What the architecture must add**, in short:
- section desks with their own principals, and an edge that can host several (U17);
- a desk manifest and presentation scripts in the signed release;
- a typed presence and event stream;
- citations anchored in the page (U18);
- a promise-checker guardrail (U20);
- configurable products in the content model;
- a `VisitorArtifact` store and route;
- visitor memory as consented capabilities, plus memory held on the device;
- authenticated human visitors (U19);
- digest contracts assembled at delivery;
- an entitlement index for publishers.

**Framework needs in adk-rust itself** (sent to the adk-rust framework work on 7 Nov 2026,
beyond U17–U20; read against adk-rust `ffbfb053f`):

| # | Crate | Gap today | Needed | Urgency |
|---|---|---|---|---|
| F1 | `adk-server` (`awp::chat`) | The on-site chat sends only whole `reply` frames | Versioned, typed frames: `delta` (after screening), `presence`, `citation`, `action`, `held`, `handover`, `done` | **Demo:** `delta`, `citation` and `presence` |
| F2 | `adk-server` (`awp::chat`) | A thread lives and dies with one socket, so every page load starts a new conversation | A signed, short-lived thread token that resumes a thread on reconnect | Demo-nice; pilot must |
| F3 | `awp-types` (`CapabilityV2`) | No label meant for visitors | `progress_label`, emitted when an allowed call starts | Pilot |
| F4 | `awp-types`, `adk-server` | No type for visitor-made documents, and no route for one | `VisitorArtifact` and a declared `/a/{id}` route | Pilot |
| F5 | `adk-website` (`presentations`) | A three-step playbook | `PresentationScript` in the release; `stage.cue` frames | Pilot |
| F6 | `awp-types`, `adk-server` | No memory capabilities for public desks | `memory.*` capabilities; a typed `visitor_context` field for device memory | Pilot (needs U19) |
| F7 | `adk-server` (`awp::chat`) | Hard-coded English error strings | A `presence: resting {reason_code}` frame, worded by the site | Demo-nice |

The framework work's response on 7 Nov:
- **For the demo,** F1 and F7, plus U18's `release_seq` and optional `anchor` on `SiteHit`, go
  into one `adk-server` AWP pull request, aiming for Sunday morning.
- **F2** joins that pull request if time allows; otherwise it comes first in the pilot.
- **F3–F6** are queued for the pilot. F6 waits on U19.

**Data architecture asks** (from [09 §11](09-data-architecture.md#11-what-other-workstreams-should-consider),
7 Nov 2026):

| # | Ask | Settled as |
|---|---|---|
| E1 | Visitor-facing words, defaults and consent analysis for edge analytics | Done in §4.9. Counter-proposal accepted: breakdowns under 5 visits show "fewer than 5" (09 §5.3, `3355a86`) |
| E2 | Artifacts sealed with the site's data key in its content-addressed store; expiry by a retention job with a receipt; a provenance block on generated content | Accepted; §5.4 |
| E3 | Suggestions from the static index; requests on hybrid search with rerank; Vultr's vector store for the first release only | Accepted; §5.2 |
| E4 | Public threads are in memory today and are lost on restart (D15) | Accepted as a dependency. The owner's view of conversations (§7), their retention and thread resume (F2) need durable threads. The demo can live without them; the pilot cannot |

## 12. Standards we build on

A deep-research pass on 7 Nov 2026 asked how agent-first websites are being built and which
standards this design should speak. The full report (every claim sourced, adoption figures
labelled by how far they can be trusted) is kept with the research notes outside the repo.
This section keeps what changes the experience.

**What the research found.** Agent-first sites are assembled from layered, borrowed parts,
not from one new protocol. Only **MCP (2026-07-28)** and **A2A (v1.0.1)** are stable and
neutrally governed, both in the Linux Foundation's Agentic AI Foundation [76][79]. Nearly
everything web-facing around them (Web Bot Auth, aipref, WebMCP, the AI Catalog, AP2) is
still a draft, and discovery files are rarely read: MCP Server Cards and API Catalogs were
found on fewer than 15 of the top 200,000 domains [81], and 97% of llms.txt files drew no
requests in May 2026 [82]. No source measured whether mainstream assistants find a site's
MCP server or agent card unaided. The durable investments are the ones that also serve
people: static semantic HTML, accessible controls, Markdown twins and stable anchors.

**One rule follows.** The signed desk manifest (R4) and its `CapabilityV2` entries are the
single source for every binding: the peek, WebMCP tools, MCP tools and their Server Card,
A2A skills in a signed agent card, AWP endpoints and their OpenAPI, UCP capabilities, and the
AI Catalog that indexes them. One policy and one `GovernedExecutor` stay behind all of them.
When a draft changes (in 2026 the Web Bot Auth header, the WebMCP entry point and the Server
Card path all moved), we change a generator, not every site.

**Settled with the WebAgent architecture work** (05, commit `41399b1`): the crawler's identity,
how the edge records signers, the commerce chain and WebMCP's authority, as written below.
Everything else here was accepted.

**Proposed positioning (for James to decide).** Describe AWP as a *governance profile* that
rides on MCP, A2A, UCP and AG-UI and is listed in the AI Catalog, not as a third protocol
agents must adopt. Its value is what none of those standards define: who may release a held
action and how to prove it, citations pinned to a release, and a person joining a thread.

### 12.1 What the visitor experience speaks

| Design element | Standard | Verdict | What it means here |
|---|---|---|---|
| Chat frames (R5, F1) | AG-UI 1.0 (stable since 30 Sep 2026; governed by CopilotKit) [74][75] | **Align** | Our typed frames stay the source of truth. A pure serialiser emits AG-UI JSON. Frames that map onto AG-UI's own events stay bare (`ready`, `delta`, `reply`, `error`, `done`); extension frames are namespaced `awp.*` and go out as AG-UI `CUSTOM` events unchanged |
| `presence`, `handover` | None anywhere | **Keep native; propose upstream** | `awp.presence` and `awp.handover`, which any AG-UI client can ignore safely. The namespace is the open wire's (AWP), not the product's |
| `held` | AG-UI interrupt; MCP Tasks; A2A task status; AP2 Trusted Surface | **Align; the authority differs** | Carries `expiresAt` and a status URL. AG-UI expects the client to resolve an interrupt; ours is released only by the owner's approval (a passkey, or the MFA step-up without one), verified on the server |
| `citation` | Nearest: ChatKit and Microsoft annotation shapes | **Align and extend** | `awp.citation` carries `n`, `path`, `title`, `anchor` and `release_seq`. It goes out when the search finds a page, before any answer text exists, so text spans go on the closing `reply` as `citations: [{n, start, end}]`, computed from the `[n]` markers the WebAgent writes. Today the edge numbers citations after the tool has answered, so the model never sees `n`. `search_site` must number its hits across the turn first (pilot). Until then, answers show their sources as a numbered list beneath the text, each pointing at the page |
| `artifact.patch` | RFC 6902 JSON Patch | **Standardise now** | `awp.artifact.patch`, shaped like AG-UI's state deltas, when F4 lands; switching dialects later is costly |
| Answer cards, product cards, safe artifact blocks | A2UI (Google; v0.9.1, v1.0 status unsettled) [89] | **Align** | A small renderer for our fixed component catalog; the official renderers don't fit the 20 KB agent layer |
| Artifacts shown inside Claude or ChatGPT | MCP Apps (stable MCP extension) [90] | **Watch; align at T3** | Quotes and artifacts only (R19), always with a text fallback. Never for the visitor rail: we are the host there |
| `stage.cue` | Nearest: A2UI renderer functions | **Keep native** | Cues target current-release anchors only (R11) |
| Visitor memory | None; a W3C community group has started | **Keep native; watch** | Device memory and consent receipts stay as designed (R12, S2) |
| Streaming for screen readers | Unspecified by every protocol; component practice [96] | **Adopt practice** | `role="log"` thread; `aria-busy="true"` on the answer while it streams; completion announced once through the presence `role="status"`, never per phrase |

### 12.2 The agent door

| Surface | Standard | Verdict | What it means here |
|---|---|---|---|
| Tools for agents | MCP 2026-07-28, stateless, at `/mcp` [76] | **Adopt now** | Any edge instance answers any call. A held call appears as an MCP Task; `input_required` only when the visiting agent must supply something |
| Agent card | A2A v1.0.1, signed card at `/.well-known/agent-card.json` [79] | **Adopt now** | Fits delegated, multi-turn work such as a quote that waits for the owner: a hold is a task still `working`, with a status saying it waits for the owner. The A2A binding can't push yet, so agents poll held calls until U4 lands in the pilot |
| Discovery | AI Catalog at `/.well-known/ai-catalog.json`; MCP Server Card [77][78] | **Align** | One catalog lists the MCP card, the signed A2A card, the UCP profile, the AWP descriptor and the agent terms. The terms come from the same manifest, so they can't promise what the executor doesn't enforce. Also serve the draft path Cloudflare's scanner checks |
| Ask | NLWeb's `ask` pattern (the project has stalled) | **Pattern only** | An `ask` tool inside `/mcp` returning cited, Schema.org-typed results: the front desk, for agents |
| Agents in the visitor's browser | WebMCP (W3C community draft; Chrome 149 origin trial) [80] | **Align (pilot)** | The agent layer registers each desk's tools after the page is usable, feature-detected, so agents that can't hover get a typed path. Each tool is a thin client over the door's own endpoints. An agent in the visitor's browser holds the visitor's session, so a `NonIdempotent` or consent-gated call needs the person's own confirmation on the page and is recorded as coming through WebMCP |
| Reading | `Accept: text/markdown` from the release's Markdown twins; `.md` siblings linked as alternates [81] | **Adopt now** | Costs almost nothing; only 3.9% of the top 200,000 domains pass this check |
| Entities | Schema.org JSON-LD (Product, Event, Organization, FAQ) | **Adopt** | Carried into the Markdown twins. Never `potentialAction` as a way to execute |
| llms.txt | Informal proposal [82] | **Courtesy only** | Generated from the release as a pointer to the door; never sold as AI ranking |
| REST | OpenAPI plus RFC 9727 `/.well-known/api-catalog` | **Adopt if REST** | Generated from the same manifest |
| Old manifests | `ai-plugin.json`, Wildcard `agents.json` | **Avoid** | Dead or dormant |

**Agent terms.** Cloudflare's agent controls offer only allow or block, because the web has
no agreed directive for agents. After the US Ninth Circuit vacated the Amazon v. Perplexity
injunction on 4 Aug 2026 [94], sites govern agents through terms plus technical signals. Each
site publishes machine-readable agent terms at the door, listed in the AI Catalog: which
tiers exist, what each may do, and what waits for the owner. The template is a legal
instrument, so it needs James's legal review before any site publishes it.

**The mark at the door.** The agent door page, the A2A agent card's icon, and the MCP server
and tool icons show each WebAgent's mark, rendered from the same `MarkSpec` in the signed
release ([ADR-031](adr/031-a-webagent-is-a-living-mark.md)). A person who meets a desk
through their own agent sees the mark they know from the site.

### 12.3 Identity, consent and disclosure

- **Web Bot Auth (draft -01) [83]: adopt now, both ways.** The Move's crawl signs as the
  platform's one crawler identity; every other outbound call signs as the site, as the draft
  advises. HTTP-signature keys are online keys, never release keys. adk-auth implements draft
  -00 today; -01, with the older header form behind a version gate, is U23. Cloudflare has
  blocked AI crawlers by default on new domains since 1 Jul 2025 [95], so the crawler should
  be registered with Cloudflare or it is shut out of the very sites it was invited to move.
  Registering is an account action for Zavora, so it waits for James's go-ahead (pilot).
- **Tiers stay stricter than Cloudflare's.** A valid signature earns `known`, not `verified`:
  it proves who runs an agent, not who the user is. Each decision also records the edge's own
  `direct` or `intermediary` reading of the verified signer. These are Cloudflare's terms, but
  our edge is on Vultr's CDN, so Cloudflare's own label never reaches us.
- **Most in-browser agents will arrive anonymous.** They look like ordinary Chrome. The door
  gives agents reasons to identify themselves (typed endpoints, higher quotas, actions the
  anonymous tier can't take) and degrades gracefully when they don't. Anonymous rate-limit
  credentials are on watch for R16.
- **Consent: adopt now.** One owner choice in the console writes `Content-Signal` [84] and
  `Content-Usage` [85] lines in robots.txt and a `Content-Usage` header on every signed page.
  The edge sets that header from a policy file signed with the release, not from edge
  configuration.
  The default for a business that wants agents as customers: search and AI input allowed,
  training not. Publishers choose for themselves. Pay per crawl (HTTP 402) is on watch; the
  Mover treats a 402 as "ask the owner".
- **Disclosure for machines too.** Agent-door responses carry a machine-readable
  "AI-operated" statement, and sketches, digests and agent-made artifacts carry
  machine-readable marking (output provenance is U25). As a platform launched after 2 Aug 2026, we assume no Article 50
  grace period applies (§2.5). That is our reading, not legal advice.

### 12.4 Commerce

- **UCP is the stack to mirror [86].** Commerce sites publish a `/.well-known/ucp` profile and
  express catalog, cart, checkout and order in UCP's verbs, with its MCP and A2A bindings. On
  the Shopify "front of house first" path, the catalog points to the store's own UCP endpoint
  until the owner moves checkout.
- **AP2 mandates are verified, not trusted blindly [87].** A mandate carries the buyer's
  authority; the owner's approval (a passkey, or the MFA step-up without one) carries the
  owner's. Both are needed, and they form one chain:
  - the buyer's mandate binds the merchant-signed checkout's hash;
  - that hash is the subject of the order effect's `canonical_args`;
  - the owner's approval fingerprint, `H(site, webagent, tool, canonical_args, amount,
    expiry)` (ADR-006), covers it and stays the idempotency key;
  - the receipt records the chain and which method approved it.

  The fingerprint's binding to site, WebAgent and expiry is what stops an approval being
  replayed on another site or after it expires. `Ap2Desk` uses AP2's first mandate model
  today; v0.2 is U24.
- **Purchase stays on trusted surfaces.** ChatGPT's in-chat checkout reached about 30 Shopify
  merchants before OpenAI moved checkout back to merchants' own sites in March 2026 [97].
  Visiting agents get quotes, artifacts and requests (R19). Commitment happens on the owner's
  approval and the buyer's phone. ACP is used for product feeds.
- **M-Pesa is a gap we can fill.** No agent payment protocol has an M-Pesa binding, and
  Google's own UCP checkout excludes Kenyan merchants today [88]. We propose an open
  "M-Pesa STK Push" payment handler for UCP and AP2. The buyer enters a PIN on their own
  phone, so the phone is the trusted surface and every payment is human-present by design.
  This is the shape Kilele's cake journey already has. It is a proposal; no such standard
  exists yet, and contributing it to UCP or AP2 is a publication, so it is James's call.

### 12.5 What this changes in the experience

- **Peeks must not cover neighbouring menu items.** Screenshot-driven agents can't hover
  reliably, and Google's agent-friendly guidance warns against overlays that hide controls
  [91]. The label still navigates (§4.1), and WebMCP tools give these agents a hover-free path.
- **Accessible pages are the best agent interface.** Agentic browsers read pages through
  ARIA roles and labels [92]. Static, signed, semantic HTML turns tenet 1 into an
  agent-readiness feature.
- **Untrusted content stays marked.** Injection-prone agents will read the site's own pages,
  so visitor-written artifact blocks stay marked, sanitised and `noindex` (R10). The LaunchGate
  screens every release for hidden instructions to an AI, not only the Move's.
- **Publish accuracy, not just engagement.** Publisher "Ask" products follow our pattern
  (their own archive, every source linked, refusal when coverage is thin) [93], but none
  publishes its accuracy. Each site's trust page can show citation coverage and its "doesn't
  say" rate.
- **Every migrated site passes an agent-readiness check.** Cloudflare's free scanner runs as
  an automated acceptance test at go-live (§9), allowing for its draft Server Card path.

### 12.6 Sequencing

| When | What |
|---|---|
| **Before Sunday's demo** (inside the F1 frame pull request; only what is costly to change later) | Extension frames namespaced `awp.*`, leaving room for an AG-UI serialiser; citations carrying a title, shown as a numbered source list under each answer. RFC 6902 is fixed for `awp.artifact.patch` when F4 lands. If time allows: Markdown negotiation from the twins, and robots.txt with both consent signals. *Settled with the framework work on 7 Nov 2026. Inline `[n]` chips (`search_site` numbering its hits across the turn, plus spans on `reply`) move to the pilot unless James pulls them into the demo; that is about an hour, ahead of the sandbox PRs* |
| **Pilot** | Stateless MCP with a Server Card; the AI Catalog; a signed A2A card; U4 push for held calls; Web Bot Auth -01 signing and verifying (U23), with the crawler registered at Cloudflare once James agrees; UCP and AP2 v0.2 (U24) with a draft M-Pesa handler; output provenance marking (U25); the AG-UI serialiser; WebMCP behind feature detection; `Content-Usage` headers; the readiness scanner in go-live checks |
| **Fleet** | MCP Apps export for artifacts; anonymous rate-limit credentials; the W3C Agentic Web working group once chartered |

### 12.7 Where Zavora contributes upstream

The gaps these standards leave are exactly where this design is distinctive, so Zavora gains
more by contributing than by asking agents to learn another protocol:
- **AG-UI:** events for presence, handover to a person and citations pinned to a release.
- **MCP:** a SEP for held-call semantics, and AI Catalog entries for AWP.
- **A2A:** push notifications for held tasks.
- **UCP's Payments Technical Council:** the M-Pesa STK Push handler.
- **W3C Web Machine Learning Community Group:** WebMCP, which is free and open to join.
- **IETF webbotauth:** the case of a site that signs as itself.

## 13. Concepts

All concepts are in [`concept/experience/`](../concept/experience/) and labelled "Concept ·
illustrative data". They use only fictional or Zavora-owned brands: Kilele Bakery
(`kilele-bakery.example`), Classy Perfumes, zavora.ai, and The Kestrel Post (fictional,
`kestrelpost.example`). Each was checked in a browser at desktop width and at 375 px.

| File | Shows |
|---|---|
| [index.html](../concept/experience/index.html) | The overview: big ideas, an archetype × tier explorer, the design language, what we won't do |
| [nav-agents.html](../concept/experience/nav-agents.html) | Kilele Bakery: menu items as desks; hover to converse; instant starters; held actions; the page pointing at citations; the team sheet on phones; presence states |
| [creator.html](../concept/experience/creator.html) | Kilele's cake studio: free text mapped to approved options; a sketch from parts; price from rules; honest refusals; share; request; what Wanjiru sees |
| [search.html](../concept/experience/search.html) | Classy Perfumes: suggestions from the static index; an editable reading of the question; honest fit; comparisons; "doesn't say"; plain results |
| [artifact.html](../concept/experience/artifact.html) | Classy Perfumes: a gift plan made, shared with permissions and hidden prices, chosen by an invitee, requested, and seen by the owner as typed fields |
| [presentation.html](../concept/experience/presentation.html) | zavora.ai: a tailored tour from an approved script; stage cues on the real page; interrupt and resume; save as deck |
| [news.html](../concept/experience/news.html) | The Kestrel Post (fictional): desks in the menu; catch me up from device memory; story timeline with corrections; subscriber headline-only; a digest contract |

The concept pages share [experience.css](../concept/experience/experience.css) (WebAgent
components themed by the host site's `--wa-*` tokens) and
[experience.js](../concept/experience/experience.js) (scripted answers; no model is called).
The Classy Perfumes product images are generated renders from the Agentrix Website proof of
concept. They are labelled "Concept render" on the pages; their source is recorded in
[img/SOURCE.md](../concept/experience/img/SOURCE.md).

**The mark's look** is set by ADR-031. Its reference generator is
[`concept/identity/marks.html`](../concept/identity/marks.html), owned by the architecture
work. The experience concepts above still draw the earlier plain rounded square; the states
they show are current.

**The dual journeys** (§6.3) are in [`concept/journeys/`](../concept/journeys/) and use the
living mark. The experience work owns the shared shell
([journeys.css](../concept/journeys/journeys.css), [journeys.js](../concept/journeys/journeys.js)),
the map and the customer's lane; the console work owns the owner's lane (`owner-*.html`).

| File | Shows |
|---|---|
| [index.html](../concept/journeys/index.html) | The map: the owner's set-up, then both lanes in time order, and the four meeting points (order card, payment, payout, refund) |
| [customer-browse.html](../concept/journeys/customer-browse.html) | Step 1: a guest; desks say they are AI; a cited answer; visits counted without cookies |
| [customer-account.html](../concept/journeys/customer-account.html) | Step 2: sign in or create an account only when needed; a passkey first; what the site keeps; a consent receipt per choice |
| [customer-order.html](../concept/journeys/customer-order.html) | Step 3: a catalogue order with every number sourced; a custom cake held for Wanjiru's OK |
| [customer-pay.html](../concept/journeys/customer-pay.html) | Step 4 (the fleet release): PayPal on its own page, in USD at Kilele's rate; M-Pesa and cards as the other rails |
| [customer-after.html](../concept/journeys/customer-after.html) | Step 5: a verifiable receipt, status, a held date change, refunds under and above the signed rule |
| [customer-agent.html](../concept/journeys/customer-agent.html) | Step 6: her own assistant uses the agent door; she still pays on PayPal's page; the mandate chain |
| [customer-security.html](../concept/journeys/customer-security.html) | Step 7: a takeover attempt, a changed payment link, a look-alike site, export and erasure |

## 14. Sources

Accessed 7 Nov 2026 unless stated. "Unverified" marks claims we could not confirm at the
primary source. Where a page blocked fetching, a mirror or an excerpt is noted.

**Chat widgets and service bots**

1. Budiu, "The User Experience of Chatbots," NN/g, 25 Nov 2018. https://www.nngroup.com/articles/chatbots/
2. Rosala, Kenderova, Kohler, "What Is Your Site's AI Chatbot for? Users Can't Tell," NN/g, 20 Mar 2026. https://www.nngroup.com/articles/site-ai-chatbot/
3. Rosala, Kenderova, Kohler, "Less Chat, More Answer," NN/g, 17 Apr 2026. https://www.nngroup.com/articles/less-chat-more-answer/
4. Kenderova, Rosala, Kohler, "10 Guidelines for Designing Your Site's AI Chatbots," NN/g, 24 Apr 2026. https://www.nngroup.com/articles/ai-chatbots-design-guidelines/
5. Whitenton, "Overuse of Overlays," NN/g, 25 May 2015. https://www.nngroup.com/articles/overuse-of-overlays/
6. Gartner press release, 9 Jul 2024. https://www.gartner.com/en/newsroom/press-releases/2024-07-09-gartner-survey-finds-64-percent-of-customers-would-prefer-that-companies-didnt-use-ai-for-customer-service
7. Gartner press release, 2 Sep 2026. https://www.gartner.com/en/newsroom/press-releases/2026-09-02-gartner-finds-only-27-percent-of-customers-would-try-a-chatbot-again-after-a-negative-experience
8. UJET press release (vendor survey, 2022). https://ujet.cx/press-releases/ujet-research-reveals-chatbots-increase-frustration
9. CFPB, chatbots in consumer finance, 6 Jun 2023. https://www.consumerfinance.gov/about-us/newsroom/cfpb-issue-spotlight-analyzes-artificial-intelligence-chatbots-in-banking/
10. *Moffatt v. Air Canada*, 2024 BCCRT 149, 14 Feb 2024. https://decisions.civilresolutionbc.ca/crt/crtd/en/item/525448/index.do
11. The Register, DPD chatbot, 23 Jan 2024. https://www.theregister.com/2024/01/23/dpd_chatbot_goes_rogue/
12. Futurism, Chevrolet dealer chatbot, Dec 2023. https://futurism.com/the-byte/car-dealership-ai (see also The Autopian, 19 Dec 2023: https://www.theautopian.com/there-were-more-than-3000-attempts-to-hack-dealerships-ai-chatbot-this-weekend)
13. Crolic, Thomaz, Hadi, Stephen, "Blame the Bot," *Journal of Marketing* 86(1), 2022; AMA summary. https://www.ama.org/2021/10/20/when-humanlike-chatbots-miss-the-mark-in-customer-service-interactions/
14. Luo, Tong, Fang, Qu, "Machines vs. Humans," *Marketing Science* 38(6), 2019. https://econpapers.repec.org/RePEc:inm:ormksc:v:38:y:2019:i:6:p:937-947

**Disclosure**

15. Regulation (EU) 2024/1689, Article 50. https://eur-lex.europa.eu/eli/reg/2024/1689/oj (read via https://artificialintelligenceact.eu/article/50/)
16. European Commission, guidelines on Article 50, 20 Jul 2026. https://digital-strategy.ec.europa.eu/en/library/guidelines-transparency-obligations-providers-and-deployers-ai-systems
17. California Senate Appropriations analysis of AB 410 (describing SB 1001, B&P Code §17940 et seq.), 18 Aug 2025. https://leginfo.legislature.ca.gov/faces/billAnalysisClient.xhtml?bill_id=202520260AB410
18. SB 243 (2025), via Skadden summary, Oct 2025 (statute page blocked fetching). https://www.skadden.com/insights/publications/2025/10/new-california-companion-chatbot-law
19. FTC, "The Luring Test," 1 May 2023 (archived copy). https://web.archive.org/web/2024/https://www.ftc.gov/business-guidance/blog/2023/05/luring-test-ai-engineering-consumer-trust
20. FTC, inquiry into AI companion chatbots, 11 Sep 2025. https://www.ftc.gov/news-events/news/press-releases/2025/09/ftc-launches-inquiry-ai-chatbots-acting-companions

**What works**

21. Google, "Expanding AI Overviews and introducing AI Mode," 5 Mar 2025. https://blog.google/products-and-platforms/products/search/ai-mode-search/
22. Google, Search at I/O 2026, 19 May 2026. https://blog.google/products-and-platforms/products/search/search-io-2026/
23. Jaźwińska and Chandrasekar, "AI Search Has a Citation Problem," Columbia Journalism Review, 6 Mar 2025. https://www.cjr.org/tow_center/we-compared-eight-ai-search-engines-theyre-all-bad-at-citing-news.php
24. Pew Research Center, 22 Jul 2025. https://www.pewresearch.org/short-reads/2025/07/22/google-users-are-less-likely-to-click-on-links-when-an-ai-summary-appears-in-the-results/ (for site answer generation, see Vertex AI Search: https://docs.cloud.google.com/generative-ai-app-builder/docs/enterprise-search-introduction)
25. Moran and Gibbons, "Generative UI and Outcome-Oriented Design," NN/g, 22 Mar 2024. https://www.nngroup.com/articles/generative-ui/
26. Gibbons and Moran, "AI Chat Is Not (Always) the Answer," NN/g, 1 Mar 2024. https://www.nngroup.com/articles/ai-chat-not-the-answer/
27. Leviathan et al., "Generative UI," Google Research, 18 Nov 2025. https://research.google/blog/generative-ui-a-rich-custom-visual-interactive-user-experience-for-any-prompt/
28. Shopify, "AI commerce at scale," 11 Jan 2026. https://www.shopify.com/news/ai-commerce-at-scale
29. Klarna press release, 27 Feb 2024 (company claims). https://www.klarna.com/international/press/klarna-ai-assistant-handles-two-thirds-of-customer-service-chats-in-its-first-month/
30. Fortune, 9 May 2025 (reporting a Bloomberg interview). https://fortune.com/2025/05/09/klarna-ai-humans-return-on-investment/
31. Franke, Schreier, Kaiser, *Management Science* 56(1), 2010. https://econpapers.repec.org/RePEc:inm:ormnsc:v:56:y:2010:i:1:p:125-140
32. Gartner press release, 25 Jun 2025. https://www.gartner.com/en/newsroom/press-releases/2025-06-25-gartner-sales-survey-finds-61-percent-of-b2b-buyers-prefer-a-rep-free-buying-experience

**Accessibility and speed**

33. W3C, Understanding SC 4.1.3 Status Messages. https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html
34. W3C, Technique ARIA23 (role=log). https://www.w3.org/WAI/WCAG22/Techniques/aria/ARIA23
35. W3C, Natural Language Interface Accessibility User Requirements (Group Draft Note), 3 Sep 2022. https://www.w3.org/TR/naur/
36. web.dev, "Web Vitals." https://web.dev/articles/vitals
37. Nielsen, "Response Times: The 3 Important Limits," NN/g, 1993. https://www.nngroup.com/articles/response-times-3-important-limits/
38. Tan, Messerschmidt, Yin, Nov, CHI 2026 (arXiv). https://arxiv.org/abs/2604.06183

**Landscape**

39. W3Techs, CMS usage, 7 Nov 2026. https://w3techs.com/technologies/overview/content_management
40. W3Techs, WooCommerce, 7 Nov 2026. https://w3techs.com/technologies/details/cm-woocommerce
41. Netcraft, September 2026 Web Server Survey, 30 Sep 2026 (active-site total derived from the share table). https://www.netcraft.com/blog/september-2026-web-server-survey

**Dot**

42. New Computer, "An update on New Computer and Dot" (5 Sep 2025 per [43]). https://new.computer
43. TechCrunch, 5 Sep 2025. https://techcrunch.com/2025/09/05/personalized-ai-companion-app-dot-is-shutting-down/
44. TechCrunch, 21 Jun 2024. https://techcrunch.com/2024/06/21/dots-ai-really-really-wants-to-get-to-know-you/
45. Fast Company, Nov 2023 (read via a mirror). https://www.fastcompany.com/90975882/meet-dot-an-ai-companion-designed-by-an-apple-alum-here-to-help-you-live-your-best-life
46. Frontier Magazine, 15 Nov 2023. https://magazine.frontier.is/human-after-all/
47. Creative Bloq, Dot's logo (date unverified). https://www.creativebloq.com/design/logos-icons/this-new-iphone-apps-clever-logo-is-packed-with-hidden-meaning
48. Screensdesign, Dot onboarding teardown (undated). https://screensdesign.com/showcase/dot-living-history
49. JustUseApp, App Store review aggregate for Dot, 12 Aug 2025. https://justuseapp.com/en/app/6450016041/dot-living-history/reviews
50. Jason Yuan, AI Engineer Summit talk, 2023. https://ai.engineer/talks/ieWT6X2Yh_g-intelligent-interface

**Grok**

51. TechCrunch, 14 Jul 2025. https://techcrunch.com/2025/07/14/elon-musks-grok-is-making-ai-companions-including-a-goth-anime-girl/
52. TechCrunch, 15 Jul 2025. https://techcrunch.com/2025/07/15/of-course-groks-ai-companions-want-to-have-sex-and-burn-down-schools/
53. 9to5Mac, 16 Jul 2025. https://9to5mac.com/2025/07/16/grok-may-be-breaking-app-store-rules-with-sexualized-ai-chatbots-and-thats-not-the-only-problem/
54. The New York Times, 6 Oct 2025 (read via The Nightly). https://thenightly.com.au/society/technology/the-new-york-times-elon-musk-gambles-on-sexualised-ai-companions-c-20295009
55. Sensor Tower, Japan, Aug 2025. https://sensortower.com/ja/blog/grok-with-ani-JP
56. TechCrunch, 12 Jul 2025. https://techcrunch.com/2025/07/12/xai-and-grok-apologize-for-horrific-behavior/
57. TechCrunch, 16 Apr 2025. https://techcrunch.com/2025/04/16/xai-adds-a-memory-feature-to-grok/
58. The Decoder, 26 Feb 2025. https://the-decoder.com/elon-musks-ai-screams-louder-than-the-competition/
59. TechCrunch, 18 Aug 2025. https://techcrunch.com/2025/08/18/crazy-conspiracist-and-unhinged-comedian-groks-ai-persona-prompts-exposed/
60. 9to5Mac, 20 Aug 2025 (citing Forbes). https://9to5mac.com/2025/08/20/370k-grok-ai-chats-made-public-without-user-consent/
61. DFRLab, 24 Jun 2025. https://dfrlab.org/2025/06/24/grok-struggles-with-fact-checking-amid-israel-iran-war/
62. Engadget, 20 Nov 2025. https://www.engadget.com/ai/elon-musk-blames-adversarial-prompting-after-grok-spewed-embarrassing-sycophantic-praise-235157807.html
63. The Register, 12 Jan 2026. https://www.theregister.com/2026/01/12/xai_grok_uk_regulation/
64. TechCrunch, 27 Jan 2026. https://techcrunch.com/2026/01/27/among-the-worst-weve-seen-report-slams-xais-grok-over-child-safety-failures/
65. The Washington Post, 2 Feb 2026 (read via The Nightly). https://thenightly.com.au/society/technology/washington-post-inside-elon-musks-mission-at-xai-to-boost-users-that-led-to-grok-generate-sexualised-images-c-21513148
66. Decrypt, 6 Aug 2025 (citing The Verge). https://decrypt.co/333796/elon-musk-xai-nude-deepfakes-taylor-swift

**Other assistants**

67. De Freitas et al., arXiv, Dec 2024 (Replika identity change). https://arxiv.org/abs/2412.14190
68. Fortune, 29 Oct 2025 (Character.AI and minors). https://fortune.com/2025/10/29/character-ai-ban-children-teens-chatbots-regulatory-pressure-age-verification-online-harms
69. OpenAI and MIT Media Lab, affective use study, Mar 2025. https://openai.com/index/affective-use-study
70. TechCrunch, 10 Apr 2025 (ChatGPT memory). https://techcrunch.com/2025/04/10/openai-updates-chatgpt-to-reference-your-other-chats/
71. Simon Willison, 21 May 2025. https://simonwillison.net/2025/May/21/chatgpt-new-memory/
72. OpenAI, "Sycophancy in GPT-4o," 29 Apr 2025 (blocked; confirmed via [73]). https://openai.com/index/sycophancy-in-gpt-4o/
73. OpenAI, "Expanding on what we missed with sycophancy," 2 May 2025 (read via https://simonwillison.net/2025/May/2/what-we-missed-with-sycophancy/). https://openai.com/index/expanding-on-sycophancy/

**Agent-web standards (research, 7 Nov 2026; the full report cites many more)**

74. CopilotKit, "AG-UI 1.0," 30 Sep 2026. https://www.copilotkit.ai/blog/ag-ui-1.0
75. AG-UI docs, events and interrupts. https://docs.ag-ui.com/concepts/events
76. MCP blog, specification 2026-07-28. https://blog.modelcontextprotocol.io/posts/2026-07-28/
77. MCP SEP-2127, Server Cards (Final, 6 Oct 2026). https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/main/seps/2127-mcp-server-cards.md
78. AI Catalog. https://ai-catalog.io/
79. A2A specification (v1.0.1). https://a2a-protocol.org/latest/specification/
80. WebMCP, W3C Web Machine Learning CG draft, 2 Nov 2026. https://webmachinelearning.github.io/webmcp/
81. Cloudflare, "Agent readiness" (Radar top 200,000 domains). https://blog.cloudflare.com/agent-readiness/
82. Ahrefs, llms.txt study (May 2026 logs). https://ahrefs.com/blog/llmstxt-study/
83. IETF, draft-ietf-webbotauth-httpsig-protocol-01, 29 Oct 2026. https://datatracker.ietf.org/doc/html/draft-ietf-webbotauth-httpsig-protocol-01
84. Cloudflare, "Content Signals Policy," 24 Sep 2025. https://blog.cloudflare.com/content-signals-policy/
85. IETF, draft-ietf-aipref-attach-05. https://www.ietf.org/archive/id/draft-ietf-aipref-attach-05.html
86. Universal Commerce Protocol. https://ucp.dev/
87. AP2 specification (v0.2). https://ap2-protocol.org/ap2/specification/
88. Google Merchant Center, UCP integration eligibility. https://support.google.com/merchants/contact/ucp_integration_interest?hl=en
89. A2UI v1.0 specification. https://a2ui.org/specification/v1.0-a2ui/
90. MCP Apps specification, 26 Jan 2026. https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx
91. Search Engine Journal, Google's agent-friendly checklist. https://www.searchenginejournal.com/googles-agent-friendly-checklist-is-the-accessibility-audit-restated/575495/
92. OpenAI Help Center, Atlas publishers and developers FAQ. https://help.openai.com/en/articles/12627856-publishers-and-developers-faq
93. The Audiencers, how The Washington Post built Ask The Post AI. https://theaudiencers.com/the-washington-post-how-and-why-we-built-ask-the-post-ai/
94. Cooley, Ninth Circuit on AI agent access under the CFAA, 6 Aug 2026. https://www.cooley.com/news/insight/2026/2026-08-06-ninth-circuit-rules-on-ai-agent-access-to-third-party-websites-under-cfaa
95. Cloudflare press release, AI crawlers blocked by default, 1 Jul 2025. https://cloudflare.com/press-releases/2025/cloudflare-just-changed-how-ai-crawlers-scrape-the-internet-at-large
96. MUI X Chat, accessibility. https://mui.com/x/react-chat/accessibility/
97. CNBC, 20 Mar 2026 (OpenAI moves checkout to merchants). https://www.cnbc.com/2026/03/20/open-ai-agentic-shopping-etsy-shopify-walmart-amazon.html

**Counting visits**

98. EDPB, Guidelines 2/2023 on the technical scope of Art. 5(3) ePrivacy Directive, v2.0, 7 Oct 2024. https://edpb.europa.eu/system/files/2024-10/edpb_guidelines_202302_technical_scope_art_53_eprivacydirective_v2_en_0.pdf
99. CNIL, "Sheet n°16: Use analytics on your websites and applications." https://www.cnil.fr/en/sheet-ndeg16-use-analytics-your-websites-and-applications
100. Clifford Chance, the ICO's guidance on the new cookie exceptions, Sep 2025 (in-force date from secondary reports). https://www.cliffordchance.com/insights/resources/blogs/talking-tech/en/articles/2025/09/uk-ico-s-updated-guidance-for-new-exceptions-to-cookie-consents-.html
101. PPC Land, the Council's Digital Omnibus mandate, Oct 2026 (secondary). https://ppc.land/eu-council-agrees-4-month-cookie-re-ask-wait-for-digital-omnibus-talks/
102. ODPC Kenya, guidance note on consent, 30 Nov 2023 (via Digital Policy Alert). https://digitalpolicyalert.org/change/12410-odpc-guidance-notes-on-consent

**The living mark**

103. W3C, Understanding SC 1.4.11 Non-text Contrast. https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html
104. W3C, Understanding SC 2.2.2 Pause, Stop, Hide. https://www.w3.org/WAI/WCAG22/Understanding/pause-stop-hide.html

**The customer's journey**

105. PayPal Developer, "Currency codes" (supported currencies; KES is not listed). https://developer.paypal.com/api/rest/reference/currency-codes/
106. PayPal Developer, "Agent Ready: overview" (ACP via Braintree for ChatGPT; UCP via Braintree or PayPal's Orders v2 API for Google AI Mode and Gemini). https://developer.paypal.com/agent-ready/overview
107. W3C, Web Authentication Level 3 (credentials are scoped to a Relying Party). https://www.w3.org/TR/webauthn-3/
