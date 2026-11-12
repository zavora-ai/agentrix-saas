# 01a · The website: visitor, customer and agent screens

Status: analysis v0.1, Sat 7 Nov 2026. Scope: every screen a visitor, a signed-in customer, an
invited person or a visiting AI agent meets on a migrated site. The owner's console is covered
elsewhere; this page names what the owner must be able to administer for each sample, so the
console work can pick it up.

Read with: [how-we-work.md](how-we-work.md) (build rules), [voice.md](voice.md) (words),
`docs/06-website-experience.md` (the design), `docs/01-product.md`, `docs/05-webagents.md`
§3.3, §4.4–4.5, §5.1, §7.

Sources read in full: `concept/site.html`, `concept/styles.css`, everything in
`concept/experience/`, `concept/journeys/index.html`, every `concept/journeys/customer-*.html`,
`concept/journeys/journeys.css`, `concept/journeys/journeys.js`, `concept/identity/marks.html`,
docs 06 and 01, the customer-facing parts of 05, and the code in
`crates/agentrix-node/src/{site,door,pages}`, `crates/agentrix-console/{templates,static}` and
`crates/agentrix-move/src/theme.rs`.

## How to read this

- **IDs.** `K-` Kilele Bakery, `C-` Classy Perfumes, `N-` the news site, `Z-` zavora.ai, `A-`
  customer account pages (shared by every site), `R-` artifacts (shared), `M-` machine surfaces
  for agents, `X-` system pages and global states, `S-` the shell on every page, `G-` gaps.
- **Persona.** G guest visitor, C signed-in customer, I invited person (holds an artifact link),
  A visiting AI agent, O the owner seeing their own site.
- **Built.** **Built**: code serves it today. **Partial**: some of it runs. **Drawn**: a concept
  shows it, no code. **Implied**: a concept cites or needs it but never draws it. **New**: not in
  the design at all; production needs it.
- **Workstreams.** W1 WebAgent runtime, W2 Sample websites, W3 Control plane, W4 Onboarding and
  owner journey, W5 Owner console, W6 Identity, payments and trust, W7 Design system and voice.
  Two new ones are proposed in §7: **W8 Commerce rules and records** and **W9 Customer
  messaging**.
- **Citations** are `path:line` from the repository root.

---

## 0. Summary

1. **The three samples.** The design draws four brands. The identity reference
   (`concept/identity/marks.html:274-276`, `:466-507`) names exactly three sample sites with
   full teams: Kilele Bakery, Classy Perfumes and Tana Valley Times. The control plane's ring 0
   asks for synthetic sites per archetype "bakery, clinic, shop, news"
   (`docs/08-control-plane.md:411`). Recommendation: **Kilele Bakery** (local services, T3),
   **Classy Perfumes** (shop, T3) and **Tana Valley Times** (local news, T2) built with the
   newsroom patterns drawn for The Kestrel Post. zavora.ai's tour is a fourth, stretch sample.
2. **What is drawn.** Kilele's customer lane is drawn in depth: guest, account, order, pay,
   after the order, her AI agent, security moments. Classy is drawn only as search and a gift
   plan. The news site is one front page. zavora.ai is one long page with a tour.
3. **What is built.** The edge serves signed releases with Markdown twins, a generic page
   theme, a floating "Ask this site" panel with cited answers and "pass to the owner", and an
   agent door with two MCP tools (`ask_site`, `request_custom_order`). No desks in the menu, no
   catalogue, cart, orders pages, payments, customer accounts, artifacts, creator, search page,
   digest or tour exist in code.
4. **Inventory.** 90 entries: 77 screens people see and 13 machine surfaces for agents. Of the
   77 screens, 39 are drawn (in whole or as a section of a page), 6 are built or partly built,
   and 32 are implied by the design or new. Of the 13 machine surfaces, 7 are built.
5. **Biggest gaps.** A data model and rules engine for catalogue, prices, stock, slots, quotes
   and orders (W8); customer accounts on the site's own domain, including returning sign-in and
   recovery (W6); every customer email and WhatsApp message (W9); decline, lapse and payment
   failure states; the owner's admin for catalogue, orders and stories (W5); Classy's home,
   product, bag and checkout pages; and the news site's story, section and subscription pages.
6. **Conflicts to settle before seeding.** Kilele's facts differ between the concepts and the
   WordPress fixture (address, hours, prices, notice, delivery fee, deposit). Desk names,
   order-number formats, brand fonts, the agent door's tool names and its discovery path also
   differ between files. §1.2 lists them; §8 asks James for the decisions.

---

## 1. The three sample websites

### 1.1 What the design shows

| Brand | Archetype (06 §3.3) | Where it is drawn | Go-live → ceiling | People | Domain in the design |
|---|---|---|---|---|---|
| **Kilele Bakery** | Local services | `concept/site.html`, `concept/experience/nav-agents.html`, `concept/experience/creator.html` (the "cake studio"), every `concept/journeys/customer-*.html`, `concept/identity/marks.html:466-478`, the WordPress fixture `fixtures/wordpress-infected/` | T1 → T3 | Wanjiru (owner), Njeri (customer, London), Amina (niece, 7) | `kilele-bakery.example` |
| **Classy Perfumes** | WooCommerce shop | `concept/experience/search.html`, `concept/experience/artifact.html`, product renders `concept/experience/img/*.jpg`, `concept/identity/marks.html:480-493` | T1 → T4 | "The shop owner" (no name), Achieng and five bridesmaids | `classyperfumes.shop` |
| **The Kestrel Post** | National news publisher | `concept/experience/news.html`; journey 4 in `docs/06-website-experience.md:946-954` | T1 → T3 | Reporters Amani Otieno, Lea Mwangi, Daniel Kiprop | `kestrelpost.example` |
| **Tana Valley Times** | Local news, weekly | `concept/identity/marks.html:494-507` only | not stated | none | not stated |
| **zavora.ai** | Company site | `concept/experience/presentation.html` | T1 → T4 | An agency owner with 40 WordPress sites | `zavora.ai` |

"Cake Studio" is not a brand. It is Kilele's creator page at `/cakes/studio`
(`concept/experience/creator.html:125-131`); the fixture's About page says Achieng runs the cake
studio (`fixtures/wordpress-infected/site/seed.php:98`).

### 1.2 Ambiguities and conflicts in the drawn design

| # | What differs | Where | Effect on the build |
|---|---|---|---|
| A1 | **Which newsroom.** The Kestrel Post (national, thousands of stories a week) has the drawn pages; Tana Valley Times (local weekly) has the mark team, palette and fonts. | `concept/experience/news.html:108`; `concept/identity/marks.html:494-507` | One brand must be picked before seeding. |
| A2 | **Kilele's address.** Westlands, Woodvale Grove (`concept/site.html:191`, `:267`; `concept/experience/nav-agents.html:149`, `:195`); Kilimani (`concept/journeys/customer-browse.html:62`; `concept/journeys/index.html:95`); Ngong Road opposite Adams Arcade (`fixtures/wordpress-infected/site/seed.php:89`, `:111`). | as listed | Citations and the Visit desk will contradict each other. |
| A3 | **Kilele's hours.** Mon–Sat 6:30–19:00, Sun 7:00–14:00 (`concept/experience/nav-agents.html:194`) against Tue–Sun 7am–6pm, closed Mondays (`fixtures/wordpress-infected/site/seed.php:112`). | as listed | Instant starter "Are you open on Sunday?" answers differently per source. |
| A4 | **Cake prices and notice.** By tier: 1 tier KSh 3,800, 2 tiers 4,500, 3 tiers 6,900, 2 days' notice (3 for 3 tiers) (`concept/experience/nav-agents.html:171-177`, `concept/experience/creator.html:231`). By flavour: vanilla 2-tier 4,500, red velvet 3,800, chocolate fudge 4,200 (`concept/site.html:232`). By weight: 1 kg KES 3,500, 2 kg 6,500, 3 kg two-tier 11,000, three days' notice, weddings three weeks, 50% deposit (`fixtures/wordpress-infected/site/seed.php:102-106`). | as listed | The configurable product cannot be seeded until one price model is chosen. |
| A5 | **Delivery.** KSh 300 to Westlands and Kilimani, Sundays 9:00–13:00 (`concept/experience/nav-agents.html:196`); Saturday slots 10:00–12:00 and 14:00–16:00 (`concept/journeys/customer-order.html:121-127`); KES 500 within Nairobi (`fixtures/wordpress-infected/site/seed.php:104`). | as listed | Slot calendar and fees conflict. |
| A6 | **Change cut-off.** "Free until 24 hours before pickup" (`concept/experience/nav-agents.html:181`); "Changes close Thu 12 Nov, 18:00" for a Sat 10:00 delivery, about 40 hours (`concept/journeys/customer-after.html:213`); "up to two days before collection" (fixture FAQ, `seed.php:124`). | as listed | The change rule must be one value. |
| A7 | **Confirmation time.** "Usually within 2 hours, 7:00–20:00, on WhatsApp" (`concept/experience/nav-agents.html:187`) against "by phone within one working day" (`seed.php:106`). | as listed | Honest wait times must come from one stated value (R14). |
| A8 | **Kilele desk names.** Bakes, Cakes, Orders, Visit (`concept/experience/nav-agents.html:283-300`) against Bread, Cakes, Orders (`concept/journeys/journeys.js:54-63`, `concept/identity/marks.html:473-477`). | as listed | Desk slugs are fixed at creation (`docs/05-webagents.md:602-604`); pick once. |
| A9 | **Order numbers.** KB-1042, KB-1043, KB-1047, H-0193 (`concept/site.html:246`, `:329`; `concept/experience/creator.html:451`; `concept/experience/nav-agents.html:313`) against K-1042, K-1043 (`concept/journeys/*`). | as listed | One format per site. |
| A10 | **Kilele brand tokens.** Fraunces, ink #3b2416, cream #fbf4ea (`concept/journeys/journeys.css:211-218`) against Young Serif, ink #2b1d14, bg #fbf6ef (`concept/identity/marks.html:468-469`). | as listed | Seed one token set. |
| A11 | **Classy desks and tokens.** Scents, Gifts, Orders, Journal; Gifts #4a2340; Cormorant Garamond (`concept/experience/search.html:14-19`, `:165`, `:227`) against Gifts, Fragrances, Orders, Reputation, Cashier; Gifts #6d2e4f; Italiana (`concept/identity/marks.html:482-491`). | as listed | Pick names, colours and fonts once. |
| A12 | **Is Classy Perfumes real?** 06 calls the brands "fictional or Zavora-owned" (`docs/06-website-experience.md:1379-1380`). `classyperfumes.shop` is a ring 0 production site (`docs/08-control-plane.md:411`) and a pilot site (`docs/01-product.md:273-274`). The concept products (Rain Oud, Amber Noir) are generated renders (`concept/experience/img/SOURCE.md`). | as listed | A sample with invented products must not run on the real shop's domain. |
| A13 | **Ask surface.** An inline "Ask Kilele Bakery" panel (`concept/site.html:198-213`); a docked Ask bar plus a side rail (`concept/experience/nav-agents.html:203-229`); in code, a fixed corner panel (`crates/agentrix-console/static/site/ask.css:11`). 06 says the Ask bar sits in the page's layout and is never a floating bubble (`docs/06-website-experience.md:358-360`). | as listed | The code drifts from the design. |
| A14 | **The Ask desk's name.** "AI concierge" (`concept/site.html:199`, `:212`, `:273`); "Front desk" (`concept/experience/nav-agents.html:220`, `:301`); "Concierge" (`crates/agentrix-console/static/site/ask.js:28`, `crates/agentrix-node/src/site/ask.rs:707`). 06 says visitors see "Front desk" (`docs/06-website-experience.md:587`). | as listed | Rename in code. |
| A15 | **Agent door tools.** `menu.list`, `stock.get`, `order.request`, `quote.request` (`concept/site.html:316-323`); `catalog.search`, `delivery.slots`, `quote.create`, `checkout.create`, `request_custom_order` (`concept/journeys/customer-agent.html:125-148`); `ask_site`, `request_custom_order` in code (`crates/agentrix-node/src/door/mod.rs:80-102`). | as listed | One manifest must name them (`docs/06-website-experience.md:1223-1228`). |
| A16 | **Tier for orders at the door.** `verified` (`concept/site.html:294`), `known` (`concept/journeys/customer-agent.html:171-178`), `Public` in code (`crates/agentrix-node/src/door/mod.rs:98`). | as listed | Decide per capability. |
| A17 | **Discovery path.** `/.well-known/awp.json` (`concept/site.html:287`), `/.well-known/awp` (`crates/agentrix-node/src/door/mod.rs:73`), `/.well-known/ai-catalog.json` (`concept/journeys/customer-agent.html:127`; `docs/06-website-experience.md:1260`). | as listed | Serve the AI Catalog and keep `/.well-known/awp` as the AWP descriptor it lists. |
| A18 | **MCP version.** Code speaks 2025-06-18 and 2025-03-26 with session ids (`crates/agentrix-node/src/door/mod.rs:42`, `:222-225`); 06 adopts stateless MCP 2026-07-28 (`docs/06-website-experience.md:1258`). | as listed | Pilot item; note it. |
| A19 | **Per-person artifact link.** `classyperfumes.shop/a/plan-k29x?for=wambui` puts a person's name in the URL (`concept/experience/artifact.html:300`). 06 requires hashed, revocable share tokens (`docs/06-website-experience.md:1149`). | as listed | Use an opaque token per person. |
| A20 | **Another Kilele domain.** `kilelebakery.co.ke` appears in [voice.md](voice.md)'s examples. | `specs/platform/voice.md` | A real-looking `.co.ke` may belong to someone. Use `.example` in docs and a Zavora host for the sample. |
| A21 | **Name reuse.** Achieng is Kilele's cake decorator (`seed.php:98`) and Classy's customer; Njeri is Kilele's customer and a Classy bridesmaid (`concept/experience/artifact.html:197`). | as listed | Harmless, but seed data reads better with distinct names. |

### 1.3 Recommendation

Build these three, end to end, each with a visitor side, a customer account and the owner's
admin:

| Sample | Why this one |
|---|---|
| **Kilele Bakery** (local services) | The most-drawn site. It carries the first release demo (the agent race ends at Wanjiru's card), the custom-order hold, the catalogue order, PayPal in USD at a posted rate, M-Pesa, refunds under a signed rule, and all four security moments. The fixture already exists as its "old site". |
| **Classy Perfumes** (shop) | The only drawn shop: agentic search, honest fit, comparisons, a shared gift plan with per-person permissions, and the request that turns five choices into one order. It exercises artifacts (R10) and catalogue scale that Kilele doesn't. |
| **Tana Valley Times** (local news, weekly) with The Kestrel Post's drawn patterns | It is the third team in the identity reference. A local weekly can be seeded honestly with about 60 stories; a "national" paper with 60 stories would look fake. It exercises subscriber accounts, entitlements, device memory, corrections and a digest with double opt-in, which neither shop covers. Port news.html's catch-me-up, story timeline, digest contract and house rules onto Tana's brand. |

Stretch: **zavora.ai** with the tour (`concept/experience/presentation.html`). It is Zavora's
own site, so it can run as itself, and it exercises presentation scripts (R11) and a deck
artifact. It is also where "Assess my site" leads into onboarding (W4).

Rules for all samples (from [how-we-work.md](how-we-work.md) and 01 §5):

- Each sample says **"Sample"** in its header band, in the style of the fixture banner
  (`crates/agentrix-console/templates/site/page.html:31`). Every person, phone number and email
  is fictional: phones in reserved ranges (`+44 7700 900xxx` as the concepts do, and a Kenyan
  test range agreed with W6), emails at `example.com`.
- Host each sample on a Zavora-controlled host (for example a subdomain of `agentrix.website`,
  as the fixture uses `fixture.agentrix.website`) and keep `.example` names in documents. W3
  decides the hosts.
- Payments run only in provider sandboxes (PayPal sandbox; M-Pesa Daraja sandbox). No sample
  ever takes a real payment.
- Seeds are files in the repository, versioned with the release, reproducible from scratch, and
  each sample doubles as a ring 0 synthetic canary (`docs/08-control-plane.md:411`).

### 1.4 Sample A: Kilele Bakery

**Brand.** Fictional Nairobi bakery, since 2016, owner Wanjiru. Recommended tokens (journeys,
the latest drawn set): cream `#fbf4ea`, cream-2 `#f5e9d8`, brown `#3b2416`, brown-2 `#6b4b36`,
terracotta `#b4541f` (hover `#8f3f13`), line `#ecdcc6`, ok `#2f6b3a`; display Fraunces; text DM
Sans (`concept/journeys/journeys.css:211-218`). Mark family `grain`, squareness 4.6
(`concept/journeys/journeys.js:51-54`).

**Archetype and tier.** Local services. The sample targets T3: desks in the menu, holds,
catalogue and custom orders, the cake studio, shared designs.

**Pages.** K-01 to K-19 in §2.3, plus the shared account pages (§2.4), artifacts (§2.6) and the
agent door (§2.9).

**Front of house (menu desks).** Recommended: Bread, Cakes, Orders, Visit, plus the Front desk
behind the Ask bar.

| Desk | Glyph, colour, seed | Mandate (visitor words) | Live facts in the peek | Starters | Capability subset |
|---|---|---|---|---|---|
| Bread | loaf, `#9c5725`, 3 | "I know what came out of the oven today, what's left and when the next batch is due." (`concept/experience/nav-agents.html:285`) | Stock per item, next batch, as-of time | What's still warm? (instant); Anything without dairy?; Can you hold 2 loaves for me? | `catalog.read`, `stock.read`, `hold.create` (standing grant: up to 3 loaves until 12:00) |
| Cakes | cake, `#8e3b46`, 7 | Design a cake from Kilele's own options, priced from the price list, confirmed by Wanjiru (rewrite in §6.1) | Next free date, notice, "from" price | Design a birthday cake with me; How much for 30 people?; Is anything gluten-free? | `catalog.read`, `calendar.read`, `configure.cake`, `quote.create`, `artifact.*` (brief), `request_custom_order` (held) |
| Orders | bag, `#3b2416`, 11 | "I find your order, tell you exactly where it stands, and pass changes to Wanjiru." (`concept/experience/nav-agents.html:293`) | Confirmations usually within 2 h; free changes until the cut-off | Where is my order?; Change my delivery time; Can I cancel? | `order.status` (status only to guests), `order.verify` (one-time code), `order.change_request` (held), `order.cancel_request` (held), `refund.request` (Cashier rule), `payment_request.check` |
| Visit | pin, `#3b2416` | "Hours, directions, parking and where we deliver." (`concept/experience/nav-agents.html:297`) | Open now until 19:00; delivery areas | Are you open on Sunday? (instant); Do you deliver to Lavington?; Where do I park? (instant) | `hours.read`, `delivery.zones.read` |
| Front desk | search | Routes a question to the right desk | none | none | `search_site`, hand over to a desk |

**Back office WebAgents customers meet indirectly.** Cashier (makes the payment request after
approval, refunds under the signed rule; `concept/journeys/customer-pay.html:194`,
`concept/journeys/customer-after.html:264-310`), Chief WebAgent (the owner's brief), Sales
(consented follow-up after collection, `docs/05-webagents.md:593-594`), Editor and Guardian.
Customers never talk to them directly; the Orders desk says when the Cashier acts.

**What the owner administers** (W5; none of these screens is drawn, see G-40):

- **Catalogue and price list:** breads, pastries, the weekend bake box, prices, ingredients and
  allergens, the posted USD rate. Confirming a price list is a release.
- **The cake configurable product:** sizes, sponges, colours, finishes, decorations, price
  deltas, constraints, lead times, sketch parts (`docs/06-website-experience.md:738-742`).
- **Daily bake plan and stock:** what is baked for which day, sold-out marks, next batch times.
- **Delivery calendar:** zones, fees, slots, capacity per slot and per cake size.
- **Orders:** the order list, decision cards for custom orders and changes, status updates
  (baking, out for delivery, delivered), cancellations.
- **Standing rules:** holds (up to 3 loaves), refunds (up to KSh 2,000 an order, 5 a week, to
  the original method, until 31 Dec 2026, `concept/journeys/customer-after.html:331-337`).
- **Desks:** names, mandates, starters, "may promise" lists, marks (`docs/06-website-experience.md:1037-1052`).
- **Teach your site:** questions passed to the owner (built: `crates/agentrix-node/src/site/pass.rs`).
- **Customers:** accounts, threads under retention, and the takeover of a live thread.
- **Look-alike and payment-link reports.**

**Seed data** (recommended values; each row is one fact with its source page):

| Fact | Value | Source page (anchor) | Conflict |
|---|---|---|---|
| Address | Pick one (A2). Recommendation: Westlands, Woodvale Grove, parking at the back | `/visit#find-us` | A2 |
| Hours | Mon–Sat 6:30–19:00; Sun 7:00–14:00 | `/visit#hours` | A3 |
| Breads | Country sourdough KSh 450 (flour, water, salt); butter croissant KSh 180 (butter, milk, egg); cinnamon bun KSh 220 (butter, milk); red velvet slice KSh 350 (butter, milk, egg); rye and caraway KSh 500 (fixture) | `/bread#…` per item | none |
| Weekend bake box | KSh 1,350: a sourdough loaf, four cinnamon buns and a banana loaf; baked Saturday morning | `/shop/weekend-bake-box` | none |
| Stock (daily plan) | Example day: sourdough 6, red velvet 12, cinnamon sold out until 6:30, croissants next batch 10:00 | `stock` capability, as-of time | Must be generated per day, never a fixed figure on a page |
| Cake sizes | 1 tier serves 12–15 KSh 3,800; 2 tiers 25–30 KSh 4,500; 3 tiers 40–45 KSh 6,900 | `/cakes#row-1tier` etc. | A4 |
| Notice | 2 days; 3 days for 3 tiers; weddings 3 weeks | `/cakes#notice` | A4 |
| Sponges | vanilla, chocolate +200, red velvet +300, lemon; gluten-free +600 on 1 and 2 tiers only | `/cakes#options` | none |
| Buttercream | cream, blush pink, forest green, chocolate, sky blue | `/cakes#options` | none |
| Finish | smooth, ombré +250, naked, chocolate drip +400 | `/cakes#options` | none |
| Decoration | fresh berries +500, edible flowers +700, sprinkles +150, none | `/cakes#options` | none |
| Message | Up to 22 characters, free | `/cakes#options` | none |
| Not offered | Sculpted figures, printed photos, anything sooner than the notice | `/cakes#not-offered` | Needs a page so the refusal can be cited |
| Allergens | "The kitchen also uses wheat flour, so Kilele can't promise anything is safe for coeliac guests." | `/allergens` | none |
| Delivery | Westlands and Kilimani KSh 300; slots Sat 10:00–12:00, 14:00–16:00; Sun 10:00–12:00; one fee per delivery when orders share a slot | `/delivery` | A5 |
| Changes and cancellation | One cut-off. Recommendation: free until 24 hours before the slot | `/orders/help#changes` | A6 |
| Confirmation time | Usually within 2 hours, 7:00–20:00 | `/orders/help#confirmations` | A7 |
| USD rate | 1 USD = KSh 129.00, posted in the price list, pinned per order | `/cakes#price-list` | none |
| Refund rule | KSh 2,000 an order, 5 a week, original method, until 31 Dec 2026 | owner's signed rule (not a page) | none |
| Hold grant | Up to 3 loaves until 12:00, no charge | owner's signed rule | none |
| Release | Price list "v14", dated 2 Nov | release manifest | Keep "price list v14" (owner's word) apart from the release sequence |
| Seed customer | Njeri, njeri@example.com, +44 7700 900418, London; recipient Amina's home in Kilimani | account seed | Needs a recipient name and phone field (G-17) |
| Seed orders | K-1042 (catalogue, KSh 2,550 = USD 19.77); K-1043 (custom cake, KSh 5,250 = USD 40.70), delivery Sat 14 Nov 10:00–12:00 | order seed | A9 |
| Migrated pages | Home, About, Custom cakes, Contact, Our menu, Bread, Wholesale, Baking classes (KES 4,500), Questions, Journal (40 posts) | `fixtures/wordpress-infected/site/seed.php:136-176` | The fixture's old facts are the "before" |

**Recommendation on the fixture conflict.** The fixture is the old site, and other lanes' tests
embed its exports (`fixtures/wordpress-infected/README.md`, "exports/dev ... are kept as they
are"). Do not edit it. Seed the sample's live release from the values above and treat the
fixture's older facts as what the owner corrected during onboarding ("Your price list says 2023.
Confirm the prices?", `docs/01-product.md:68`). That makes the sample's history honest: the
facts changed because the owner confirmed new ones, and each confirmation is a release with a
receipt. James decides (§8, D1).

### 1.5 Sample B: Classy Perfumes

**Brand.** Fragrance and gifts, Westlands. Tokens drawn in search and artifact: ink `#1c1719`,
ink-2 `#5b5056`, bg `#f7f3ee`, line `#e7ddd2`, gold `#a8842f`, plum `#4a2340`, dark header
`#1a1418`; display Cormorant Garamond (`concept/experience/search.html:14-19`). The identity
reference uses Italiana and a different palette (A11). Mark family `facets`, squareness 6.2
(`concept/identity/marks.html:483-485`).

**Domain.** See A12. Recommendation: run the sample on a Zavora sample host with a "Sample"
band and a fictional catalogue, unless James confirms the sample should be the real shop's
catalogue (then it is not a sample and no invented data may appear).

**Archetype and tier.** WooCommerce shop. The sample targets T3: agentic search, comparisons,
gift plans, orders, M-Pesa and PayPal.

**Pages.** C-01 to C-10 (§2.5), shared account pages, artifacts, agent door. Order pages reuse
Kilele's templates (K-07 to K-13) with Classy's tokens.

**Front of house.** Recommended: Fragrances, Gifts, Orders, Journal, plus a Front desk ("Shop
desk" in `concept/experience/search.html:228`).

| Desk | Mandate | Starters (proposed from the drawn scenarios) | Capability subset |
|---|---|---|---|
| Fragrances | Explains notes, sizes and what is in stock (`concept/identity/marks.html:488`) | What smells like rain?; Which is lightest?; Is it in stock? | `catalog.search`, `stock.read`, `compare.build` |
| Gifts | Helps choose a gift and have it wrapped (`concept/identity/marks.html:487`) | A gift under KSh 5,000; Plan gifts for a group; Help me write the card | `catalog.search`, `quote.create`, `artifact.*` (plan, comparison), `request_order` (held above threshold) |
| Orders | Takes orders and delivery details (`concept/identity/marks.html:489`) | Where is my order?; Do you deliver to Mombasa?; Can I return it? | `order.status`, `order.verify`, `delivery.zones.read`, `returns.read` |
| Journal | Answers from journal posts only (proposed) | Which scent for the long rains? | `search_site` scoped to `/journal` |

**Back office.** Cashier (M-Pesa reconciliation, refunds above the limit ask,
`concept/identity/marks.html:491`), Reputation (review replies, owner approves), Sales.

**What the owner administers.** Products (name, size, price, notes, family, images, stock),
collections, gift wrapping, delivery zones and fees, returns policy, orders and decision cards
(the plan request, `concept/experience/artifact.html:279-294`), standing rules (order
threshold, refunds), reviews, teach your site, desk settings.

**Seed data:**

| Fact | Value | Source |
|---|---|---|
| Vanilla Gift Set | KSh 4,500 · vanilla candle and 30 ml travel spray · gift-wrapped with a card · family Gourmand · stock 7 | `concept/experience/search.html:230` |
| Citrus Dawn | KSh 4,900 · 50 ml · bergamot and neroli over a clean musk · Fresh citrus · stock 3 | `:231` |
| Amber Noir | KSh 6,400 · 50 ml · amber, smoked vanilla, oud · Amber · stock 14 | `:232` |
| Rain Oud | KSh 5,800 · 30 ml · petrichor and oud · Woody · long-rains limited run · stock 9 | `:233` |
| Not published | Longevity, vegan status. Search must grey these refinements out (`:260`, `:304`) | `:211` |
| Delivery and returns | Nairobi before 2 pm same day KSh 300; Mombasa and other towns 2–3 working days KSh 650; unopened bottles returnable within 14 days; free 2 ml samples | `:236` |
| Gift wrapping | Free on every order, with a handwritten card | `:237` |
| Journal | "How amber wears in cold air"; "Scents for the long rains" | `:238-239` |
| Release | v31; stock as of 11:02 | `:286` |
| Images | Four generated renders, labelled "Concept render"; in the sample they must be labelled "Sample image" or replaced with drawn placeholders (`docs/06-website-experience.md:1122`) | `concept/experience/img/SOURCE.md` |
| Seed plan | Achieng's bridesmaids' plan `plan-…`: Wambui (citrus or rain), Njeri (vanilla or amber), Atieno (amber, swapped to rain), Muthoni (vanilla or citrus), Zawadi (citrus or vanilla); budget KSh 25,000; KSh 25,200 before the swap, 24,600 after; deliver by Fri 4 Dec; wedding 12 Dec | `concept/experience/artifact.html:195-204`, `:317` |

Gap for the seed: four products cannot show catalogue-scale search. Seed at least 24 products
across 5 families and 3 sizes so fit badges, refinements and plain results mean something.

### 1.6 Sample C: Tana Valley Times (with The Kestrel Post's patterns)

**Brand.** Fictional local weekly. Tokens: bg `#f4f5f7`, ink `#14213d`, ink-2 `#56607a`, line
`#dfe3ea`; display Libre Caslon Text; mark family `strata`, squareness 7.2
(`concept/identity/marks.html:495-499`).

**Archetype and tier.** News publisher. The sample targets T2: desks that answer only from
published, bylined, dated stories; catch me up from device memory; story timelines with
corrections; a digest with double opt-in; subscriber accounts.

**Front of house.** Newsroom (answers from published stories, with links), Archive (past
issues by date and topic), Subscriptions (sign-ups and renewals) from
`concept/identity/marks.html:501-503`, plus section links that open section fronts. The
section-desk peek pattern is news.html's Transport desk
(`concept/experience/news.html:207-215`).

**House rules** (shipping copy, `concept/experience/news.html:158-159`): desks never write
news; every claim links to a dated, bylined story; subscriber stories show headline and link
only to non-subscribers; corrections reach answers and digests within minutes; editors approve
each desk's scope and wording.

**What the owner (the editor) administers.** Stories (publish happens in the CMS or the Editor
WebAgent's drafts, never by a desk), corrections with visible notes, sections, story timelines,
subscriber list and entitlements, digest rules and the send ("Ask me first" at start,
`docs/06-website-experience.md:791`), desk scopes and wording, teach your site.

**Seed data.** About 60 stories over three years of weekly issues across 4 or 5 sections; 3 or
4 reporters; one multi-part story with a published correction (port the rail story:
`concept/experience/news.html:128-150`); at least 5 subscriber-only stories; one explainer;
issue dates for Archive; two seed readers (one subscriber, one not) with device-memory items
"Last visit" and "Follows"; one confirmed digest subscription.

**Open decision.** Paid subscriptions need recurring billing, which no release plans yet. For the
sample, make "subscriber" an entitlement granted by the owner or by a one-time sandbox payment;
recurring billing waits for the pilot (§8, D4).

### 1.7 Stretch sample: zavora.ai

Long single page with sections hero, the Move, what your clients get, trust, agencies, open
source, start (`concept/experience/presentation.html:127-192`); a Tours desk with two approved
scripts, GENERIC and AGENCY (`:266-279`); "Assess my site free" leads to the platform's start
page (built: `crates/agentrix-console/templates/start.html`); "Book a 20-minute call" needs a
booking capability that is not drawn. Owner admin: presentation scripts (chapters, must-say,
must-not-say, sources, length), leads and briefs, bookings.

---

## 2. Page-by-page inventory

### 2.1 Master index

| ID | Route | Screen | Persona | Drawn in | Built | W |
|---|---|---|---|---|---|---|
| K-01 | `/` | Kilele home | G C A | nav-agents, site, customer-browse | Partial | W2 W1 W7 |
| K-02 | `/bread` | Today's bread and stock | G C | nav-agents `#bakes`, site `#bakes` | Drawn | W2 W8 W1 |
| K-03 | `/cakes` | Celebration cakes and price list | G C | nav-agents `#cakes` | Drawn | W2 W8 |
| K-04 | `/cakes/studio` | Cake studio (creator) | G C | creator; customer-order custom tab | Drawn | W8 W1 W7 W2 |
| K-05 | `/shop/weekend-bake-box` | Product page | G C A | Implied (cited "Shop: weekend bake box") | Implied | W2 W8 |
| K-06 | `/checkout` | Basket, slot, quote | C (G if guest checkout) | customer-order catalogue tab | Drawn | W8 W2 W6 |
| K-07 | `/orders` | Your orders (guest lookup, signed-in list) | G C | nav-agents `#orders`; customer-pay K-1043 tab | Drawn | W2 W8 W6 |
| K-08 | `/orders/{id}` | Order page: status, payment request, thread | C | customer-after status; customer-pay; customer-agent; customer-security | Drawn | W8 W1 W6 |
| K-09 | `/orders/{id}/verify` | Prove it's your order (one-time code) | G | Implied (nav-agents "Confirm it's me") | Implied | W6 W9 |
| K-10 | `/orders/{id}/pay` | Pay (PayPal, M-Pesa, card) | C | customer-pay | Drawn | W6 |
| K-11 | `/orders/{id}/receipt` | Receipt and verify | C (anyone with the receipt) | customer-after receipt | Drawn | W6 W7 |
| K-12 | `/orders/{id}/change` | Ask to change the date | C | customer-after change | Drawn | W8 W1 |
| K-13 | `/orders/{id}/help` | Something's wrong (refund) | C | customer-after wrong | Drawn | W1 W6 W8 |
| K-14 | `/visit` | Hours, find us, delivery | G C A | nav-agents `#visit`, site `#visit` | Drawn | W2 |
| K-15 | `/delivery` | Delivery zones and fees | G C A | Implied (cited "Delivery: zones and fees") | Implied | W2 W8 |
| K-16 | `/allergens` | Kitchen allergen note | G C A | Implied (cited "Allergens: kitchen note") | Implied | W2 |
| K-17 | `/help/*` | What desks can and can't do; how Kilele contacts you; changing your details | G C A | Implied (cited in customer-security) | Implied | W2 W7 |
| K-18 | `/order-a-cake` | Simple cake request form (no-JS fallback) | G C | site `#order` | Drawn | W2 W8 |
| K-19 | `/about`, `/journal/*`, `/classes`, `/wholesale`, `/questions`, `/contact` | Migrated pages | G C A | fixture `seed.php` | Built (generic theme) | W4 W2 |
| A-01 | `/sign-in` | Sign in to order (why, passkey first, options) | G | customer-account 1 | Drawn | W6 |
| A-02 | `/sign-in#keeps` | What the site keeps | G C | customer-account 2 | Drawn | W6 W7 |
| A-03 | `/sign-in` (passkey) | Create account with a passkey | G | customer-account 3a/3b | Drawn | W6 |
| A-04 | `/sign-in` (password) | Email and password, or social provider | G | customer-account 3a (tab 2) | Drawn | W6 |
| A-05 | `/sign-in/authenticator` | Add an authenticator app | G | customer-account 3b (tab 2) | Drawn | W6 |
| A-06 | `/sign-in/recovery-codes` | Save recovery codes, once | C | customer-account 3c | Drawn | W6 |
| A-07 | (interstitial) | One passkey offer per device | C | customer-account 3d | Drawn | W6 |
| A-08 | `/account/choices` | Optional choices and consent receipts | C | customer-account 4, 5 | Drawn | W6 W9 |
| A-09 | `/account` | Account home, new sign-in alert, devices | C | customer-security takeover 2 | Drawn (partly) | W6 |
| A-10 | `/account/details` | Change details (fresh sign-in) | C | customer-security takeover 3 | Drawn (gate only) | W6 |
| A-11 | `/account/data` | Download my data | C | customer-security erasure 1 | Drawn | W6 |
| A-12 | `/account/data` (delete) | Delete account | C | customer-security erasure 2 | Drawn | W6 |
| A-13 | `/account/data/erasure/{id}` | Erasure receipt | C (then by email) | customer-security erasure 3 | Drawn | W6 |
| A-14 | `/sign-in` (returning) | Returning sign-in, both paths | G | New | New | W6 |
| A-15 | `/sign-in/verify-email` | Verify email before ordering | G | Implied (`docs/06-website-experience.md:692-693`) | Implied | W6 W9 |
| A-16 | `/sign-in/recover` | Lost device: use a recovery code | G | Implied (06 §4.10) | Implied | W6 |
| A-17 | `/sign-in/reset` | Forgot password | G | New (and conflicts with "no magic links") | New | W6 |
| A-18 | `/sign-out` | Sign out; signed out | C | New | New | W6 |
| A-19 | `/account/sign-in` | Manage passkeys, authenticator, recovery codes | C | Implied ("Account → Sign-in", customer-account.html script) | Implied | W6 |
| A-20 | `/account/addresses` | Delivery addresses and recipients | C | New | New | W6 W8 |
| A-21 | `/account/memory` | Account memory items, delete one by one | C | Implied (06 R12) | Implied | W1 W6 |
| A-22 | (state) | Session expired; sign in again | C | New | New | W6 |
| C-01 | `/` | Classy home | G C A | New | New | W2 W7 |
| C-02 | `/search?q=` | Agentic search answer | G C | search | Drawn | W1 W8 W2 |
| C-03 | `/search?q=&mode=plain` | Plain results | G C | search (mode toggle) | Drawn | W1 W2 |
| C-04 | `/fragrances/{slug}` | Product page | G C A | Implied (cards cite products) | Implied | W2 W8 |
| C-05 | `/search` (compare) | Comparison table | G C | search `compare()` | Drawn | W8 W1 |
| C-06 | `/gifts/plan` | Gift plan maker | C | artifact (Achieng) | Drawn | W1 W8 W2 |
| C-07 | `/bag` | Bag | G C | New ("Add to bag" toasts only) | New | W8 W2 |
| C-08 | `/checkout` | Checkout (K-06's template) | C | New for Classy | New | W8 W6 |
| C-09 | `/delivery`, `/gifts/wrapping`, `/journal/*` | Content pages | G C A | search page hits | Drawn (as hits) | W2 |
| C-10 | (step) | Write the gift card | C | Implied ("Help me write the card") | Implied | W1 |
| R-01 | `/a/{id}` | Artifact, creator's view | C (G for a brief) | creator share; artifact (Achieng) | Drawn | W1 W7 |
| R-02 | `/a/{token}` | Artifact, invited person's view | I | artifact (Wambui) | Drawn | W1 W7 |
| R-03 | (sheet) | Share settings | C | creator, artifact share sheets | Drawn | W1 W7 |
| R-04 | `/a/{id}` (states) | Expired, revoked, no permission, changed since | I C | New | New | W1 W7 |
| N-01 | `/` | Front page with desks and catch me up | G C | news | Drawn | W2 W1 |
| N-02 | `/{section}` | Section front | G C A | Implied ("Click Transport for the section front") | Implied | W2 |
| N-03 | `/{yyyy}/{mm}/{slug}` | Story page with byline, date, correction note | G C A | New | New | W2 |
| N-04 | `/stories/{topic}` | Story so far (timeline) | G C | news (box) | Drawn | W2 W1 |
| N-05 | `/digest` | Digest sign-up with contract | G C | news (box) | Drawn | W9 W1 |
| N-06 | `/digest/confirm` | Double opt-in confirmed | G | Implied ("Check your email to confirm") | Implied | W9 |
| N-07 | `/digest/stop` | Stop the digest, topics deleted | G | Implied (contract "Stop") | Implied | W9 |
| N-08 | `/subscribe` | Subscribe, subscriber sign-in, locked story | G C | Implied (subscriber lock) | Implied | W6 W2 |
| N-09 | `/corrections` | Corrections log | G A | Implied (house rules) | Implied | W2 |
| N-10 | `/archive` | Archive by date and topic | G C A | Implied (Archive desk) | Implied | W2 W1 |
| Z-01 | `/` | zavora.ai page | G A | presentation | Drawn | W2 |
| Z-02 | `/` (tour on) | Tour stage | G | presentation stage bar | Drawn | W1 W7 |
| Z-03 | platform `/` | Assess my site | G | Built on the platform host | Built | W4 |
| Z-04 | `/book` | Book a 20-minute call | G | Implied | Implied | W1 W8 |
| M-01 | `/.well-known/awp` | AWP descriptor | A | site (as `awp.json`) | Built | W1 |
| M-02 | `/mcp` | MCP tools | A | site, customer-agent | Built (2 tools) | W1 |
| M-03 | `/awp/v2/calls/{id}` | Held call status | A | customer-agent | Built | W1 |
| M-04 | `/.well-known/ai-catalog.json` | AI Catalog | A | customer-agent | Drawn | W1 |
| M-05 | `/.well-known/agent-card.json`, `/a2a` | Signed A2A card and endpoint | A | customer-agent, site | Drawn | W1 |
| M-06 | `/.well-known/ucp` | UCP profile | A | Implied (06 §12.4) | Implied | W1 W8 |
| M-07 | any page with `Accept: text/markdown`, `/x.md` | Markdown twins | A | site (agent view) | Built | W1 |
| M-08 | `/robots.txt`, `/llms.txt` | Consent signals, pointer | A | Implied (06 §12.2-12.3) | Implied | W1 W3 |
| M-09 | `/agents` | Agent door page and agent terms | A (and people) | customer-agent door panel | Drawn | W1 W7 |
| M-10 | `/.well-known/agentrix/keys` | Receipt keys | A, anyone | customer-after note | Built (platform host) | W6 |
| M-11 | `/receipts/{digest}` | Receipt page | anyone | customer-after | Built (platform host, console chrome) | W6 |
| M-12 | `POST /ask`, `POST /ask/pass` | Ask SSE and pass to owner | G C | nav-agents, site | Built | W1 |
| M-13 | (demo) | "What an AI agent sees" | O, judges | site agent view | Drawn | W2 |
| X-01 | any unknown path | Not found, in the site's brand | G C A | New | Partial (plain text) | W2 W1 |
| X-02 | any | Site unavailable, platform hold, takedown | G C A | Words only (06 §4.3) | Partial (plain text) | W3 W7 |
| X-03 | any | Answers paused (resting) | G C | Words only (06 §4.3) | Partial | W1 W7 |
| X-04 | `/privacy` | Privacy notice and visit counts | G C | Words only (06 §4.9) | Implied | W6 W7 W2 |
| X-05 | footer, `/report` | Report a look-alike | G C | customer-security look-alike 3 | Drawn | W3 W6 |
| X-06 | `/terms`, `/refunds` | Terms of sale, refunds and returns | G C | New | New | W2 W6 |
| X-07 | `/accessibility` | Accessibility statement | G C | New | New | W7 |
| X-08 | (state) | No JavaScript or offline | G C | Principle only (06 §4.7) | New | W2 W7 |

Counts: 90 entries. 77 are screens people see (K 19, A 22, C 10, R 4, N 10, Z 4, X 8): 39
drawn, 6 built or partly built (K-01, K-19, Z-03, X-01, X-02, X-03), 32 implied or new. 13 are
machine surfaces: 7 built (M-01, M-02, M-03, M-07, M-10, M-11, M-12), 4 drawn, 2 implied.

### 2.2 The shell on every site page

These parts appear on every page of every sample. They are components of the kit (§5), listed
here because their states apply page by page.

| ID | Part | Drawn in | What it holds | States | Built |
|---|---|---|---|---|---|
| S-1 | **Header with desks.** Logo, menu items that are desks (mark, label, AI chip in the peek), one primary call to action, account chip | `concept/experience/nav-agents.html:140-145`, `:376-392`; `concept/journeys/customer-browse.html:55-60` | Label is a link to the static page. Hover (120 ms intent), focus or long-press opens the peek: mark, name, "WebAgent · answers from this site", AI chip, mandate, live facts with as-of, 3–4 starters (some "Instant"), input, foot ("Click Cakes to open its page", "Esc to close") | Closed; open; pinned (typing pins it; "Pinned · Esc to close"); answering inside the peek; handed over ("Cakes handed you to Orders, with the conversation so far"); resting (grey mark; "Answers are paused right now. The page and the form below still work."); waiting (amber dot); no JS (plain links) | No. Theme nav is plain links: `crates/agentrix-console/templates/site/page.html:34-36` |
| S-2 | **Phone menu: the team sheet.** "Menu" opens a sheet with each desk: mark, name, mandate, "Ask {desk}", "Open page" | `concept/experience/nav-agents.html:232-235`, `:483-492` | Desk manifest | Open; closed; Esc closes and returns focus | No |
| S-3 | **Ask bar.** One line in the page layout, `/` focuses it, AI chip, hint line ("Answers cite this site", "Wanjiru is one tap away") | `concept/experience/nav-agents.html:203-213`; `concept/experience/experience.css:172-177` | Front desk routes the question | Idle; typing; sending; no JS (search form to plain results) | Partial: a corner panel, `crates/agentrix-console/static/site/ask.js:21-69`, `ask.css:11` (A13) |
| S-4 | **Thread rail (desktop) or sheet (phone).** Desk head with mark and name; the thread (`role="log"`); memory chip ("This chat is forgotten when you close the tab." + "Remember me here"); input with send and a disabled "Voice (later)"; disclosure line; person chip ("W Wanjiru · person · WhatsApp, ~2 h") | `concept/experience/nav-agents.html:216-229`, `:446-480`; `concept/experience/experience.css:179-192` | Thread for this session; desk that is speaking; who is the person and their honest wait time | Empty; streaming (phrases, no autoscroll); cited answer with numbered sources that point at the page; action buttons; "Was this right? Yes · No"; handover between desks; handed to a person; person joined ("Wanjiru from Kilele joined · a person", `docs/06-website-experience.md:459-464`); person left, desk back; waiting on owner; resting; error | Partial: the corner panel streams cited answers and the pass-to-owner form (`ask.js:104-264`). No desks, actions, feedback, handover, memory or person states |
| S-5 | **Footer.** "The WebAgents on this site are AI. They never ask for a password, a code or a card." A person one tap away ("Talk to Wanjiru"). Visit counts notice with "Don't count my visits". Report a look-alike. Made with Agentrix. Release signed | `concept/journeys/customer-browse.html:86-93`; `concept/journeys/customer-security.html:280-293`; `concept/experience/nav-agents.html:200` | Release seq, privacy words (06 §4.9), contact href | Counting on; opted out ("Your visits on this device aren't counted. Count them again"); Global Privacy Control; counting off | Partial: footer disclosure and a counting line exist (`page.html:44-48`) but counting is off (`crates/agentrix-move/src/pipeline.rs:822`) and there is no beacon, opt-out or look-alike form |
| S-6 | **Mark and presence.** Living mark per desk (squircle, signature, glyph, state) with an AI chip beside it | `concept/journeys/journeys.js:151-207`; `concept/identity/marks.html` | `MarkSpec` from the signed release | idle, listening, working (with a real tool label), answering, speaking (later), waiting, resting; settles after 5 s; reduced motion | No. Code draws a plain square (`ask.css:27-33`) |
| S-7 | **Sample band.** "Sample site. People, products and orders are fictional." | `page.html:31` (fixture variant) | Site flag | On for samples | Partial (fixture flag) |

### 2.3 Kilele Bakery pages

**K-01 · `/` · Home** · G C A · Partial · W2 W1 W7

- **Drawn:** `concept/experience/nav-agents.html:140-200`; older version `concept/site.html:180-276`; guest view `concept/journeys/customer-browse.html:50-81`.
- **Purpose:** show the bakery as itself, introduce the desks, answer the first question.
- **Components:** S-1 header (Bread, Cakes, Orders, Visit; "Design a cake"); hero (headline, one line, "Design a celebration cake", "Today's bread"); team line with marks: "Each menu item is an AI WebAgent that answers from this site. Wanjiru approves anything that costs money." (`nav-agents.html:152`); a today's-bread teaser; S-3; S-4; S-5; S-7.
- **Data:** page copy and desk manifest (signed release); live facts (stock, next free date, open now) from capabilities through the edge cache with an as-of time (`docs/06-website-experience.md:566`).
- **Actions:** hover or focus a desk → peek, no model (R3); click a label → K-02, K-03, K-07, K-14; instant starter → capability answer, footer "Answered from today's stock, 09:42"; other starter or typed question → cited answer; citation → the page scrolls and the cited element lights up with "Cited [1]" (`concept/experience/experience.js:27-41`); `/` → S-3.
- **States:** guest; signed in (account chip "N Njeri", `concept/journeys/customer-account.html:175`); no JS; resting; platform hold (X-02); language switch EN · SW shown in `concept/site.html:185` (not designed beyond the pill; G-33).
- **Code:** generic theme `crates/agentrix-console/templates/site/page.html:9-50`, served byte for byte by `crates/agentrix-node/src/site/mod.rs:185-264`, with the corner Ask panel.

**K-02 · `/bread` · Today's bread** · G C · Drawn · W2 W8 W1

- **Drawn:** `concept/experience/nav-agents.html:157-166`; `concept/site.html:215-224`.
- **Components:** product cards (image or drawn placeholder, name, price, stock badge, ingredients line); "Stock updates as the morning goes · as of 09:42"; Bread desk peek.
- **Data:** catalogue and prices (release); stock and next batch (live `stock` capability; display only, orders re-check, `docs/06-website-experience.md:1141`); ingredients (owner-confirmed facts).
- **Actions:** "Can you hold 2 loaves for me?" → `hold.create` inside the standing grant → line "Held: 2 sourdough loaves until 12:00 · H-0193 …" (`nav-agents.html:311-313`), owner sees it in the brief; "Anything without dairy?" → cited answer. **Not drawn:** "Add to basket" on a card, though K-06 shows loaves in a basket (G-15).
- **States:** in stock with count; next batch at a time; sold out "back at 6:30"; stock source unavailable (show no count and say so; New); hold limit reached (above 3 loaves the request waits for Wanjiru; New).

**K-03 · `/cakes` · Celebration cakes** · G C · Drawn · W2 W8

- **Drawn:** `concept/experience/nav-agents.html:168-183`.
- **Components:** price table with stable row anchors (`#row-1tier`, `#row-2tier`, `#row-3tier`) so citations can point; notes for gluten-free and changes (`#note-gf`, `#note-cancel`); link to the studio; Cakes peek.
- **Data:** the configurable product's base prices and constraints (release, `Known` only after the owner confirms).
- **Actions:** "How much for 30 people?" → answer citing `#row-2tier` and `#row-3tier`, action "Design it with me" (`nav-agents.html:316-319`); "Is anything gluten-free?" → answer citing `#note-gf`, action "Ask Wanjiru about allergens" → handover to a person.
- **States:** next free date shown live; price list superseded since a thread cited it ("changed since", `docs/06-website-experience.md:522`).

**K-04 · `/cakes/studio` · Cake studio** · G C · Drawn · W8 W1 W7 W2

- **Drawn:** `concept/experience/creator.html:116-214`, script `:228-457`; journey view `concept/journeys/customer-order.html:170-260`.
- **Purpose:** turn the customer's words into a cake Kilele makes, priced by rules, then a held request.
- **Components:** breadcrumb "Cakes › Cake studio" and "AI WebAgent · Wanjiru confirms"; talk panel (desk head, one-question thread, option chips with price deltas and colour swatches, free-text input, disclosure); "New here? Watch a 40-second example" (concept only; drop in production); sketch stage with "Sketch, not a photo · Wanjiru bakes the real one" (`creator.html:148`); cake title; spec list ("Not chosen yet" per unset row); flags (warnings and notes); price card with each line's rule id, total, "price list v14 · valid until Sat 14 Nov"; rules note; "Request this cake" (disabled until complete and valid); "Save & share the design"; the request summary with note field ("No nuts"), delivery joined to an existing order, price in KSh and USD (`customer-order.html:217-247`); "What happens next" list.
- **Data:** `ConfigurableProduct` (options, constraints, lead times, allergens, price deltas, sketch parts) from the release; calendar capacity per date and size; price rules; posted rate; the customer's open orders (to join a delivery).
- **Actions:**
  - pick an option → rules recompute total, flags and sketch in under 100 ms, no model;
  - free text → model maps words to approved options and says what isn't offered ("Kilele doesn't make sculpted figures…", `creator.html:422`);
  - "Save & share" → `artifact.create` (kind `brief`) → share sheet R-03 → `/a/{id}`;
  - "Request this cake" → sign-in if needed (A-01, return here with the thread) → `request_custom_order` with the artifact digest and the pinned quote → held call → **owner decision card** (`meet-order`, `concept/journeys/index.html:129-135`) → presence "waiting", status line "Waiting for Wanjiru's OK. Nothing charged.", pill "K-1043 · held for Wanjiru".
- **States:** empty ("Pick a size to see the price"); invalid combination (gluten-free with 3 tiers; Saturday full for 3 tiers, date chip disabled with a reason); complete; sending; held; **approved** (payment request appears on K-08); **declined**, **owner suggests a change**, **lapsed** (card expired: "the request lapsed, nothing was charged, and how to ask again", `docs/06-website-experience.md:1024-1025`): none drawn (G-08); model resting (chips still work, free text off with words); quote expired.
- **Code:** none. The door's `request_custom_order` takes only description, date, name and email (`crates/agentrix-node/src/door/mod.rs:104-116`), so it cannot carry the typed design or a quote yet.

**K-05 · `/shop/weekend-bake-box` · Product page** · G C A · Implied · W2 W8

- **Cited:** "Shop: weekend bake box" (`concept/journeys/customer-browse.html:112`; `concept/journeys/customer-order.html:156`).
- **Needs:** name, contents, price, which days it is baked, availability for a chosen date, "Add to basket", the Bread or Orders desk peek, Schema.org Product in the Markdown twin.

**K-06 · `/checkout` · Basket and checkout** · C (G if the owner allows guest checkout) · Drawn · W8 W2 W6

- **Drawn:** `concept/journeys/customer-order.html:96-140`; Orders desk in checkout `:144-166`.
- **Components:** basket lines, each with its sources ("Price list, 2 Nov", "Stock: Sat 14 Nov bake (live)", "Available Sat 14 Nov"); delivery line; delivery-slot radio group from the live calendar with a full slot disabled and labelled; total; pinned-quote line ("Quote pinned to Kilele's release of 2 Nov (price list v14). Valid until Sat 14 Nov 08:00."); pay note ("Pay USD 19.77 at Kilele's rate…"); "Continue to pay"; rails note; Orders desk with the scope line "Orders sees this basket's items, slot and area. It can't see your account or payment details." (`:150`).
- **Data:** price list, stock per bake day, delivery calendar, zones and fees, posted rate; the quote (`quote.create`, pinned with `release_seq` and `valid_until`).
- **Actions:** change slot → re-quote; "Continue to pay" → create order K-1042 with its quote → K-10 (the fleet release); ask Orders "Can you make the bake box gluten-free?" → "No…", cites the menu, offers what exists.
- **Not drawn:** adding and removing items, quantities, the empty basket, address and recipient entry ("Amina's home" appears with no form), guest-or-sign-in choice, stock gone at submit, slot taken at submit, quote expired (G-15 to G-19).

**K-07 · `/orders` · Your orders** · G C · Drawn · W2 W8 W6

- **Drawn:** guest form "Order number, e.g. KB-1042" + "Track" (`concept/experience/nav-agents.html:185-189`); signed-in list (`concept/journeys/customer-pay.html:160-206`).
- **Guest:** shows status only; details need a one-time code to the number on the order ("For your privacy I've shown only the status. Confirm the WhatsApp number on the order to see the details.", `nav-agents.html:323`) → K-09.
- **Signed in:** one card per order: name, status pill, the desk's status line with its mark ("Waiting for Wanjiru's OK. Nothing charged."), "No payment request yet. It appears here after Wanjiru approves."
- **States:** no orders (empty state, New); held; approved and unpaid; paid; in progress; delivered; cancelled; refunded; lapsed.

**K-08 · `/orders/{id}` · Order page** · C · Drawn · W8 W1 W6

- **Drawn:** status `concept/journeys/customer-after.html:154-200`; payment request after approval `concept/journeys/customer-pay.html:182-206`; prepared by an assistant `concept/journeys/customer-agent.html:216-246`; payment-link check `concept/journeys/customer-security.html:196-232`.
- **Components:** order head with id and pill; summary line; status timeline (Requested, Approved by Wanjiru, Paid, Baking, Out for delivery, Delivered, each with when); Orders thread with the same updates; "Where updates arrive" (this page always; WhatsApp only if chosen; the thread); payment request card after approval (Cashier line "made this payment request from the approved order. It can't change the amount."; amount, payee, order; Pay buttons; "Payment requests appear only here, on your orders page…"); pill "Prepared by your assistant" with the line "Prepared through the agent door by assistant.example, a known agent. Nothing is charged until you pay."
- **Data:** order record and events (W8); payment request (Effects Gateway, W6); receipts.
- **Actions:** Pay → K-10; Receipt → K-11; Change → K-12; Something's wrong → K-13; ask Orders "Is this link real?" → `payment_request.check` compares the link with the order's own request and cites it; "Report it" (X-05 path, to the owner).
- **States:** every order state above. **Gap:** who moves "Baking", "Out for delivery" and "Delivered" is not designed (owner tap, a WebAgent, or a courier integration; G-41).

**K-09 · `/orders/{id}/verify` · Prove it's your order** · G · Implied · W6 W9

- From 06 §5.6: "details only after the visitor proves it's them (a one-time code to the number on the order)" (`docs/06-website-experience.md:883`). Proving one order is not signing in (`:706-707`).
- **Needs:** code sent by WhatsApp or SMS to the number on the order, a code field, rate limits, wrong-code and expired-code states, "Continue on this device only".

**K-10 · `/orders/{id}/pay` · Pay** · C · Drawn · W6 (the fleet release)

- **Drawn:** `concept/journeys/customer-pay.html:90-150`, after approval `:182-230`, other rails `:232-251`, flow `:253-265`.
- **Components:** "Pay for K-1042", pill "Not paid yet"; amount in USD with KSh and the rate; payee; order summary; source line; "Pay with PayPal"; "M-Pesa"; "Card"; small print about dollars; PayPal's window as a labelled placeholder; return screen with "Confirmed", "Paid · USD 19.77 · PayPal" and a receipt preview.
- **Effects:** PayPal: the Effects Gateway creates an Orders v2 order for the exact amount, payee Kilele, under the fingerprint `H(site, webagent, tool, canonical_args, amount, expiry)`, captures once on approval, writes a signed receipt (`:257-262`). M-Pesa: STK Push to a Kenyan number, KES, PIN on the buyer's phone. Card: provider-hosted fields.
- **States drawn:** not paid; PayPal open; paid. **Not drawn:** cancelled at PayPal, capture failed, M-Pesa waiting for PIN, M-Pesa timeout, insufficient funds, wrong number, payment request voided after a change, double submit (G-20 to G-23).

**K-11 · `/orders/{id}/receipt` · Receipt** · C, anyone with the link · Drawn · W6 W7

- **Drawn:** `concept/journeys/customer-after.html:104-150`.
- **Components:** lines, total, charged amount and method, rate, payee and domain, paid at (with the customer's time zone), receipt id; "Verify this receipt" → "Signed by the Effects Gateway for kilele-bakery.example · valid"; note that PayPal also emails its own receipt; the CLI check `agentrix audit verify --receipt` for third parties.
- **Code:** a receipt page and an in-browser check exist on the platform host with console chrome (`crates/agentrix-node/src/pages/mod.rs:52`, `crates/agentrix-console/templates/console/receipt.html`, `crates/agentrix-console/static/verify.js`). They need a site-branded version on the site's domain (G-28).

**K-12 · `/orders/{id}/change` · Ask to change the date** · C · Drawn · W8 W1

- **Drawn:** `concept/journeys/customer-after.html:204-243`.
- **Before the cut-off:** current slot, the cut-off time with its source, a select of open slots from the live calendar, "Ask to move it" → `order.change_request` held for Wanjiru unless a standing rule covers it → card; "Waiting for Wanjiru's OK" with "Withdraw the request".
- **After the cut-off:** "Changes closed Thu 12 Nov, 18:00. Amina's cake is being made." and "Talk to Wanjiru" (a person; a desk can't move an order after the cut-off).
- **Not drawn:** approved change (the payment request is voided and reissued if the price changes), declined change.

**K-13 · `/orders/{id}/help` · Something's wrong** · C · Drawn · W1 W6 W8 (refunds the fleet release)

- **Drawn:** `concept/journeys/customer-after.html:248-310`.
- **Components:** Orders thread; message; photo upload (drawn as a placeholder); handover line "The Cashier works behind the scenes. It sees the order and the request, not your account."; refund offer with amount in KSh and USD at the pinned rate; "Refund the bake box"; the rule check panel (amount, recipient, frequency, expiry, signature and cap); result "Refunded USD 10.47 to your PayPal. Receipt R-1042-1."; above the rule: "That's above what can be refunded without Wanjiru. She will decide; you'll hear by Sun 15 Nov, 20:00. Nothing has been refunded yet." → **owner card**.
- **Needs:** photo intake treated as hostile input (size limits, type sniffing, stripped metadata, stored sealed, shown to the owner only); refund declined state; cap reached state (even a small refund waits).

**K-14 · `/visit` · Hours, find us, delivery** · G C A · Drawn · W2

- **Drawn:** `concept/experience/nav-agents.html:191-198`.
- **Components:** three blocks with anchors `#visit-hours`, `#visit-where`, `#visit-delivery`; Visit desk peek with two instant starters.
- **Actions:** "Do you deliver to Lavington?" → "The site lists Westlands and Kilimani only, so I won't promise Lavington." → "Yes, ask Wanjiru" → handover, and the gap lands in teach your site (`nav-agents.html:335-337`).

**K-15 · `/delivery`**, **K-16 · `/allergens`**, **K-17 · `/help/*`** · Implied · W2 W7

- Cited as sources but never drawn: "Delivery: zones and fees", "Allergens: kitchen note", "Help: what the desks can and can't do", "Help: how Kilele contacts you", "Help: changing your details" (`concept/journeys/customer-security.html:163`, `:226`). The security story depends on K-17 being real pages a desk can cite.

**K-18 · `/order-a-cake` · Simple cake request** · G C · Drawn · W2 W8

- **Drawn:** `concept/site.html:226-261`.
- **Purpose:** the form-only path that still works with JavaScript off or the model resting, and the order path at T0 and T1 before the studio is switched on.
- **Components:** cake, pickup or delivery, message, date, name, WhatsApp number, "Request this cake", "Nothing is charged until Wanjiru confirms."; held card with a four-step tracker (Requested, Wanjiru confirms, Pay with M-Pesa, Collect on Saturday); "Why does a person confirm?" note.
- **Action:** posts to the same held capability as K-04, without the sketch.

**K-19 · Migrated pages** · G C A · Built (generic) · W4 W2

- About, Journal and its 40 posts, Classes, Wholesale, Questions, Contact from the fixture (`fixtures/wordpress-infected/site/seed.php:136-176`), rendered by the Move through `crates/agentrix-move/src/theme.rs:198` into `crates/agentrix-console/templates/site/page.html`. They need the S-1 to S-7 shell, anchors on headings (built for `:target` styling, `crates/agentrix-console/static/site/site.css:31`) and the sample's corrected facts.

### 2.4 Customer account pages (shared by every site)

These run on **the site's own domain**, never the platform host, because a passkey is bound to
the domain it was made for (`docs/06-website-experience.md:1011`). Owner sign-in exists on the
platform host (`crates/agentrix-node/src/auth/routes.rs:44-53`, passkey, password, TOTP,
sessions) and is the starting point, but it must run per site host with the site's relying-party
id and its own session cookie (W6). Desks see "customer", never the account
(`docs/06-website-experience.md:701-703`).

| ID | Screen | Drawn in | Components and words | Actions → effect | States |
|---|---|---|---|---|---|
| A-01 | Sign in to order | `concept/journeys/customer-account.html:96-119` | "Your basket, Amina's cake design and your thread with Cakes AI are waiting. They pick up again once you're signed in." "Sign in to order." "So you can follow your order and get a receipt. New here? The same buttons create an account." Buttons: Continue with a passkey; Use email and password; Continue with Google, LinkedIn, GitHub (owner picks which); "You can still ask and browse without an account. What Kilele keeps" | Passkey → A-03 or sign-in; password → A-04; provider → its own page | Arrived from a desk (return target and thread token, F2, `docs/06-website-experience.md:1183`); providers the owner turned off are hidden |
| A-02 | What the site keeps | `customer-account.html:122-139` | Six rows with why and how long (`docs/06-website-experience.md:991-1002`) and "never keeps a card number, a PayPal password or an M-Pesa PIN" | none | Same list again in the account at any time |
| A-03 | Create account with a passkey | `:150-185` | Name; email for receipts; phone for order updates (optional); "Create a passkey"; device prompt drawn as a placeholder; "Done: this device's screen lock signs you in to kilele-bakery.example only." | WebAuthn registration with rpId = site domain | Device refuses; passkey already exists; no platform authenticator |
| A-04 | Email and password, or a provider | `:187-207` | Name, email, password ("A long one you don't use anywhere else. Next, you'll add an authenticator app."); provider page placeholder ("asks it for her name and a verified email only") | Create account; email verification before ordering (A-15) | Weak or breached password; email in use |
| A-05 | Add an authenticator | `:209-224` | QR, typed key, 6-digit code, Verify, "No authenticator app? Create a passkey instead." | TOTP enrolment | Wrong code; clock skew |
| A-06 | Recovery codes | `:226-239` | Eight codes, "We show these once", Download, "I've saved them" | none | Must not be shown again |
| A-07 | One passkey offer | `:241-259` | "Next time, sign in with this device's screen lock" · Create a passkey · Not now → "Done. This device won't offer a passkey again." | Registration, or a per-device flag | Shown once per device |
| A-08 | Your choices and receipts | `:263-338` | Three switches, all off by default except what she picks: order updates on WhatsApp; remember me and my designs (account memory); offers by email. Each "on" shows a consent receipt (purpose, what, shared with, kept for, given, withdrawn, how to withdraw, signed id) | Each switch writes or withdraws a consent receipt (W6); WhatsApp updates need W9 | Active; withdrawn (receipt kept with both times) |
| A-09 | Account home | `concept/journeys/customer-security.html:123-145` | Alert "New sign-in on a Windows PC in Leeds. Not you?" · Sign out everywhere · It was me; "Signed in on" device list with "New" pill | Sign out everywhere ends every session, this one included | No alert; alert dismissed. **Not drawn:** the rest of the home (orders, details, choices, sign-in methods, data) |
| A-10 | Change details | `:147-170` | "Sign in again to change this. We'll tell njeri@example.com when it changes." · passkey · password and code; Orders desk refusing "Can you change my email for me?" with two citations | Fresh authentication, then change, then a notice to the old email (W9) | **Not drawn:** the details form itself |
| A-11 | Download my data | `:309-333` | What's in the bundle; "A page you can read, plus files a computer can read (JSON Lines and CSV). Signed by kilele-bakery.example." | Export job (ADR-028) | **Not drawn:** preparing, ready, expired link |
| A-12 | Delete account | `:334-350` | Erased list, kept list, "You have no open orders. This can't be undone."; "Delete, with my passkey"; "Keep my account" | Fresh authentication, then erasure across stores | **Not drawn:** open orders block deletion |
| A-13 | Erasure receipt | `:351-383` | Per-store rows with time and state (erased, kept reduced, pseudonymised, scheduled for backups); signed id; "A copy went to … before that address was erased." | none | In progress; complete |
| A-14 | Returning sign-in | New | Passkey first (conditional UI on the email field); password then code | Sign in, return to where she was | Wrong password; locked after attempts (rate-limit words, G-36) |
| A-15 | Verify your email | Implied | Code by email (06 bans magic links, `docs/06-website-experience.md:688-689`) | Unlock ordering | Code expired; resend |
| A-16 | Lost your device | Implied | Use a recovery code; then add a new passkey or authenticator | Recovery | No codes left: what then? (G-04) |
| A-17 | Forgot password | New | Must work without a magic link. Proposal: emailed code plus the authenticator code | Reset | Decision D6 |
| A-18 | Sign out | New | "Signed out. Your basket stays on this device." | End session | none |
| A-19 | Sign-in methods | Implied | Passkeys by device with remove; authenticator reset; new recovery codes | Each needs fresh authentication and a notice email | Last method can't be removed |
| A-20 | Addresses and recipients | New | Address, area (zone), recipient name and phone, delivery notes | Used by K-06 | Outside delivery zones |
| A-21 | Account memory | Implied | Items with "From your account", delete one at a time, export | `memory.forget`, `memory.export` | Memory off; never for minors (G-46) |
| A-22 | Session expired | New | "You were signed out after a while away. Sign in to carry on; your basket is still here." | Sign in | none |

### 2.5 Classy Perfumes pages

**C-01 · `/` · Home** · New · W2 W7. Not drawn. Needs: the dark header with logo, search-or-ask
box and desks (`concept/experience/search.html:153-166`); collections (Fragrances, Gifts);
featured products with real stock; delivery promise from the delivery page; S-5 footer.

**C-02 · `/search?q=` · Agentic search** · G C · Drawn · W1 W8 W2

- **Drawn:** `concept/experience/search.html:150-187`, scenarios `:256-330`.
- **Components:** search-or-ask input with "AI answers" chip; suggestions dropdown in two groups, "Products <50 ms · no AI" and "Ask: …" (`:371-381`); query title; mode toggle Answer / Plain results; answer card (desk mark and name, phrases, numbered sources, actions, basis line, "Was this right?"); shortlist cards (render with "Concept render" tag, rank badge "Best fit" or "Exact match", price and size, fit badges such as "Over budget by KSh 1,400", "Not vanilla", "Gift-wrapped", a one-line why, "Add to bag"); "From the site" page hits with highlighted terms; side panels "How I read your question" (editable intent chips), "Refine" (unsupported refinements greyed with "The site doesn't publish this, so search can't filter by it"), "What I used" (catalogue release, stock as-of, pages, model calls).
- **Data:** static index in the release (suggestions); catalogue and stock (rules); hybrid retrieval for requests (`docs/06-website-experience.md:767-773`).
- **Actions:** navigational query → product, no model; request → streamed cited answer; edit a chip → re-rank by rules, no model; "Add the gift set" → bag (C-07); "Compare these three" → C-05; "Help me write the card" → C-10; "WhatsApp me" / "Email me" on a gap → passes the question to the owner with a reply route (built as `/ask/pass` with a free-text contact, `crates/agentrix-node/src/site/pass.rs:24-31`).
- **States:** typing; no product names match (drawn in suggestions only, `:376`); answer; doesn't say (vegan scenario, `:301-315`); navigational; no results at all (New, G-31); model resting (plain results take over); stock stale.

**C-03 · Plain results** · Drawn (`concept/experience/search.html:345-351`). "{n} results · static index, no AI". Works with no model and no JavaScript (server-rendered form).

**C-04 · `/fragrances/{slug}` · Product page** · Implied · W2 W8. Cited by every card and answer. Needs: name, size, price, notes, family, stock with as-of, images labelled honestly, "Add to bag", gift wrap note, delivery estimate from rules, Fragrances desk peek, anchors for citations, Schema.org Product.

**C-05 · Comparison** · Drawn (`concept/experience/search.html:361-368`). Rows: price (with over-budget badge), size, family, notes, vanilla, gift-ready, in stock. Footer: "Built from the catalogue … without a model. Longevity isn't published, so it isn't compared." and "Save & share this comparison" → `artifact.create` (kind `comparison`).

**C-06 · `/gifts/plan` · Gift plan maker** · C · Drawn · W1 W8 W2

- **Drawn:** `concept/experience/artifact.html:123-131`, script `:212-320`.
- **Components:** left: Gifts desk thread; right: the artifact (R-01) building block by block, with a fresh-block marker; versions panel; next actions ("Share with them", "Request this order · KSh 24,600").
- **Actions:** words per person → plan rows with a pick and one alternative from the catalogue; over budget → the desk proposes a swap → "Apply the swap" (a new version); share → R-03; request → `request_order` with the artifact digest → **owner card** with typed fields only (`:287-293`).
- **States:** not started ("The plan appears here as you and the Gifts WebAgent make it."); building; draft; shared with "n of 5 chosen"; requested ("Order requested · waiting for the shop"); approved; declined; paid.

**C-07 · `/bag`** and **C-08 · `/checkout`** · New · W8 W6 W2. Not drawn for Classy. Reuse K-06's
basket and pay templates with Classy's tokens; add gift wrap and card message per item, delivery
by town with the courier fee, and the returns note.

**C-09 · Content pages** · Drawn as hits (`concept/experience/search.html:235-240`): Delivery &
returns, Gift wrapping, two journal posts. Build as pages with anchors.

**C-10 · Write the gift card** · Implied. A short step with the Gifts desk. The card text is
written by the customer and marked "Written by Achieng" (`docs/06-website-experience.md:545`).

### 2.6 Artifact pages (shared)

**R-01 · `/a/{id}` · Creator's view** · C · Drawn · W1 W7

- **Drawn:** `concept/experience/artifact.html:212-248`; the cake brief share in `concept/experience/creator.html:201-214`.
- **Components:** URL strip with lock, `noindex` chip and the viewer's permission ("Achieng · owner of this plan"); artifact head (kind "Plan · gifts", title, status pill); safe blocks (budget meter, people rows, delivery, a visitor-written note block "Written by Achieng · on every card"); foot ("Made by Achieng with Classy Perfumes' Gifts WebAgent (AI)", "Prices pinned to release v31 · held until 14 Nov", "Signed · r-4f0a…c21d", "Expires 7 Dec unless saved"); versions list; next actions.
- **Data:** `VisitorArtifact` store (sealed, signed, versioned; `docs/06-website-experience.md:808-825`).

**R-02 · `/a/{token}` · Invited person's view** · I · Drawn · W1 W7

- **Drawn:** Wambui's view, `concept/experience/artifact.html:263-278` and the artifact with prices hidden.
- **Components:** "Ask about this plan" desk scoped to her two scents; starter chips; her row with two choice buttons; other rows shown as "A surprise until she chooses"; prices "hidden"; "What you can do" ("Choose between your two scents and ask about them. You can't see prices or other people's picks until they choose, and you can't order."); disclosure "You opened Achieng's private link…".
- **Actions:** choose → a new version, the creator sees it; ask → scoped answer; "How much did she spend?" → "Achieng chose to keep prices private, so I can't share them."
- **Fix:** the drawn URL carries `?for=wambui` (A19). Use an opaque per-person token.

**R-03 · Share settings sheet** · Drawn (`concept/experience/creator.html:201-214`, `concept/experience/artifact.html:165-178`). Rows: anyone with the link can view (or named people only); they can comment; they can choose; hide prices; they can request or place the order (off by default); let the shop see this (needed to request; "Wanjiru sees the design, not this conversation"); expiry and price-hold note; "Copy link" or "Send five private links".

**R-04 · Link states** · New · W1 W7. Expired, revoked, not permitted, the shop can't see it,
facts changed since the quote ("changed since"), saved. Link previews show only the site name,
the desk's mark and a generic line (`docs/06-website-experience.md:822-825`).

### 2.7 News site pages (Tana Valley Times, news.html patterns)

**N-01 · `/` · Front page** · G C · Drawn (Kestrel) · W2 W1

- **Drawn:** `concept/experience/news.html:115-161`.
- **Components:** masthead with date and "Sample" band; section desks nav; Catch me up box (desk mark, "AI · from published stories only", device-memory chips "Last visit · Tue 3 Nov", "Follows · Transport", "Follows · Climate", "Forget this device"; buttons "Catch me up since Tuesday", "Just today's front page"); lead story card (kicker, headline, standfirst, byline, time, "Corrected 16:20"); more stories (one with a "Subscriber" lock); right rail: Story so far, Your digest, House rules for desks.
- **Actions:** Catch me up → answer listing items with correction flags and a subscriber source shown as headline and link only (`:254-269`); actions "Follow the rail story", "Get this every Friday"; "Forget this device" → items deleted, toast "Forgotten. Nothing about you was stored on our side."; "Remember my topics here" → stored in the browser after an explicit tap.
- **Section desk peek** (`:207-215`): starters "Where does the rail extension stand?", "Who has been reporting it?" (instant, from bylines), "Is the extension a good idea?" → "I don't give opinions. … hasn't published an editorial on it." (`:235`).

**N-02 · `/{section}`**, **N-03 story page**, **N-09 `/corrections`**, **N-10 `/archive`** · Implied or New · W2. Story pages need byline, date, section, the correction note at the top when corrected, a "Story so far" link, subscriber lock, anchors per paragraph for citations, NewsArticle in the twin.

**N-04 · Story so far** · Drawn as a box (`concept/experience/news.html:143-150`): dated steps linking to stories, a correction step styled apart, a scheduled next step.

**N-05 · `/digest` · Digest sign-up** · Drawn as a box (`:152-156`, contract `:274-283`). Topic toggles; the contract in four rows (You get, Made from, We keep, Stop); "Subscribe by email" and "Double opt-in"; toast "Check your email to confirm. Nothing is sent until you do."

**N-06 confirm**, **N-07 stop**, **N-08 subscribe and subscriber sign-in** · Implied · W9 W6. The stop page must delete the topics with the subscription, as the contract promises.

### 2.8 zavora.ai pages (stretch)

**Z-01 · `/`** · Drawn (`concept/experience/presentation.html:125-192`). Hero with "Give me the tour · 4 min" and "Assess my site free"; an ask box with two chips; sections with anchors used by stage cues; footer "Pages on this site are answered by AI WebAgents · Release v9 · signed".

**Z-02 · Tour stage** · Drawn (`:195-215`, behaviour `:282-345`). Stage bar with the Tours mark, "Script approved by Zavora · 4 Nov · claims limited to published pages", chapters as tabs, captions, mid-tour answer with sources and "Resume the tour", controls (previous, play or pause, next, ask, captions, "Voice · later", "Save as deck", "Exit"), progress; persona panel "Tailored to your question" with "From what you typed in this visit only. Nothing is stored." and "Change"; the page dims and the section on stage is lifted. Keyboard: Space, arrows, Esc.

**Z-04 · Book a call** · Implied. Needs a booking capability with real slots (06 §5.6 Bookings).

### 2.9 Machine surfaces (the agent door)

| ID | Surface | Drawn | Built today | What production needs |
|---|---|---|---|---|
| M-01 | `GET /.well-known/awp` | `concept/site.html:287-299` (as `awp.json`) | `crates/agentrix-node/src/door/mod.rs:129-154`: site, MCP url and versions, held-calls path, Markdown note, capabilities visible to `Public`, plain terms | Listed from the AI Catalog; generated from the signed desk manifest, not hand-written |
| M-02 | `POST /mcp` | `concept/site.html:316-323`; `concept/journeys/customer-agent.html:125-148` | `door/mod.rs:189-277`: initialize, ping, tools/list, tools/call; tools `ask_site` (cited answer, refusal policy) and `request_custom_order` (held, receipt `door.order-held`) | The catalogue tools (`catalog.search`, `delivery.slots`, `quote.create`, `checkout.create`), `order.status`, typed custom orders with a quote, tiers from Web Bot Auth, MCP 2026-07-28 stateless (A18) |
| M-03 | `GET /awp/v2/calls/{id}` | `concept/journeys/customer-agent.html:272-273` | `door/mod.rs:156-172` | A2A task state "working, waiting for the owner"; push later (U4) |
| M-04 | `/.well-known/ai-catalog.json` | `customer-agent.html:127-128` | none | One catalog listing MCP, the signed A2A card, UCP, AWP and the agent terms |
| M-05 | A2A card and endpoint | `customer-agent.html:128`, `concept/site.html:298` | none | Signed card with the mark as icon |
| M-06 | `/.well-known/ucp` | none | none | Catalog, cart, checkout and order verbs (pilot) |
| M-07 | Markdown twins | `concept/site.html` agent view | `crates/agentrix-node/src/site/mod.rs:209-218`, `Link` alternate `:252-259`, `x-awp-release` `:260-262` | Schema.org JSON-LD carried into twins |
| M-08 | `robots.txt`, `llms.txt`, `Content-Usage` | none | none (the Move writes neither) | Generated from the owner's consent choice and the release |
| M-09 | `/agents` door page and terms | `customer-agent.html:156-195` | none | Human page with the desk marks, what each tier may do, what waits for the owner; terms need legal review (`docs/06-website-experience.md:1269-1274`) |
| M-10 | `/.well-known/agentrix/keys` | `customer-after.html:141` | `crates/agentrix-node/src/app.rs:224` (platform host) | Also served on each site host, or linked from it |
| M-11 | `/receipts/{digest}` | `customer-after.html:137-149` | `crates/agentrix-node/src/pages/mod.rs:52` (platform host, console chrome) | Site-branded receipt page on the site host (G-28) |
| M-12 | `POST /ask`, `POST /ask/pass` | everywhere | `crates/agentrix-node/src/site/mod.rs:104-111`, `:335-438`; frames `crates/agentrix-node/src/site/ask.rs:695-872`; refusal before any model call `site/mod.rs:359-404`, `crates/agentrix-node/src/site/policy.rs`; uncited answers withdrawn `ask.rs:826-839`; pass to owner 5 an hour per IP `crates/agentrix-node/src/site/pass.rs:24-31` | Desk routing, handover, actions, `held`, thread token (F2), promise checker (U20), per-site rate limits with visitor words |
| M-13 | "What an AI agent sees" | `concept/site.html:279-343` | none | A demo view for owners and judges; not a production page |

### 2.10 System pages and global states

| ID | State | Visitor words (from the design) | Built |
|---|---|---|---|
| X-01 | Not found | New. Proposal: "That page isn't on this site. Try the menu, or ask: [Ask bar]". Branded, with the shell | Plain "Not found." unless the release has `404.html` (`crates/agentrix-node/src/site/mod.rs:266-278`); the Move writes none |
| X-02 | Platform hold; suspension in grace; takedown | "Answers and forms are paused right now. You can still read the site." with the release's contact line; or a neutral page "This site isn't available right now." with small print naming the operator chain (`docs/06-website-experience.md:466-474`) | Plain text 503 with the right sentence (`site/mod.rs:164`, `:195-199`); no page, no small print, no read-only mode |
| X-03 | Desks resting (budget, model down, rate-limited) | "Answers are paused right now. The page and the form below still work." Never the reason (`docs/06-website-experience.md:453`) | Partial: `ask.js:74`, `ask.rs:726-734`, `:766-775` use "Answers are paused right now. The page still works." |
| X-04 | Privacy notice | 06 §4.9 text, opt-out, Global Privacy Control line, counting off line (`docs/06-website-experience.md:639-650`) | Footer line only (`page.html:46`) |
| X-05 | Report a look-alike | Form with the address seen and what it asked for; "Send to Kilele and Agentrix" (`concept/journeys/customer-security.html:283-290`) | No |
| X-06 | Terms of sale, refunds, returns | New | No |
| X-07 | Accessibility statement | New | No |
| X-08 | JavaScript off, offline, slow network | "With JS off or the model down, every link, page and form still works" (`docs/06-website-experience.md:576-577`) | Static pages work; forms that need the server (orders, sign-in) don't exist yet |

**State matrix for every interactive surface** (peek, Ask bar, thread, studio, search,
checkout, account, artifact):

| State | Rule |
|---|---|
| Guest | Ask, compare, design and share by link work. Ordering, saving a design to the account and tracking details ask for sign-in and say why. |
| Signed in | Account chip in the header; desks see "customer" only. |
| Loading | No skeleton for static pages. For answers: a real progress label within 300 ms ("Checking today's stock"), first words under 1.5 s (`docs/06-website-experience.md:561-575`). No typing dots. |
| Empty | Say why it's empty and the next step ([voice.md](voice.md) rule 6). |
| Error | Say what broke and what to do now; never blame the owner; never show the platform reason to visitors. |
| Offline | Static pages from cache where possible; forms show "You're offline. This will send when you're back" only if the action is safe to queue; payments never queue. |
| Resting | Desks grey; forms work; the words above. |
| Held | Amber dot; "Waiting for Wanjiru's OK. Nothing charged." |
| Person joined | Person chip (a circle), "Wanjiru from Kilele joined · a person"; desks silent until hand-back. |
| Reduced motion | No animation; states read by shape. |

---

## 3. The customer journey, end to end

Worked on Kilele with Njeri (`docs/06-website-experience.md:956-1029`;
`concept/journeys/index.html`). Phases: **B1** the first release (this weekend), **B2** the fleet release (12 Nov,
payments and refunds), **P** pilot.

| Step | What Njeri sees | Screens | Backend capabilities required | Owner meets | Phase | Built today |
|---|---|---|---|---|---|---|
| 0. Find the site | A WhatsApp link from her sister; a search result; or her assistant reads the door | K-01, M-04, M-07 | Signed static release on the edge (W1); Open Graph previews; Markdown twins; AI Catalog; consent signals in robots.txt | none | B1 (twins), P (catalog) | Release serving and twins |
| 1. Arrive as a guest | The page loads as itself. No pop-up. Desks with AI chips. "Do you deliver to Kilimani on Saturday?" gets a cited answer | K-01, S-1, S-3, S-4, S-5 | Desk manifest in the release (R4); peek from the manifest with live facts from the edge cache (R3); Ask over SSE with typed frames (R5, F1); retrieval scoped to the release; citations with anchors (R7, U18); promise checker (R8, U20); refusal policy; content-free visit counting with opt-out (§4.9) | A beacon count | B1 (Ask), P (desks) | Ask with citations and refusal; no desks, no counting |
| 2. Browse and ask | Instant starters; holds ("Hold 2 loaves till 12:00"); honest gaps passed to Wanjiru; handover to a person | K-02, K-03, K-14, S-4 | Capabilities answered without a model (hours, stock); standing grant for holds, signed (ADR-006); pass-to-owner store (built); person handover with honest wait time from stated hours (R14); thread token (F2) | Teach your site list; a line in the brief for holds; a WhatsApp card when she asks for Wanjiru | B1 (pass), P (rest) | Pass to owner |
| 3. Make the cake | One question at a time; sketch from parts; price line by line from rules; save and share | K-04, R-01, R-03 | `ConfigurableProduct` in the signed content model (R9); deterministic price and constraint rules; calendar capacity; sketch composer (S1); quote pinned to the release with `valid_until`; `VisitorArtifact` store and `/a/{id}` route (R10, F4) | Nothing unless she shares it with the shop | P | None |
| 4. An account when needed | "Sign in to order", a passkey first, what Kilele keeps, three optional choices with receipts | A-01 to A-08, A-14, A-15 | Visitor identity per site host (U19): WebAuthn with the site's rpId; password with breach check; TOTP always on without a passkey; Google, LinkedIn, GitHub OIDC (name and verified email only); email verification by code; sessions; consent receipts store; thread resume after sign-in | none | P (B1 shows it as concept) | Owner auth exists on the platform host only |
| 5a. Order from the catalogue | Basket with sources, live slots, total, pinned quote, USD at the posted rate | K-05, K-06, A-20 | Catalogue, stock per bake day, delivery zones, slots and fees, quote engine, order record with idempotency, live re-check at submit (W8) | none | P | None |
| 5b. Order something custom | "Request this cake" → "Waiting for Wanjiru's OK. Nothing charged." | K-04, K-07, K-08 | `request_custom_order` as a held call carrying the artifact digest and quote; decision card with the design; approval fingerprint (ADR-006); expiry and lapse; customer notification of approve, decline, change, lapse (W9) | **Order card** (`meet-order`): approve with passkey or step-up code | **B1** (ends here) | Door tool and held call exist with a minimal schema (`door/mod.rs:319-385`); no site page |
| 6. Pay | Amount, payee and rate first; PayPal's own window; back to "Confirmed" and a receipt. M-Pesa STK Push for Kenyan numbers; card fields hosted by the provider | K-10, K-11 | Effects Gateway holds PayPal keys; Orders v2 create for the exact amount under the fingerprint; capture once; webhook reconciliation; payment request only after approval for custom orders; voiding on change; M-Pesa Daraja STK Push; card provider; signed payment receipts; posted-rate rule (W6) | **The payment** lands on her order; **the payout** settles in Kilele's PayPal balance | B2 (PayPal); M-Pesa and card: decide (D3) | None |
| 7. After the order | Receipt she can verify; status timeline; ask to move the date before the cut-off; "Something's wrong" with a photo; refund under the rule at once, above it held | K-08, K-11, K-12, K-13 | Order status events and who sets them (G-41); notifications by email and opted-in WhatsApp (W9); change requests held unless a rule covers them; cut-off rule; refund rule with four limits and the site-wide value-given cap, checked by the gateway; photo intake as hostile input; receipt verification on the site domain | **A refund**: under the rule a line in the brief; above it a card | B2 (refunds), P (status, changes) | None |
| 8. Her AI agent orders | Her assistant finds the door, quotes, makes a checkout link; she reviews on Kilele's page and pays on PayPal's; the cake request is held | M-02 to M-05, M-09, K-08 | AI Catalog; MCP tools from the manifest; signed A2A card; Web Bot Auth verification and tiers (U23); agent terms; `checkout.create` returning a signed link, never a credential; held calls as A2A tasks; AP2 mandate chain (U24) | Order card for the custom cake; payment as in step 6 | B1 (custom order via MCP), P (the rest) | `ask_site`, `request_custom_order`, held-call status |
| 9. Security moments | New sign-in email and alert with "Sign out everywhere"; fresh sign-in for big changes; desks refuse account changes; a fake payment link checked by the Orders desk; a look-alike where her passkey isn't offered; report forms; export and erasure with receipts | A-09 to A-13, K-08, X-05 | Sign-in history (13 months); new-device detection; session revocation; step-up for changes and deletion; notice to the old email; desks without account tools; `payment_request.check`; report routing to owner and operator (W3); export bundle; erasure across every store with a receipt (ADR-028) | Reports reach Wanjiru (and the operator for look-alikes) | P | None |

**Classy Perfumes variant.** Arrive → search (C-02) with editable reading and honest fit →
compare (C-05) → plan with the Gifts desk (C-06) → share five private links with prices hidden
(R-03) → each invitee chooses (R-02) → Achieng requests the order → **owner card** with typed
fields → payment request on her orders page → M-Pesa STK Push or PayPal → delivery by Fri 4 Dec.
Extra capabilities beyond Kilele: hybrid search with rerank and fit badges from catalogue
attributes (W1, W8), per-person share tokens and permissions (W1), invitee identity without an
account (W6), gift wrap and card lines (W8).

**News variant.** Arrive → section desk answers from published stories only → catch me up from
device memory → follow a story → set up a digest (contract, double opt-in) → hit a subscriber
story (headline and link only) → subscribe or sign in → account memory if she opts in →
unsubscribe in one tap. Extra capabilities: entitlement-aware retrieval (R17), device memory
schema in the manifest (S2), corrections that reach answers and digests within minutes, digest
assembly per subscriber without the Editor touching subscriber data (R13), double opt-in and
unsubscribe (W9).

---

## 4. Gaps: what the design implies but never drew

Priority: **Must** for a production site and for the samples to work end to end; **Should**
before the pilot; **Could** later.

| ID | Gap | Why | Priority | W |
|---|---|---|---|---|
| G-01 | Returning customer sign-in (passkey and password-plus-code) | Only first-time creation is drawn (A-14) | Must | W6 |
| G-02 | Customer accounts on each site's own domain with the site's passkey rpId | The look-alike defence depends on it; owner auth runs on the platform host only (`auth/routes.rs:44-53`) | Must | W6 |
| G-03 | Email verification by code before ordering | Required by 06 §4.10 (`docs/06-website-experience.md:692-693`) | Must | W6 W9 |
| G-04 | Recovery: lost passkey device, lost authenticator, no codes left | Recovery codes are drawn; using them is not; "a desk can never recover an account" leaves people with no path | Must | W6 |
| G-05 | Forgot password without a magic link | 06 bans magic links (`docs/06-website-experience.md:688-689`); a reset needs a design | Must | W6 (D6) |
| G-06 | Sign out, session expired, signed-out states | Not drawn | Must | W6 |
| G-07 | Account home, details form, addresses and recipients, sign-in methods | Only the alert, a re-auth gate and data pages are drawn | Must | W6 W8 |
| G-08 | Custom order declined, owner suggests a change, request lapsed | Named in 06 (`:1024-1025`) and customer-order (`:244`) but never drawn | Must | W1 W8 W9 |
| G-09 | Order history list empty state and filters | Two cards drawn, no empty state | Must | W2 |
| G-10 | Guest order lookup with a one-time code (K-09) | 06 §5.6 rule; nav-agents shows only a toast | Must | W6 W9 |
| G-11 | Every transactional email: account created, verify email, new sign-in, request held, approved with payment request, declined, lapsed, payment receipt, status updates, change approved or declined, refund issued, erasure and export ready | Only the new-sign-in email is drawn (`concept/journeys/customer-security.html:110-120`) | Must | W9 |
| G-12 | WhatsApp templates for opted-in order updates; one-time code messages | A-08 offers it; nothing is drawn; provider not chosen (`docs/01-product.md:287`) | Must for Kilele | W9 |
| G-13 | Digest confirm and unsubscribe pages; "offers by email" unsubscribe link | The contract promises one-tap stop and deletion | Must for the news sample | W9 |
| G-14 | Privacy notice page per site with visit counts, retention and the opt-out | 06 §4.9 gives the words; no page | Must | W6 W7 W2 |
| G-15 | Add to basket, quantities, remove, empty basket | K-06 starts from a full basket | Must | W8 W2 |
| G-16 | Classy home, product pages, bag and checkout | Not drawn (C-01, C-04, C-07, C-08) | Must | W2 W8 |
| G-17 | Address entry with recipient name and phone (ordering for someone else) | "Amina's home" has no form; delivery needs a recipient | Must | W8 W6 |
| G-18 | Stock gone or slot taken at submit; quote expired; price changed since the quote | Live facts are display-only and orders re-check (`docs/06-website-experience.md:1141`) | Must | W8 |
| G-19 | Guest checkout (owner's choice) | 06 `:1028-1029` | Should | W6 W8 |
| G-20 | PayPal cancelled, capture failed, pending | Not drawn | Must for B2 | W6 |
| G-21 | M-Pesa states: enter number, waiting for PIN, timeout, insufficient funds, wrong number, cancelled on the phone | Only described in a box | Must if M-Pesa ships | W6 |
| G-22 | Payment request voided after a change, and the new request | Rule in `concept/journeys/customer-security.html:238` | Must | W6 W8 |
| G-23 | Double submit and retry messages | Idempotency exists in the design; the words don't | Should | W6 W7 |
| G-24 | Order status updates: who sets "Baking", "Out for delivery", "Delivered" | Drawn as a timeline with no source | Must | W5 W8 (G-41) |
| G-25 | Branded 404 page, generated by the Move for every release | Edge serves plain text (`site/mod.rs:266-278`) | Must | W2 W1 W4 |
| G-26 | Branded unavailable, platform-hold and takedown pages with operator small print | Plain text today (`site/mod.rs:164`) | Must | W3 W7 |
| G-27 | Rate-limit words for Ask, pass-to-owner, sign-in, codes | Pass is limited to 5 an hour per IP (`pass.rs:28`) but the visitor words are generic; `/ask` has no limit | Must | W1 W6 W7 |
| G-28 | Site-branded receipt page and Verify on the site domain | Built only on the platform host in console chrome | Must | W6 W7 |
| G-29 | Terms of sale, delivery, refunds and returns pages per sample | Cited facts need pages; returns differ per shop | Must | W2 |
| G-30 | Help pages a desk can cite: what desks can't do, how the site contacts you, changing your details | The security story cites them (K-17) | Must | W2 W7 |
| G-31 | Search with no results; plain results with zero hits; misspellings | Only "No product names match" in suggestions | Must | W1 W2 |
| G-32 | Artifact link states: expired, revoked, not permitted, changed since, saved; link previews | R-04 | Must for Classy | W1 W7 |
| G-33 | Language switch (EN · SW) and approved Swahili pages | Pill drawn (`concept/site.html:185`); voice.md requires human-written Swahili | Should | W7 W2 |
| G-34 | Accessibility statement | Public bodies require it; good practice for all | Should | W7 |
| G-35 | No-JS and offline behaviour for forms (order request, sign-in fallback, search) | Principle in 06 §4.7, not designed | Must | W2 W7 |
| G-36 | Lockout and throttling words for sign-in and codes | Not drawn | Must | W6 W7 |
| G-37 | Person handover thread: Wanjiru's reply appears in the thread, "joined", "back" | Words in 06 `:459-464`; not drawn on the site | Must | W1 W5 |
| G-38 | Device memory "See or delete" ledger page | Linked in nav-agents `:479`, not drawn | Should | W1 W7 |
| G-39 | Account memory items page | R12; not drawn | Should | W1 W6 |
| G-40 | Owner admin for catalogue, prices, stock, slots, orders, stories, subscribers, digests, presentation scripts | No console screen draws them; the console design covers cards, brief and trust | Must | W5 W8 |
| G-41 | The order status source and the owner's "mark as baked or delivered" action | Status timeline has no writer | Must | W5 W8 |
| G-42 | Report-a-look-alike and report-a-payment-link receiving end, with acknowledgement | Forms drawn; routing to owner and operator not designed | Should | W3 W6 |
| G-43 | Subscription purchase, renewal, cancel, and the subscriber sign-in for the news sample | Implied by the lock and the Subscriptions desk | Must for news | W6 W8 |
| G-44 | Story page, section front, archive, corrections log | Not drawn | Must for news | W2 |
| G-45 | Cookie and storage disclosure for sign-in sessions and device memory | No banner is needed for counting (06 §4.9); strictly necessary cookies and device memory still need a line in the privacy notice | Must | W6 W7 |
| G-46 | Minors: memory is never offered to minors, but no age signal exists | 06 R12 | Should | W6 (D7) |
| G-47 | Booking flow (classes, a call on zavora.ai) | 06 §5.6 Bookings; not drawn | Could | W8 W2 |
| G-48 | Card writing step for gifts | Mentioned only | Could | W1 |
| G-49 | Reviews on product pages (Reputation) | Back-office agent exists; no visitor surface | Could | W2 |
| G-50 | Continue on my phone (QR) | T4 | Could | W1 |
| G-51 | Voice with captions | Later tier | Could | W1 |
| G-52 | Plain-language mode and translations | T1/T4 | Could | W1 W7 |
| G-53 | Print styles for receipts and artifacts | Customers print receipts | Could | W7 |
| G-54 | Data export for a guest who ordered without an account | Erasure and export are designed for accounts only | Should | W6 |
| G-55 | Open Graph images and titles for every page and artifact | Link previews are part of the referral loop | Should | W2 W7 |
| G-56 | Business identity details in the footer or a contact page (registered name, address, contact) | Expected for any shop taking payments | Should | W2 |
| G-57 | "Was this right? No" follow-up (what was wrong) | Feedback is one tap; owners need a reason | Could | W1 |
| G-58 | Agent door human page and agent terms | Drawn; not built; terms need legal review | Should | W1 (D8) |

---

## 5. Design system extraction

### 5.1 Two token layers

The concepts use two layers, and the kit must keep them apart.

**Layer 1: Agentrix tokens** (`concept/styles.css:4-65`). Used by the platform's own pages, the
owner console, the "Sample" band, sheets that the platform owns (share settings), and as
fallbacks.

| Group | Tokens |
|---|---|
| Brand | `--indigo #38356d`, `-900 #1f1d3d`, `-700 #2c2a57`, `-100 #ecebf6`, `-50 #f6f6fb`; `--coral #de4060`, `--coral-strong #b72d50`, `--coral-50 #fff0f4` |
| Neutrals | `--ink #23223a`, `--ink-2 #4a4960`, `--muted #656479`, `--line #e3e2ec`, `--line-strong #cfcedd`, `--paper #fff`, `--surface #f6f6fa`, `--surface-2 #efeff5` |
| Semantics | ok `#11734a` on `#e6f5ee`; wait `#8a5300` on `#fff4dd`; risk `#a3302a` on `#fdecea`; info `#2f55c4` on `#eaf0ff`; sealed `#3b3a78` on `#eeedfb` |
| Type | Manrope (display), DM Sans (text), JetBrains Mono; sizes 12, 13, 14, 16, 18, 20, 24, 32, 44, 56 |
| Space | 4, 8, 12, 16, 24, 32, 48, 64 |
| Radius | 8, 12, 18, 26 |
| Depth | three shadows; focus ring `0 0 0 3px` coral at 35% |
| Motion | `--ease cubic-bezier(.2,.7,.2,1)`; `rise` and `pulse` keyframes; reduced motion stops all (`:234-236`) |

**Layer 2: host tokens `--wa-*`** (`concept/experience/experience.css:5-23`), extracted from the
site at the Move so desks wear the site's brand.

| Token | Meaning | In code (`crates/agentrix-move/src/theme.rs:18-54`) |
|---|---|---|
| `--wa-bg`, `--wa-soft` | Surfaces | `bg`, `soft` |
| `--wa-ink`, `--wa-ink-2` | Text | `ink`, `ink2` |
| `--wa-line` | Rules | `line` |
| `--wa-accent`, `--wa-on-accent` | Brand colour and text on it | `accent`, `on_accent` |
| `--wa-accent-2` | Second brand colour (stage ring, gold) | missing |
| `--wa-font`, `--wa-display` | Fonts | `text`, `display` |
| `--wa-radius` | Card radius (18 px default); also seeds the mark's squareness | missing |
| `--wa-shadow` | Peek and rail shadow | missing |
| `--wa-wait`, `--wa-ok` | Held and ok colours that pass contrast on the site's surface | missing |
| `--wa-ground` | Ring colour behind the waiting dot (`concept/journeys/journeys.css:217`) | missing |
| `--wa-*-dark` | Dark-mode set read by the Ask island (`crates/agentrix-console/static/site/ask.css:18-24`) | missing: the island falls back to platform dark colours while the page stays light |
| `--t-fast 120ms`, `--t-med 220ms` | Peek and rail motion | missing |

### 5.2 Host token sets per sample

| Sample | Source | bg / soft | ink / ink-2 | line | accent / accent-2 | display |
|---|---|---|---|---|---|---|
| Kilele | `concept/journeys/journeys.css:211-218` | `#fffdf9` / `#fbf4ea` | `#3b2416` / `#6b4b36` | `#ecdcc6` | `#b4541f` / `#b4541f` | Fraunces |
| Classy | `concept/experience/search.html:14-19` | `#ffffff` / `#f7f3ee` | `#1c1719` / `#5b5056` | `#e7ddd2` | `#4a2340` / `#a8842f` | Cormorant Garamond |
| Kestrel (news patterns) | `concept/experience/news.html:14-19` | `#ffffff` / `#f4f2ec` | `#141414` / `#4b4b4b` | `#d9d6cf` | `#0f4c5c` / `#9a3b17` | Newsreader |
| Tana (recommended news brand) | `concept/identity/marks.html:495-499` | `#f4f5f7` | `#14213d` / `#56607a` | `#dfe3ea` | from palette `#b3261e`, `#2f5d8a` | Libre Caslon Text |
| zavora.ai | `concept/experience/presentation.html:14` | `#fff` / `#f6f6fb` | `#1f1d3d` / `#4a4960` | `#e3e2ec` | `#38356d` / `#de4060` | Manrope |

### 5.3 The living mark and its states

The experience concepts draw the earlier plain square (`concept/experience/experience.css:56-83`);
the journeys and the identity reference draw the living mark decided in ADR-031
(`concept/journeys/journeys.js:151-207`; `concept/identity/marks.html`). Build only the living
mark:

- **MarkSpec:** site, desk id, colour, squareness `n`, glyph or monogram, family (orbits, grain,
  strata, weave, facets), seed, font. Sizes 20, 28, 40 px; signature from 28 px; signature opacity
  capped at 22% (28–44 px) and 30% above (`journeys.js:105-106`).
- **One generator, two builds:** Rust renders SVG into the release and PNG for WhatsApp; the
  same algorithm in JS for the console's live preview; golden tests keep them identical
  (`concept/identity/marks.html:414`).
- **States** (`concept/journeys/journeys.css:175-208`): idle, listening (breathing halo), working
  (trace line, signature spins or drifts), answering (steady ring), speaking (ring follows the
  audio level), waiting (amber dot), resting (grey); console-only asleep, paused, frozen. Motion
  settles after 5 s (`journeys.js:187-196`). Reduced motion keeps shapes.
- **Contrast:** mark 3:1 against the site background; glyph 3:1 against the fill with the
  signature drawn (`journeys.js:105-106`); the editor offers the nearest passing shade.
- `aria-hidden` on the mark; the name is text beside it; state is announced through a
  `role="status"` element.

### 5.4 Component inventory

| Component | Purpose | Source | States and rules | Used by |
|---|---|---|---|---|
| AI chip | "AI" beside every mark a visitor sees | `.wa-ai` `concept/experience/experience.css:85-86`; `.k-ai` `concept/journeys/journeys.css:247`; `.ax-ai` `crates/agentrix-console/static/site/ask.css:36` | One component, three copies today | Every desk surface |
| Desk nav item and peek | Menu item that is a desk | `experience.css:88-120`; `concept/experience/nav-agents.html:376-439` | closed, open, pinned, answering; bottom sheet under 720 px (`experience.css:242-246`); `role="dialog"`, `aria-expanded` on the link, Esc returns focus; must not cover neighbouring items (`docs/06-website-experience.md:1341-1343`) | S-1 |
| Live fact chip | A fact with an as-of time | `.fact` `experience.css:111-113` | fresh; stale; unavailable | Peeks |
| Starter | Suggested question, "Instant" marker | `.starter` `experience.css:115-117` | instant (no model); normal | Peeks, threads |
| Input | Desk input with send and voice | `.wa-input`, `.wa-send`, `.wa-mic` `experience.css:123-131` | focus (listening state on the mark); disabled; voice "later" | Peek, rail, studio |
| Thread | Visitor and desk turns | `.wa-thread`, `.wa-v`, `.wa-a` `experience.css:133-139` | `role="log"`, phrases not characters, `aria-busy` while streaming, no autoscroll | Rail, peek, studio, plan |
| Citation chip and source list | `[n]` in text and numbered sources that point at the page | `.cite`, `.srcs`, `.src` `experience.css:140-146`; `.ax-cite`, `.ax-sources` `ask.css:48-50` | subscriber-locked source; clicking scrolls and highlights | Every answer |
| Pointer highlight | The cited element lights up with "Cited [n]" | `.pointed` `experience.css:168-170`; `concept/experience/experience.js:27-41`; `:target` style in `crates/agentrix-console/static/site/site.css:31` | lasts about 5 s; reduced motion: no pulse | Pages |
| Action buttons | Real capabilities in an answer | `.act` `experience.css:147-151` | primary; disabled after use; held result line | Answers |
| Basis line and feedback | "From the price list, 2 Nov" and "Was this right? Yes · No" | `.wa-meta` `experience.css:152-153`; `experience.js:115-118` | answered; doesn't say; instant | Answers |
| Progress label | Real tool label while working | `.wa-progress` `experience.css:154`; `experience.js:66-72` | appears under 300 ms; never fake | Answers |
| Handover line | Desk to desk, or to a person | `.wa-handover` `experience.css:155-156` | desk; person; held | Threads |
| Person chip | A person, with honest wait time | `.wa-person` `experience.css:157-159` | circle initial, never a squircle | Rail, footer |
| Disclosure line | "You're talking to AI…" | `.wa-disclose` `experience.css:160-161`; `.k-first` | fixed words (06 §4.6) | Every thread |
| Ask bar | One line in the layout, `/` to focus | `.ask-dock` `experience.css:172-177` | idle; focused; no JS | S-3 |
| Rail and sheet | Thread across pages | `.rail` `experience.css:180-190`; phone `:247-248` | open; closed; desk changed | S-4 |
| Memory chip and ledger | Session notice, remember, see or delete | `.memory-chip` `experience.css:191-192`; `.ledger` `:232-235`; news chips `concept/experience/news.html:246-252` | session only; remembered on device; forgotten | Rail, news |
| Product card and fit badges | Shortlist card | `concept/experience/search.html:57-70` | best fit; exact match; over budget; not in stock; image labelled | Search, plan |
| Intent chips, refine, what I used | Search side panels | `search.html:81-92`, `:337-344` | editable; unsupported (greyed with reason) | Search |
| Comparison table | Products side by side | `search.html:93-98`, `:361-368` | unpublished attributes not compared | Search, artifacts |
| Price lines with rule ids | Line-by-line price from rules | `concept/experience/creator.html:155-159`; `.k-line`, `code.rule` in customer-order | each line names its rule; total | Studio, checkout |
| Pinned quote line | Release, price list, valid until | `concept/journeys/customer-order.html:132`, `:209` | valid; expired; changed since | Studio, checkout, artifacts |
| Source line | "Source · Price list, 2 Nov" | `.k-src` `concept/journeys/journeys.css:239-240` | live sources marked "(live)" | Basket, pay, change |
| Slot picker | Real slots only | `.k-slots` in `customer-order.html:119-131` | open; full (disabled with label) | Checkout, change |
| Status pill | Order and artifact states | `.status` `experience.css:209-213`; `.k-pill` `journeys.css:243-246` | draft, shared, held, ok, risk | Orders, artifacts |
| Status timeline | Steps with when | `.k-time` in `customer-after.html:160-170`; `.track` `concept/site.html:90-95` | done, now, next | Order page |
| Sketch stage | Composed SVG with "Sketch, not a photo" | `creator.html:146-153`, composer `:244-289` | empty; partial; complete; sanitised SVG only | Studio, card for owner |
| Artifact | Safe blocks with head and foot | `.artifact` `experience.css:200-208`; `artifact.html:212-248` | fresh block; visitor-written block marked | Plans, briefs, decks |
| Sheet with option rows and switches | Share settings, choices | `.sheet`, `.opt-row`, `.switch`, `.linkbox` `experience.css:215-229`; `.k-choice` in customer-account | switch `role="switch"` with visible On/Off | Share, choices |
| Consent receipt | Purpose, what, shared with, kept, given, withdrawn | `customer-account.html:300-336` | active; withdrawn | Account |
| Erasure receipt | Per-store rows | `concept/journeys/customer-security.html:356-380` | erased, kept reduced, pseudonymised, scheduled | Account |
| Alert with one action | New sign-in | `customer-security.html:129-135` | shown; dismissed | Account |
| Provider placeholder | Another company's window, never redrawn | `.j-provider` `journeys.css:260-264` | PayPal, passkey prompt, WhatsApp | Pay, sign-in |
| Budget meter | Plan budget | `artifact.html:218` | ok; over | Plan |
| Digest contract | Four plain rows | `news.html:274-283` | topics change the "You get" row | Digest |
| Story timeline | Dated steps with correction | `news.html:143-150` | correction; scheduled | News |
| Stage bar | Tour controls | `concept/experience/presentation.html:199-215` | playing; paused; answering; ended | Tours |
| Toast | One-line confirmation | `.toast` `experience.css:237-239`; `experience.js:43-48` | `role="status"` | Everywhere |
| Sample band | "Sample" notice | `.s-fixture` `site.css:24`; `page.html:31` | on for samples | All samples |

### 5.5 What the shared kit must contain

1. **Tokens as data.** A single token file for each layer; the Move's `Tokens` grows to the full
   `--wa-*` set (§5.1), including the dark set and the radius that seeds the mark.
2. **Server templates** (minijinja, strict) for every component above, so pages work without
   JavaScript, plus small islands for the peek, Ask bar, rail, studio, search and artifact.
   Islands set server text with `textContent` only, as `ask.js` does (`ask.js:5`).
3. **One frame client** for the typed SSE frames (`ready`, `awp.presence`, `delta`,
   `awp.citation`, `reply`, `error`, `done`, later `held`, `handover`, `action`,
   `awp.artifact.patch`, `stage.cue`) shared by the peek, rail, studio and search.
4. **The living mark** generator and CSS states (§5.3).
5. **A words table** keyed by state and audience (visitor and owner words per state,
   `docs/06-website-experience.md:446-453`), in one place, ready for Swahili.
6. **Budgets.** Agent layer at most 20 KB gzipped, loaded on idle or first interaction; zero
   layout shift; peek under 100 ms from the manifest (`docs/06-website-experience.md:563-566`).
7. **Accessibility.** WCAG 2.2 AA; every hover affordance has a focus and touch equivalent;
   `role="log"` threads; status through `role="status"`; 44 px targets; sheets on phones;
   reduced motion.
8. **Light and dark.** The site's brand in both, or light only with a clear decision; never a
   dark island on a light page.
9. **A visual test page** per component per sample brand, compared against the concept, as
   [how-we-work.md](how-we-work.md) requires.

### 5.6 Inconsistencies to fix in the kit

- Three AI chips, two Ask surfaces, two mark styles: one each.
- Host token names differ between files (`--k-*`, `--cp-*`, `--ink-n`): the kit uses `--wa-*`
  only; site-specific names stay inside the site's own CSS.
- The theme's `site.css` claims light and dark but only sets `color-scheme`
  (`crates/agentrix-console/static/site/site.css:36-38`).
- `ask.css` places the island fixed at the corner with a very high z-index
  (`crates/agentrix-console/static/site/ask.css:11`); the design docks it in the layout.

---

## 6. Copy review

The concepts are mostly plain and specific. The problems cluster in four places: flattery
openers in desk lines, salesy phrases near money, slogan headings on the explainer pages, and
vocabulary drift ("AI concierge", "chatting"). [voice.md](voice.md) is the rulebook; this
section applies it to the website.

### 6.1 Strings visitors would see

| Where | String | Problem | Rewrite |
|---|---|---|---|
| `concept/site.html:212` | "You're chatting with an AI concierge. It only uses this site's pages and never guesses prices or dates. Ask for Wanjiru anytime; she replies on WhatsApp." | "chatting", "AI concierge" (06 says WebAgent and Front desk); differs from the fixed disclosure; no wait time | "You're talking to AI. It answers only from this site and never guesses prices, dates or allergens. Wanjiru replies on WhatsApp, usually within 2 hours (7:00–20:00)." |
| `concept/site.html:199` | "AI concierge · answers from this site" | Wrong term | "Front desk · AI · answers from this site" |
| `concept/site.html:273` | "This site uses an AI concierge" | Wrong term; 06 §4.6 fixes the footer line | "The WebAgents on this site are AI." |
| `concept/site.html:274` | "Made with Agentrix · Move your site in 2 minutes →" | Unbacked claim: the assessment takes about 3 minutes and a Move up to 10 (`specs/first-release/requirements.md`, N1) | "Made with Agentrix · Get a free check of your own site" |
| `concept/site.html:371` | "Happy to help. Celebration cakes need two days' notice…" | Banned opener ([voice.md](voice.md)) | "Celebration cakes need two days' notice, and Wanjiru confirms every order. The order form is below." |
| `concept/site.html:248`; `concept/experience/creator.html:451` | "Then pay by M-Pesa to lock it in." | Sales phrase | "Then you pay by M-Pesa, and the order is confirmed." |
| `concept/experience/creator.html:421` | "Lovely. From Kilele's options I've picked …" | Flattery opener | "From Kilele's options: forest-green ombré buttercream and fresh berries." |
| `concept/experience/artifact.html:311` | "Lovely. Give me a word or two about each and I'll draft a plan…" | Flattery opener | "Give me a word or two about each person. I'll draft a plan from what the shop sells, and you can share it so each one chooses." |
| `concept/experience/artifact.html:273` | "…so I can't share them. It's a gift!" | Exclamation and a quip from a desk | "Achieng chose to keep prices private, so I can't share them." |
| `concept/experience/creator.html:308` | "Strawberries are in season this week, and the berries are fresh on the day." | A claim with no source, shown as a rule's note | Cut, or only if the cakes page says so, with its citation: "Berries are added on the day [1]." |
| `concept/experience/search.html:279` | "Bergamot and neroli; light and clean. Only 3 left." | "Only … left" reads as urgency (06 §4.3 bans false urgency) | "Bergamot and neroli; light and clean. 3 in stock." |
| `concept/experience/search.html:232` | "Warm amber, smoked vanilla, a whisper of oud. Made for Nairobi evenings." | Stock perfume-copy cliché; in seed data it reads machine-made | "Amber and smoked vanilla with a little oud. A warm evening scent." (Seed copy is the shop's voice; desks never write product copy.) |
| `concept/experience/search.html:365` | Comparison cell "The whole point" | A joke inside a facts table | "Vanilla candle and vanilla spray" |
| `concept/experience/search.html:273` | "Prices and stock from release v31 · ranked by fit to your request, never by margin" | "release v31" is our jargon; "never by margin" is defensive | "Prices and stock as of 11:02. Ranked by how well each fits your request." |
| `concept/experience/creator.html:141` | "AI WebAgent. It only offers what Kilele makes; prices come from the price list; allergens are never guessed." | Semicolon chain; differs from the fixed line | "You're talking to AI. It offers only what Kilele makes, and never guesses prices, dates or allergens." |
| `concept/experience/nav-agents.html:289` | "I design celebration cakes with you from what Kilele actually bakes, price them from the price list, and hold the order for Wanjiru." | "actually" filler; three verbs in a row | "I help you design a cake from Kilele's own options. Prices come from the price list. Wanjiru confirms every order." |
| `concept/experience/nav-agents.html:225` | "Remember me here" | Scope unclear | "Remember me on this device" |
| `concept/experience/nav-agents.html:308` | "Answered instantly from today's stock · no AI model used" | Implementation detail | "From today's stock list, 09:42" |
| `concept/experience/experience.js:117` | "Thanks. The owner sees this in “teach your site”." | Owner jargon shown to a visitor | "Thanks. Wanjiru will see this so the page can be fixed." |
| `crates/agentrix-console/static/site/ask.js:28`; `crates/agentrix-node/src/site/ask.rs:707` | Desk name "Concierge" | Owner-facing name; visitors see "Front desk" (`docs/06-website-experience.md:587`) | "Front desk" |
| `crates/agentrix-node/src/site/ask.rs:671` | "This site doesn't say. I won't guess." | Fine, but 06 uses "The site doesn't say, so I won't guess." Pick one and keep it fixed | "This site doesn't say, so I won't guess." |
| `concept/experience/presentation.html:132` (a sample page) | "The site answers every customer, sells, publishes, defends and maintains itself, and asks its owner before anything that matters." | Overclaim ("every"); five-verb list | "Agentrix moves your website onto WebAgents. They answer customers from your pages, take orders and keep the site current. Anything that costs money or makes a promise waits for you." |
| `concept/experience/presentation.html:139` | "Migration as a show, room by room." | Slogan | "How a site moves, step by step" |
| `concept/experience/presentation.html:163` | "Promises, enforced in code." | Slogan fragment | "Four promises, and how each is kept" |
| `concept/experience/presentation.html:189` | "an honest health report and a living preview in about three minutes" | Self-praise ("honest") | "a health report with evidence for each finding, and a working preview, in about three minutes" |

### 6.2 Explainer and marketing strings in the concepts

These pages are internal explainers, but their lines get reused in decks and on zavora.ai.

| Where | String | Problem | Rewrite |
|---|---|---|---|
| `concept/experience/index.html:107` | "What every website becomes." | Grand claim | "What a site can do after the Move" |
| `concept/experience/index.html:128-133` | "Wow: …" labels on six cards | Hype label | "Why it matters: …", or drop the label |
| `concept/experience/index.html:126` | "Six ideas, each one a concept you can try." | Filler | "Six ideas. Each links to a working concept." |
| `concept/experience/index.html:150`, `nav-agents.html:252` | "Presence without the bot costume." / "Presence, not a costume" | "Not X but Y" shape | "How a WebAgent shows it is working" |
| `concept/experience/index.html:153` | "The page is the avatar … Motion belongs to the content, not to a character." | Aphorism; "not X but Y" | "When a desk cites a price, the price row on the page lights up. Tours scroll the real sections." |
| `concept/experience/search.html:139-140` | "Search that answers, cites and acts." / "understands a request rather than matching keywords" | Rule of three; "rather than" contrast | "Search that answers from the shop's own pages. Type a request, see how it was read, and get an answer with sources. Plain results stay one tap away." |
| `concept/experience/artifact.html:110` | "Make it together, share it, act on it." | Rule of three slogan | "Plans you make with a desk and share with friends" |
| `concept/experience/artifact.html:145-148` | "Why owners love it" / "word of mouth with a working 'buy' inside" | Puffery | "What the owner gets" / "Each shared link carries the shop's name, and the group can request the order from it." |
| `concept/experience/news.html:107` | "A newsroom whose desks answer back." | Cute heading | "A news site whose desks answer from published stories" |
| `concept/experience/news.html:171` | "an integrity feature print never had" | Inflated | Cut the clause. |
| `concept/experience/news.html:170` | "The archive becomes the product." | Slogan | "Readers get answers from the archive, with links back to the stories." |
| `concept/journeys/index.html:92` | "where the two lives touch" | Poetic | "where the two lanes meet" |
| `docs/01-product.md:5-6` (reused on zavora.ai) | "a website that answers every customer, sells, publishes, defends and maintains itself" | As above | As the zavora.ai rewrite in §6.1 |

### 6.3 Vocabulary drift

| Use | Not | Why |
|---|---|---|
| WebAgent; the desk's name (Cakes, Orders) | AI concierge, chatbot, bot, assistant (for our desks), staff | 06 §4.8 (`docs/06-website-experience.md:601-604`) |
| Front desk (visitor) / Concierge (owner) | Concierge to visitors | 06 §4.8 |
| talking to AI | chatting | Fixed disclosure line |
| Bread (Kilele), Fragrances (Classy) | Bakes, Scents | Pick once (A8, A11) |
| price list, 2 Nov | release v14 (to visitors) | Visitors know the owner's words; the release sequence stays in machine fields |
| KSh 4,500 (prose), KES (machine fields) | KES in visitor prose | [voice.md](voice.md) rule 11 |
| Waiting for Wanjiru's OK. Nothing charged. | Pending approval | 06 §4.3 |
| assistant (for the customer's own AI) | none | Allowed: it is not ours, and the customer-agent concept uses it that way |

### 6.4 Copy rules for the website

[voice.md](voice.md) applies in full. These add what is specific to visitor screens, drawn from
the voice 06 already uses:

1. **Fixed lines stay fixed.** The disclosure ("You're talking to AI. It never guesses prices,
   dates or allergens."), the footer line, "Waiting for {owner}'s OK. Nothing charged.",
   "Answers are paused right now…", the doesn't-say line. They come from one words table; a desk
   cannot rephrase them.
2. **Every number names its source** in the owner's words: "Price list, 2 Nov", "Stock: Sat 14
   Nov bake (live)", "Delivery calendar (live)". Never "estimated", never a number a model
   wrote.
3. **Say who decides and by when.** "Wanjiru confirms every order, usually within 2 hours
   (7:00–20:00)." Wait times come only from stated hours or records (R14).
4. **Say no plainly, then offer what exists.** "No. Kilele's menu has no gluten-free bake box.
   What Kilele does offer is…" (`concept/journeys/customer-order.html:155-159`).
5. **No openers, no sign-offs, no exclamation marks** in desk lines. Start with the answer.
6. **Desks speak for the business, never as the owner.** "Kilele's options", "Wanjiru confirms",
   never "I'll bake it".
7. **Never the platform's reason to visitors.** Resting, holds and suspensions use the visitor
   words in 06 §4.3; the owner sees the reason.
8. **Security lines are calm and specific.** What happened, what is already done, one next step
   (`concept/journeys/customer-security.html:94`). Nothing counts down.
9. **Money lines show amount, payee and rate before the provider opens.** "Pay USD 19.77 to
   Kilele Bakery (KSh 2,550 at 1 USD = KSh 129.00)."
10. **Sample data says "Sample"** and uses reserved phone ranges and `example.com` emails.
11. **Swahili is written by a person** and ships as an approved release.

---

## 7. Workstream tagging

| W | Owns for the website |
|---|---|
| **W1 WebAgent runtime** | Desk manifest and section desks (R4, U17); peeks from the manifest with live facts; Ask SSE frames (F1, F7), thread token (F2), handover between desks and to a person (U21); promise checker (R8, U20); citations with anchors (R7, U18); search answers and plain results (C-02, C-03); comparisons; `VisitorArtifact` store, routes, share tokens and states (R10, F4); presentation scripts and stage cues (R11, F5); device and account memory capabilities (R12, F6); the agent door: AI Catalog, MCP tools from the manifest, A2A card, Web Bot Auth tiers, agent terms page (M-01 to M-09); rate limits on `/ask` |
| **W2 Sample websites** | The three samples' content, seed files and fixtures (§1); page templates for K-01 to K-19, C-01 to C-10, N-01 to N-10, Z-01 to Z-04; help, delivery, allergens, terms and returns pages; the "Sample" band; the branded 404; the no-JS form paths; "what an AI agent sees" demo view |
| **W3 Control plane** | Sample hosts and TLS; platform hold, suspension and takedown pages with operator small print (X-02); routing of look-alike and payment-link reports to the operator (G-42); ring 0 canary scripts that drive the samples |
| **W4 Onboarding and owner journey** | The Move producing the sample from the fixture: desk drafts from the old menu, extracted tokens, migrated pages (K-19), `404.html`, robots and llms files; the owner's confirmation of facts that become the sample's corrected price list (§1.4); zavora.ai's "Assess my site" path |
| **W5 Owner console** | Decision cards for custom orders, plans, changes and refunds above the rule; the owner's admin for catalogue, price list, stock and bake plan, delivery calendar, orders and status updates, stories and corrections, subscribers, digests, presentation scripts (G-40, G-41); live thread takeover (G-37); teach your site |
| **W6 Identity, payments and trust** | Customer accounts on each site host (A-01 to A-22); consent receipts; sign-in history, alerts, sign out everywhere, step-up for changes; export and erasure with receipts; payments through the Effects Gateway (PayPal, M-Pesa, card), payment requests, voiding, receipts and Verify on the site domain; refund rules and the value-given cap; order lookup codes; privacy notice content |
| **W7 Design system and voice** | Both token layers and the extended `--wa-*` set; the component kit (§5.4, §5.5); the living mark generator; the words table and copy lint; accessibility statement; visual comparison pages; the copy fixes in §6 |
| **W8 Commerce rules and records** (proposed) | The deterministic layer every number comes from: catalogue and products, `ConfigurableProduct` with constraints and sketch parts, price rules, stock and bake plans, delivery zones, slots and capacity, quotes pinned to releases, baskets, orders and their state machine, holds, change and cancel rules, bookings, posted currency rates. W1 desks call it; W6 charges against it; W5 administers it. It doesn't fit W1 (agents never decide numbers) or W6 (money movement, not product data) |
| **W9 Customer messaging** (proposed) | Every message a customer gets outside the page: transactional email (G-11), WhatsApp templates and the provider (G-12), one-time codes, digest assembly and delivery with double opt-in and one-tap stop (R13, G-13), notices to the old email on account changes, deliverability and unsubscribe handling |

---

## 8. Decisions for James

| # | Decision | Recommendation |
|---|---|---|
| D1 | Kilele's canonical facts (A2 to A7) | Seed the sample from the journeys and 06 §6.3 values (§1.4); leave the fixture untouched as the "old site"; the difference becomes the owner's confirmed price-list change, with receipts |
| D2 | The news sample's brand (A1) | Tana Valley Times, built with The Kestrel Post's drawn patterns |
| D3 | Payment rails for the samples | PayPal sandbox (the fleet release) for all three; M-Pesa Daraja sandbox for Kilele and Classy if W6 has capacity; card later |
| D4 | News subscriptions | An entitlement granted by the owner or a one-time sandbox payment; recurring billing in the pilot |
| D5 | Classy Perfumes (A12) | Run the sample on a Zavora sample host with a fictional catalogue and a "Sample" band; keep `classyperfumes.shop` for the real store |
| D6 | Password reset without magic links (G-05) | An emailed one-time code plus the authenticator code; passkey users never need it |
| D7 | Minors and memory (G-46) | Offer account memory only after an "I'm 18 or over" confirmation recorded in the consent receipt |
| D8 | Agent terms (G-58) | Publish the human door page in the samples with "Sample terms, not legal advice"; real terms after legal review |
| D9 | Agent door tool names and tiers (A15, A16) | `catalog.search`, `delivery.slots`, `quote.create`, `checkout.create`, `request_custom_order`, `order.status`; reads anonymous, quotes and checkout links `known`, custom orders `known` and always held |
| D10 | Desk names (A8, A11) | Kilele: Bread, Cakes, Orders, Visit. Classy: Fragrances, Gifts, Orders, Journal |
