# 04 · UI parity: the build against the concepts

Mon 9 Nov 2026, against `platform` at 9d8b461. This refreshes the audit of Sun 8 Nov 19:47
(`platform` at 0e33f28). Every page was judged from its template and handler on `platform`. The
live site runs d524443, which is older, so agentrix.website and the three sample hosts were read
with GET only, as a check. Nothing was built, signed in, created or posted.

Decisions already taken are not counted as gaps: PayPal is the only payment rail (facts.md),
chat channels never approve (13 §3), and Telegram is the live chat channel while WhatsApp waits
behind a setting (13 §2). Owner social sign-in is left out by decision (requirements R2.4). The
home page's words are James's own (b4e141c), so §2 doesn't count them as faults; a promise on it
that the build can't keep is still counted (§3.2 item 5).

**Status.** Matches: the screen and its main controls are built, small differences only.
Partial: built, but a section, control or state the concept draws is missing. Missing: nothing
built for it. A page that now says honestly what isn't there yet is still Partial.

Paths in the table start under `crates/` unless they start with `apps/`. `os/` is
`agentrix-shell/templates/os/` and `onboarding/` is `agentrix-onboarding/templates/onboarding/`.

## 1. Concept screens

57 concept screens: **9 Matches, 43 Partial, 5 Missing**, the same totals as on 8 Nov. Three of
the five Missing are owner surfaces (terminal and desktop tray), one is the zavora.ai page, and
one is a concept map with no product page.

### Changed since 8 Nov

No row changed status. Much was built, but every Partial row still misses something the
concept draws. These rows lost one or more gaps:

- `console.html`: Orders and Customers apps; Edit, Decline and teach, and Allow, and tell me on
  v2 cards; AI spend monthly in the site's currency.
- `move.html` and `journeys/owner-golive.html`: the go-live card says what approving does and
  that leaving it never declines.
- `site.html`, `experience/nav-agents.html` and `journeys/customer-security.html`: a Privacy page
  on every sample, with a look-alike section; Yes or No on every desk answer.
- `journeys/customer-after.html`: customers can cancel an order. `experience/creator.html`: the
  desk draws the cake idea itself.
- `journeys/owner-plan.html` and `journeys/owner-billing.html`: 12 months at 20% off, and a
  switch between monthly and yearly.
- `journeys/owner-order.html`: real sites have an Orders page.
- `adk-surface/WebHome`, `WebToday`, `WebTrust` and `console/trust.html`: rules with Remove and
  standing grants with Stop.
- `adk-surface/Channel` and `console/phone.html`: this month's AI spend in the site's currency.
- `index.html`: Sign in opens the owner sign-in.
- `control-plane/index`, `fleet`, `incident`, `billing` and `portfolio`: an overview of the
  scopes, a holds list, Model Gateway health checks, Vultr spend per site, a ring card.

One row lost ground in use: `console/live.html`. No site's Ask records conversations yet, so
Live now hides itself on every site (235ded6).

| Concept | Built route · template | Status | What differs |
|---|---|---|---|
| `index.html` | `/` · agentrix-home `home.html`; promises and roster on `/how-it-works` · `how.html` | Partial | Different hero; a film and console shots replace the brief, sealed-room and decision collage. The seven promises and the WebAgent roster are only on How it works. Header Sign in now opens the owner sign-in (`agentrix-home/templates/base.html:38`) |
| `assess.html` | `/` check (`home.html`, `check.js`), `/start`, `/assess/{id}` · `onboarding/start.html`, `assess.html` | Partial | Paste, live reading, report with evidence, keep and share are built; the keep button says "Keep this site: create your account" (`onboarding/assess.html:109`). No "your new website, alive" preview (old and new side by side, match by area, 7-day link), no teach card, no "Meet your WebAgents". The concept stage designs three home pages, but only after an account and a verified claim (`agentrix-concepts/src/lib.rs:5`). Share sends the report, not a preview |
| `move.html` | `/console/move` · `onboarding/move.html`; `/console/{site}/quarantine` · `os/quarantine.html` | Partial | Stages, sealed rooms, left behind, match facts, go-live card and undo are built. The card now lists what approving does and says leaving it never declines (`onboarding/move.html:171-187`), and the chosen look shows (`:38`). No old/new page comparison, Keep/Release lives on its own page, refusals are only a count, no "31 old addresses redirected" |
| `console.html` | `/console/{site}` · `os/home.html`, `decisions.html`, `decision.html`, `webagents.html`, `website.html`, `trust.html`, `orders.html`, `customers.html` | Partial | Orders and Customers apps are built (`agentrix-shell/src/routes.rs:35-47`, `os/orders.html`, `os/customers.html`). Cards have Edit, Decline and teach, and Allow, and tell me (`os/decision.html:66-132`), and a taught rule reaches the WebAgent's prompts (`agentrix-kernel/src/taught.rs`). AI spend is this month's against a monthly budget in the site's currency (`agentrix-shell/src/budget.rs`, `os/home.html:109-116`). Still: the new verbs show only on v2 cards (`agentrix-shell/src/control.rs:505`), so held orders and the go-live card are Approve or Decline. Today has no visitor counts or "Next up". Teach your site has no answer box |
| `site.html` | sample `/` · kilele `home.html`, `layout.html`; `/privacy` · `kilele/privacy.html`; agent view at `/agents` · door `agents.html` | Partial | The footer links Privacy, and `/privacy` says what the site keeps (`agentrix-samples/src/common.rs:28`, `kilele/layout.html:22`). No Human/Agent toggle on the page (the agent view is `/agents` plus JSON). No EN/SW switch, no "Release · signed" line. The cake form is at `/cakes/studio` and `/order-a-cake` |
| `adk-surface/Main.dc.html` | web shell, Telegram bot, Flutter phone app, Talk (voice) | Partial | Terminal and desktop surfaces are missing. The shared rules hold: Esc never decides, chat only declines, voice allows an everyday card once or declines |
| `adk-surface/MainDark.dc.html` | same, Night Time theme | Partial | Same gaps as Main |
| `adk-surface/Channel.dc.html` | agentrix-channels Telegram bot; message chat blocks | Partial | Morning message (with this month's AI spend in the site's currency, `agentrix-channels/src/service.rs:579-597`), Decline, pause and STOP are built. Decline shows only on v2 cards (`agentrix-channels/src/bot.rs:464`), so a held order or the go-live card offers only "Open in the console". "Allow once" in chat is off by decision |
| `adk-surface/PhoneDecide.dc.html` | `apps/owner/lib/src/decisions.dart`, `/d/{card}` | Partial | Effect and fingerprint shown. A passkey approval opens the browser instead of signing in the app (`apps/owner/lib/src/decisions.dart:138`) |
| `adk-surface/PhoneInbox.dc.html` | `apps/owner` `now.dart`, `decisions.dart`; push "A decision is waiting" | Partial | No question cards from WebAgents; no decided list showing where each was decided |
| `adk-surface/Terminal.dc.html` | none | Missing | No terminal UI for owners |
| `adk-surface/TerminalLight.dc.html` | none | Missing | Same |
| `adk-surface/Tray.dc.html` | none | Missing | No desktop tray |
| `adk-surface/Voice.dc.html` | Talk (agentrix-media `voice.js`, `/api/voice/{site}/decide`), `/console/{site}/walk` | Partial | Allow once, deny and stop by voice, with a 3-second cancel. No camera or screen sharing |
| `adk-surface/WebHome.dc.html` | `/console/{site}` · `os/home.html`, dock, lane | Partial | Allow, and tell me for one person or one kind, and Edit, are built on v2 cards (`os/decision.html:68-69`). Edit shows only when a raiser marks fields, and no live raiser does (`raise_editable` is called only in `agentrix-shell/examples/shell-node.rs:107`). No thread with each WebAgent ("Message Finance"), no "Coming up" |
| `adk-surface/WebToday.dc.html` | "While you were away" · `os/home.html` | Partial | Spend (now this month's, in the site's currency), quiet hours, waiting count and a tagged timeline are built, plus "Done under your standing grants" with Stop (`os/home.html:61-71`). No action on each line (Retry, Do it, Not now), no "Routines today" |
| `adk-surface/WebTrust.dc.html` | `/console/{site}/memory` · `os/memory.html`; rules and grants on `os/webagent.html`, `os/trust.html` | Partial | Rules list with Remove and grants with Stop, on each WebAgent's page (`os/webagent.html:28-49`) and in the Trust centre (`os/trust.html:44-54`). What the WebAgents believe is still an empty state with no Confirm, Fix or Forget (`os/memory.html:10-14`). No Extend on a grant, no Check again, no device Remove or Add |
| `console/index.html` | `/os` → `/console/{site}` · `os/base.html` | Partial | Overview of nine ideas. Dock, worlds, Night Time, Freeze, signed decisions and gestures are built. The command bar only finds pages; it doesn't plan or hand out jobs (`os/base.html:97`). No mission reports, desk whispers or editing by voice |
| `console/shell.html` | `/console/{site}` · `os/base.html`, `os/home.html` | Partial | Dock, worlds, lane, Night Time, Freeze and the approval sheet match. Live leaves the dock and the phone tabs until a site's desks send it a conversation, and Orders takes its tab (`os/base.html:47-53`, `agentrix-shell/src/routes.rs:256`). No plan preview with Go, no Proposals in the lane, no "what visitors triggered" card, no site switcher. The Activity monitor shows this month's spend by job only (`os/base.html:110-121`) |
| `console/mission.html` | `/console/{site}/missions`, `/missions/{id}` · `os/missions.html`, `os/mission.html` | Partial | Say the goal, read the contract, approve; count, pace, pause, end. No weekly report, mission budget, mission cards, "Ask about this mission" or night log (the page says so) |
| `console/live.html` | `/console/{site}/live`, `/live/{thread}`, `/walk` · `os/live.html`, `live-thread.html`, `walk.html` | Partial | Sit in, take over and hand back are built, but no site's Ask records conversations yet, so Live says "Not connected yet" and stays out of the dock everywhere (`os/live.html:9-20`, `agentrix-shell/src/live.rs:354`). Connecting waits on new privacy wording (§3.2 item 3). No desk whispers, no "save this reply as an answer". The walk has no pointer, Editor draft or "Sign and publish" |
| `console/store.html` | `/console/{site}/store` · `os/store.html` | Partial | Listings, can/never, wakes, ladder and budget share; Add raises a card, and a WebAgent's page now removes it the same way (`os/webagent.html:50-64`). Packs and Connections are empty. No filter tabs or "why suggested". Budget share in %, not in the site's currency (`os/store.html:30`) |
| `console/trust.html` | `/console/{site}/trust` · `os/trust.html` (+ `/memory`, `/quarantine`) | Partial | Freeze, test-attack score, receipts and left behind are built, and standing grants list with Stop (`os/trust.html:44-54`). Missing on the page: protections summary, platform notices, support access list (reachable only from the email), export and erasure, devices |
| `console/phone.html` | `apps/owner` (Now, Decisions, Live, Apps, Pair) and shell phone tabs under 900 px | Partial | Now says this month's AI spend in the site's currency (`apps/owner/lib/src/now.dart:57`, `:119`). Passkey approval still leaves the app for the browser. Live shows WebAgent activity, not visitor conversations. No mission tile on Now. In the web shell's phone tabs, Orders stands in for Live until Live is connected |
| `console/gestures.html` | `/console/{site}/gestures` · `os/gestures.html` | Matches | Six held gestures, camera opt-in, hold pad, palm countdown, keys. Only the demo cards differ |
| `control-plane/index.html` | ops host `/` · operator `overview.html`; `/holds` · `holds.html`; scopes in `layout.html` | Partial | An overview at `/` shows the operator scope and the partners' portfolios with real counts, money and AI spend, and what waits for you (`agentrix-operator/src/overview.rs:24-26`, `templates/overview.html:13-75`); `/holds` lists holds in force and lifted (`agentrix-operator/src/pages.rs:509-568`). The site (owner) scope is only a footnote. Every operator WebAgent still says "not running yet" (`agentrix-operator/templates/layout.html:73-76`), so none proposes or acts |
| `control-plane/fleet.html` | `/fleet` · operator `fleet.html` | Partial | Brief from counts, KPIs, one cell, runner pool, Migration Factory, SLO table, model routes, spend, ledger. The SLO table now has a Model Gateway row from its per-minute health checks (`agentrix-operator/templates/fleet.html:87-88`). Edge availability and Ask first-token p95 still read "Not measured" and "Not recorded" (`:32-34`). One cell, not four. No operator WebAgent runs, so no activity list and no Capacity, Release or Billing cards |
| `control-plane/rollout.html` | `/rollouts` · `rollouts.html` | Partial | Rings sized from live sites, the gate list, model-route artefacts and releases signed per day. No train has run: no waves, no canary against control, no train timeline, no two-person promotion. Every gate but the test attacks says "not recorded". Only words changed since 8 Nov |
| `control-plane/incident.html` | `/incidents`, `/incidents/{id}` · `incidents.html`, `incident.html` | Partial | Declare, timeline, evidence, drafts, status and postmortem are built, and the forms keep typed text after a refusal. A new card shows the Model Gateway's hourly health checks around the incident (`agentrix-operator/templates/incident.html:24-39`), not the concept's latency chart with its SLO line. No SRE WebAgent, so no lever pulled within a grant and no "Undo the cap". Drafts are copied by hand; there is no status page |
| `control-plane/billing.html` | `/money` · `money.html` | Partial | KPIs, reconciliation table, PayPal rail, reminders, MRR by plan, plan catalogue and the invoice-run card. The table now shows what the gateway says it sent to Vultr for each site (`agentrix-operator/templates/money.html:29-39`). Vultr's own usage still isn't imported, so there is no variance column and nothing reconciles or is held. No two-signer payouts card |
| `control-plane/portfolio.html` | `/portfolio`, `/partners/{id}` · `portfolio.html` | Partial | Brief, clients with role and ring, invite (operators only), Moves, people, brand, authority, plus a "Your ring" card and "AI above the plan" in the margin card (`agentrix-operator/templates/portfolio.html:78-87`). Dock Rollouts now goes to `#ring`; Sites still goes to `#import` (`templates/layout.html:49`). No bulk import, no client health, no margin total. The Approver lane is hard-coded empty (`agentrix-operator/src/partners.rs:725-726`) |
| `control-plane/cli.html` | `/cli` · `cli.html` | Partial | Sessions from the ledger (none yet), pack rules and the can/never list. No session replay, no guard blocking a publish, no pack has shipped |
| `experience/index.html` | platform `/samples` · agentrix-home `samples.html` | Partial | Three samples and films exist. No archetype and tier picker. Kestrel Post became Tana Valley Times; zavora.ai is not built |
| `experience/nav-agents.html` | every sample page · kilele `layout.html`, `macros.html`, themes `desks.js` | Partial | Peeks with live facts, Instant starters, team sheet, ask bar, thread rail and person chip match. Each answer now asks "Was this right? Yes / No", a No can carry a note, and the owner sees the marks (`agentrix-themes/static/desks.js:104-171`, `agentrix-samples/templates/owner/questions.html:19-27`); the footer links Privacy. No "Remember me on this device" in the thread, no "Release · signed" line |
| `experience/creator.html` | Kilele `/cakes/studio` · `kilele/studio.html` | Partial | Options, price lines, sketch, calendar, request and no-JS fallback match, and the desk now draws the idea itself from plain defaults (`agentrix-themes/static/studio.js:215`, `agentrix-samples/src/studio.rs:452`). "Save and share" only copies a link: no share sheet, no 30-day expiry, no "Play Amina's 7th" |
| `experience/artifact.html` | Classy `/gifts/plan` · `classy/plan.html` | Partial | The creator's plan works (budget bar, swap, add all to bag). No share sheet, invited view, owner view, `/a/{token}` or expiry; the plan lives only in the URL |
| `experience/search.html` | Classy `/search` · `classy/search.html`, `search_body.html` | Matches | Reading chips, refine, Answer/Plain toggle, compare and "What I used" are there |
| `experience/news.html` | Tana `/`, `/stories/{topic}`, `/digest`, `/corrections`, `/privacy` · `tana/*.html` | Matches | Front page, catch me up, story so far with its correction, Friday five and house rules. A local weekly instead of a national paper, as planned |
| `experience/presentation.html` | none | Missing | No zavora.ai page, no guided tour, no `/book` |
| `identity/marks.html` | themes `marks.js`, agentrix-marks, `macros.html` `mark()`, console Appearance | Partial | Living marks with states render on the desks. SVG only: no PNG for chat avatars or the A2A icon |
| `journeys/index.html` | none | Missing | A map of the two journeys for the concept set. No product page needs it |
| `journeys/customer-browse.html` | Kilele `/` · `kilele/home.html`, `layout.html` | Matches | No pop-up, the AI chip is shown, visit counting with "Don't count my visits" and GPC |
| `journeys/customer-order.html` | `/bag`, `/checkout` · `shop/bag.html`, `checkout.html`; `/cakes/studio` | Matches | Source chips per line, live slot calendar, pinned rate, held cake request (checkout judged from the template; it needs sign-in) |
| `journeys/customer-pay.html` | `/orders/{id}`, `/orders/{id}/pay` · `shop/order.html`, `pay.html` | Matches | Amount, payee and rate first, then PayPal. The M-Pesa and Card buttons are left out by decision |
| `journeys/customer-after.html` | `/orders/{id}/receipt`, `/change`, `/help`, `/cancel` · `shop/receipt.html`, `change.html`, `help.html`, `order.html` | Partial | Receipt, status steps, change with cut-off and refund under the rule match. A customer can now cancel: an unpaid order ends at once, and a paid one goes to the owner to decide (`agentrix-samples/templates/shop/order.html:25-30`, `agentrix-samples/src/commerce.rs:1982`). Sample receipts are unsigned, so no "Verify this receipt"; no photo in "Something's wrong"; no order updates by chat |
| `journeys/customer-agent.html` | `/agents`, `/mcp`, `/.well-known/ai-catalog.json`, `/.well-known/awp` · `door/agents.html` | Partial | Tiers, terms, checkout link and held request match. Web Bot Auth isn't verified (a self-named agent counts as known). No signed A2A card, no `/a2a`, no "what the door sees" panel |
| `journeys/customer-account.html` | `/account/sign-in`, `/create`, `/keeps`, `/recovery-codes`, `/passkey-offer`, `/choices` · identity `visitor/*.html` | Partial | Passkey or email code only: no password, no social sign-in, no authenticator step. The "your bag is waiting" note shows only after a session expires |
| `journeys/customer-security.html` | `/account/security`, `/data`, `/erased` · `visitor/security.html`, `data.html`, `erased.html`; `/privacy#report` on each sample | Partial | New sign-in alert, sign out everywhere, fresh sign-in gate, export and erasure match. Every sample's footer now links a look-alike section (`agentrix-samples/templates/kilele/layout.html:21`, `kilele/privacy.html:90-97`), but the report is an email link to Agentrix, not the concept's form that reaches the owner and Agentrix, and nothing is recorded |
| `journeys/owner-discover.html` | `/` check · `home.html`; `/assess/{id}` · `onboarding/assess.html` | Partial | No account needed; paste, report and a talk box work. No preview with a match score, no "Want to teach it?", no first WebAgent suggestions, no "Share the preview". The concept stage's looks need an account and a verified claim |
| `journeys/owner-account.html` | `/account/new` · `onboarding/account.html`; `/setup` · `setup.html` | Matches | Passkey first, password plus a required authenticator, recovery codes, the Chief's "Karibu". Adds email-code sign-up. Social sign-in left out by decision |
| `journeys/owner-plan.html` | `/setup/plan` · `plan.html`; `/setup/pay` · `pay.html` | Partial | Plans, AI budget slider, a suggestion, PayPal on its own page, and now monthly or 12 months at the catalogue's discount (`onboarding/plan.html:55-58`, `onboarding/pay.html`). No "what it costs us" estimate. The budget is set in USD here (`onboarding/plan.html:49`), while the console shows it in the site's currency |
| `journeys/owner-domain.html` | `/sites/{id}/claim` · `onboarding/claim.html` | Matches | DNS or file tabs, Check now, found or not found in words; once found, it leads to the concept stage (`onboarding/claim.html:28-29`). No registrar-aware help |
| `journeys/owner-move.html` | `/console/move` · `onboarding/move.html` | Matches | Same facts as the concept, drawn as room cards instead of a timed log |
| `journeys/owner-golive.html` | `/console/move` · `move.html`; approval sheet on the decision page | Partial | Card, approve with passkey or code, live state, undo. The card now lists what approving does: the address record, which WebAgents start, the first charge (`onboarding/move.html:171-177`, `os/decision.html:26-32`), and says leaving it never declines (`move.html:187`, `decision.html:133`). Unlike the concept, going live doesn't move the domain, and no page says which record to set. No receipt line when live; two undo buttons (`move.html:197-198`); "Not yet" is a sentence, not a button |
| `journeys/owner-day1.html` | Night Time and brief card · `os/home.html`; `messages/owner/morning_brief.txt` | Partial | Night timeline and a brief built from receipts, with this month's AI spend in the site's currency. No visits in the brief or on Back office, though moved sites now count them (02c98c3). No "Share: my website did this overnight" |
| `journeys/owner-order.html` | `/console/{site}/orders`, `/orders/{id}` · `os/orders.html`, `os/order.html`; sample `/samples/{site}/custom` · `owner/custom.html`; `os/decision.html` | Partial | Real sites now have Orders and Customers; orders come from customers' AI agents at the agent door (`os/orders.html:6-7`). The held order is a first-release card: Approve or Decline, no Edit. No books or payout, no refund button (`os/order.html:63`), no refund-above-the-rule card. The sample back office still has payments and refunds |
| `journeys/owner-billing.html` | `/billing` · `onboarding/billing.html` | Partial | AI budget meter, plan list, cancel with passkey or code, and now how often you pay, with a switch at the next renewal through PayPal (`onboarding/billing.html:33-48`). No invoices (`:55`), no plan change (`:70`), no cancel timeline, no export bundle (`:95`). The budget shows in USD here |

Not screens, so not in the table: `styles.css`, the `.css`, `.js`, `.svg` and `.json` support
files, `adk-surface/README.md` and `experience/img/`.

## 2. Words

Re-checked briefly on 9 Nov. The new screens (Orders, Customers, the decision card's Edit, Decline
and Allow boxes, billing's period, the control plane's forms) read plainly. James put back his
own words on the home page (b4e141c, Sun 8 Nov 19:48), so the home rows below are kept by his
decision, not open.

### 2.1 Fixed on 8 Nov

89 lines in 49 files, committed on `w0/ui-parity` and merged in 414b383. Line numbers are as of
8 Nov. b4e141c reverted every `crates/agentrix-home` row in the first table; the rest stand.

**Platform home, samples and prices** (`crates/agentrix-home`)

| File:line | Was | Now |
|---|---|---|
| `templates/home.html:24` | Turn your WordPress, Shopify or React website into an autonomous AI business with WebAgents… We don't just remake your website. We rebuild it from the ground up… | We move your WordPress, Shopify or React site to Agentrix and give it WebAgents. They answer customers from your own pages, take orders and bookings, and ask you before anything that costs money… |
| `templates/home.html:5`, `:10` | We rebuild your WordPress site on a secure platform hosted on Vultr, even if it has security issues and vulnerabilities. Your new Agentrix Website serves customers who come from… | We move your WordPress site to a secure platform on Vultr and leave any hack behind. The new site answers your customers, and the AI assistants and agents they send. |
| `templates/home.html:43` | See how a bakery transformed its business into an Agentrix Website. | See a bakery's Saturday, before and after its site moved to Agentrix. |
| `templates/home.html:61` | A busy shop, and an idle, passive website. | A busy shop, and a website nobody answers. |
| `templates/home.html:63` | The website is now an autonomous AI business that works for you, handling orders, talking with customers and supporting them. | The site answers customers and takes orders, and asks Wanjiru before anything that needs her. |
| `templates/home.html:70` | Agentrix WebAgent showcase websites | Samples |
| `templates/home.html:88` | See the samples and films for a shop, a paper and a salon | See all three samples, and films about a bakery, a perfume shop, a paper and a salon |
| `templates/yoursite.html:13` | Show me how I can transform my site | What your site becomes with WebAgents |
| `templates/yoursite.html:14` | Your website can become an autonomous AI business… | WebAgents answer your customers, take orders and look after the site. You approve anything that costs money or makes a promise. |
| `templates/prices.html:12` | Priced for Intelligence | Plans and prices |
| `templates/prices.html:13`, `:41` | Lead said "monthly"; the FAQ offered 12 months at 20% off | Both say monthly or for 12 months |
| `data/samples.json:9` | each menu item is a WebAgent: … a cake designed with you | each part of the menu has its own desk: today's bread, a cake you design with it, and orders Wanjiru confirms |
| `templates/how.html:59` | Dials, not blind trust | A dial for each job |
| `templates/how.html:124` | a gVisor room: read-only, not root, with no capabilities | its own gVisor container: read-only, with no admin rights |
| `templates/how.html:125` | The proxy refuses the cloud's metadata address. | …its proxy blocks the cloud's internal address for machine secrets. |
| `templates/how.html:176` | Its challenge commits to the card's fingerprint: the release's digest and the site's charter. | Your passkey signs a fingerprint of exactly this card: this release and your site's rules. |
| `templates/how.html:346` | PayPal through the Effects Gateway. Every payment is an approved effect with a receipt. | PayPal, through one gateway that holds the keys. Every payment is approved first and gets a receipt. |
| `templates/privacy.html:34` | …is on the home page | …is on the How it works page (it was the wrong page) |

**Sample sites** (`crates/agentrix-samples/templates`)

| File:line | Was | Now |
|---|---|---|
| `base.html:21` | The business, people and orders are fictional, and payments run in PayPal's sandbox. | The business and its people are invented, and payments run in PayPal's sandbox, so no real money moves. |
| `classy/layout.html:59` | It never guesses prices, dates or allergens. (a perfume shop) | It never guesses prices, stock or delivery dates. |
| `classy/layout.html:63`, `tana/layout.html:59` | Classy Perfumes's WebAgents; Tana Valley Times's desks | The WebAgents at Classy Perfumes; The desks at Tana Valley Times |
| `kilele/cakes.html:50` | The desks won't promise these, so you won't be surprised later. | The desks won't take an order for these. |
| `kilele/delivery.html:18` | Somewhere else? The site lists only these areas, so the desks won't promise others. | Delivery covers only these areas. Somewhere else? Ask Wanjiru. |
| `kilele/notfound.html:7`, `classy/not_found.html:6` | Nothing lives at /x. | There's no page at /x. |
| `classy/search_body.html:19` | static index, no AI | plain search, no AI |
| `classy/search_body.html:74` | Results re-rank from the catalogue, without a model. | Tap a part to drop it, and the list re-sorts from the catalogue without AI. |
| `classy/search_body.html:75` | No budget, family, size or product found in the words. Results match the words themselves. / Ask something and the parts of your question appear here. | No budget, family, size or product in your words, so the results match the words as typed. / Type a request, such as a budget or a scent family, and its parts show here. |
| `shop/orders.html:13` | For your privacy I've shown only the status. This order was placed with another account. | This page shows only the status, to keep the order private. This order belongs to another account. |
| `shop/checkout.html:54` | Only real times can be picked. | Only open times are listed. |
| `shop/receipt.html:25` | signed by the gateway that took it | signed by the payment service that took it |
| `shop/help.html:21` | The Cashier works behind the scenes. | You don't talk to the Cashier yourself. |
| `tana/macros.html:22` | Double opt-in | We email you a link to confirm first |
| `tana/subscribe.html:37` | Subscribers already: sign in and subscriber stories open. | Already a subscriber? Sign in and subscriber stories open. |

**Sign-up, sign-in, account and billing**

| File:line | Was | Now |
|---|---|---|
| `crates/agentrix-console/templates/auth/signin.html:35` | No magic links and no SMS codes: | We never sign you in from an email link or a text message: |
| `crates/agentrix-identity/templates/visitor/keeps.html:6` | your recovery codes as one-way hashes | your recovery codes, stored so nobody can read them back |
| `crates/agentrix-identity/templates/visitor/security.html:27` | Ends every session, this device included. | Signs you out on every device, this one too. |
| `crates/agentrix-identity/templates/visitor/erased.html:4` | Every store has answered. / A store couldn't finish | Every record below is cleared. / A record marked Open below isn't cleared yet |
| `crates/agentrix-onboarding/templates/onboarding/plan.html:19` | {host} · site in setup | Choose a plan for {host} |
| `crates/agentrix-onboarding/templates/onboarding/billing.html:33` | Counted by the Model Gateway on every call… it's a limit, not a meter. | Counted on every AI call… it's a hard limit. |
| `crates/agentrix-onboarding/templates/onboarding/billing.html:50` | Changing plan from here comes next. | You can't change plan here yet, so your plan stays as it is. |
| `crates/agentrix-onboarding/templates/onboarding/billing.html:75` | The one-file export… comes next. Today every receipt… | A one-file export… isn't ready yet. For now, every receipt… |
| `crates/agentrix-billing/data/plans.json:72` | Agent-to-agent commerce | Orders from customers' AI agents |

**Console: Today** (`crates/agentrix-shell/templates/os`, `crates/agentrix-console/templates/console`)

| File:line | Was | Now |
|---|---|---|
| `os/home.html:79` | Visitors passed these to you from Ask | Visitors asked these, and your pages don't say |
| `os/home.html:91` | planted attacks held against this deployment | test attacks were stopped on your site |
| `os/home.html:94` | The containment gauntlet hasn't run against this deployment yet | The test attacks haven't been run against your site yet |
| `os/home.html:103` | By job (button) | Spend by job |
| `os/home.html:128` | Decisions never expire into a yes | Nothing is approved without you |
| `os/home.html:146` | wants you (chip) | needs your OK |
| `os/home.html:161` | …once presence is switched on for your site | Live conversations will show here too, once that's switched on for your site. |
| `os/base.html:106` | from the Model Gateway's records | from the spending records |
| `os/base.html:114`, `console/today.html:51` | Every call goes through the Model Gateway, which stops at the limit before asking a model. | Every AI call is checked against your limit first, and refused once you reach it. |
| `console/today.html:49` | Model spend | AI spend |

**Console: Decisions and the approval sheet**

| File:line | Was | Now |
|---|---|---|
| `os/decision.html:28`, `console/decision.html:31` | These starting levels are written into your charter and signed with this card. Signing them unchanged is safe… | Approving this card also sets these starting levels… |
| `os/decision.html:42`, `console/decision.html:47` | The card and your approval come from the same arguments that make this fingerprint. Your passkey signs it, or a single-use authenticator code is bound to it, and it runs once. | This fingerprint stands for exactly what will run. The card you read and the approval you give are both made from it, so what you approve is what runs, once. |
| `os/decision.html:57`, `console/decision.html:61`, `os/base.html:131` | Single use, and bound to this exact card (effect). | It works once, and only for this card (approval). |

**Other owner screens and messages**

| File:line | Was | Now |
|---|---|---|
| `crates/agentrix-shell/templates/os/trust.html:22`, `crates/agentrix-console/templates/console/trust.html:20` | Ten attacks planted in Zavora's fixture, run against this deployment… The score is computed from receipts only. | Ten test attacks, hidden in a copy of an infected site we keep for testing, run against your site… The score counts only what the receipts show. |
| `os/trust.html:28`, `console/trust.html:25` | The gauntlet hasn't run against this deployment yet | The test attacks haven't been run against your site yet |
| `crates/agentrix-shell/templates/os/live-thread.html:35` | What you write here can become an answer for everyone… Teach your site (link to `#teach`, which doesn't exist) | What you write here reaches this visitor only. If the desk didn't know, add the answer to your site so it can answer the next person; open questions are listed on Website (link to `#teach-h2`) |
| `crates/agentrix-console/templates/console/webagents.html:12` | Read-only this weekend. | You can't change levels on this page. |
| `crates/agentrix-console/templates/console/receipt.html:26` | checks the Ed25519 signature here, without trusting this page | checks the signature in your browser, so you don't have to take this page's word for it |
| `crates/agentrix-samples/templates/owner/custom.html:4` | No model prices anything. | AI never sets a price. |
| `crates/agentrix-messages/templates/owner/weekly_review.txt:15` | Your WebAgents earned these. Each one is a suggestion; nothing changes until you sign it | Your WebAgents could do more without asking. Each is only a suggestion; nothing changes until you approve it in the console |
| `crates/agentrix-ui/words/en.json:69` | Unfreeze needs your passkey. | Unfreezing needs your passkey or an authenticator code. |

**Operator** (`crates/agentrix-operator/templates`)

| File:line | Was | Now |
|---|---|---|
| `app.html:7`, `:16`, `:19`, `:22`; `rollouts.html:25`; `customers.html:39` | Spec numbers on screen: "(08 §3.5)", "(08 §4.2)", "(08 §2.2, §4.4)", "(08 §2.4)", "Stages follow 08 §4.1" | Removed, or "Each stage comes from what the records show." |
| `incident.html:29`, `trust.html:30`, `cli.html:36`, `money.html:55` | "08 §3.7", "08 §2.4", "08 §6.4", "policy: 08 §4.2" as captions | set by each one's grant; the same for every operator; the same for every coding agent; nothing to chase |
| `fleet.html:49` | …come with the fleet profile (08 §3.2). | …come when the platform runs in more than one region. |
| `fleet.html:55` | They start with the operator tenant (08 §5). | They start when Zavora's operator tenant is set up. |
| `fleet.html:80`, `money.html:9` | targets are hypotheses…; still hypotheses until James sets them | targets are guesses…; drafts until James sets them |
| `signin.html:28` | Continue (button) | Sign in |

### 2.2 Still to fix on 8 Nov, re-checked

Of the 23 rows, 18 are done, 3 are kept by James's decision, 1 is partly done and 1 is open.

| File:line on 8 Nov | As built then | Now |
|---|---|---|
| `agentrix-node/src/pages/map.rs:804-840`; `apps/owner/lib/src/now.dart:58` | $0.04 of your $1.00 daily limit; USD 0.04 | Done: this month against a monthly budget in the site's currency (7a5829b, 82ce31f; `map.rs:798-837`, `now.dart:119`) |
| `agentrix-shell/src/routes.rs:2405`, `:2424`, `:2494` | Something went wrong on our side… | Done: "We couldn't finish that on our side. Your site stays frozen. Try again in a minute." (7be3927; `routes.rs:2840`, `:2859`, `:2929`) |
| `agentrix-home/templates/home.html:52` | How an Agentrix Website can transform your business | Kept: James's words (b4e141c) |
| `agentrix-home/src/desks.rs:58`; `static/guide.js:247` | Show me how I can transform my site | Kept: James's words (b4e141c) |
| `agentrix-home/static/guide.js:39` | each menu item is a WebAgent | Kept: it matches James's line in `data/samples.json:9` (b4e141c) |
| `agentrix-home/src/view.rs:579-581`, `:622` | Set checks · AI answers; Set tests | Done: Rules · AI answers; Rules (`view.rs:583`, `:585`, `:626`) |
| `agentrix-home/src/view.rs:553`, `:559` | No checks on this server yet… | Done: "No sites checked yet." and "The Guardian hasn't attacked this site yet." (`view.rs:557`, `:563`) |
| `agentrix-home/src/view.rs` (stats) | 1 signed receipts issued | Done: singular and plural (`view.rs:933`) |
| `agentrix-onboarding/templates/onboarding/start.html:20`, `:36` | Show me | Done: Check my site (992f301) |
| `agentrix-onboarding/templates/onboarding/assess.html:109` | Continue with passkey | Done: Keep this site: create your account (992f301) |
| `onboarding/plan.html:48` with `static/plan.js:16` | Nothing above it is ever billed. | Done in both (992f301; `plan.html:52`, `plan.js:18`) |
| `agentrix-identity/templates/visitor/sign-in.html:3`, `signed-out.html:3` | basket | Done: bag (992f301) |
| `agentrix-shell/templates/os/trust.html:20` | The containment gauntlet | Done: Test attacks |
| `agentrix-samples/templates/classy/search_body.html:85` | Model calls | Done: AI calls (992f301) |
| `agentrix-samples/src/owner.rs:1347` | PayPal (sandbox) | Done: PayPal test mode (992f301; `owner.rs:1385`) |
| `agentrix-samples/src/owner.rs:1717` | Today's bake, {} · left is the plan less… | Done (992f301; `owner.rs:1862`) |
| `agentrix-samples/src/news.rs:797` | Double opt-in, one-tap stop. | Done: "We email you a link to confirm first. One tap stops it." (992f301) |
| `fixtures/samples/tools/tana.py:93`; `apps/customer/lib/src/flavour.dart:71` | Tana Valley Times's | Done (992f301, da13e2b) |
| `fixtures/samples/tools/kilele.py` | Visit desk: Hours, directions… | Done: "I know the hours, directions, parking and where Kilele delivers." (da13e2b) |
| `agentrix-messages/templates/owner/*.txt` | Buttons to `/os/review`, `/os/webagents/budget` and other pages that don't exist | Done: `console_url` prints as it is, deeper pages get their own key, and `agentrix-messages/src/links.rs` tests every link (b02f889) |
| `agentrix-messages/templates/owner/decision_waiting.txt:2`, `decision_expiring.txt:2` | push link `/console/decisions` | Done: the card's own link; the old route answers with a 308 to `/os/decisions` (b02f889, 49fc9f0) |
| `agentrix-home/templates/how.html:245-253` | The old five-tab console | Partly: 4396475 drew today's four phone tabs. The tab list still says "the day's AI spend" (`how.html:249`) and "the gauntlet score" (`:252`), and the console shot's alt text says "the day's AI spend" (`:257`). The budget is monthly; James's page, so his call |
| `agentrix-onboarding/templates/onboarding/move.html:28` | {host} on WordPress | Open (`move.html:31`), with §3.2 item 5 |

### 2.3 New since 8 Nov

| File:line | As built | Rewrite | Why |
|---|---|---|---|
| `agentrix-samples/templates/kilele/privacy.html:43`, `classy/privacy.html:33`, `tana/privacy.html:37` | The site doesn't save what you ask them | Say that a Yes or No mark keeps the question and the desk's answer for the owner, and for how long | Untrue since d524443 (`agentrix-samples/src/ask.rs:1716`); §3.2 item 3. Needs James |
| `agentrix-concepts/templates/concepts/page.html:41`, `:53` | The Move rebuilds your site in the look you choose | The Move rebuilds your site with this look's colours and fonts (as `:68` and `onboarding/move.html:38` already say) | Promises more than the Move does; §3.2 item 9 |
| `agentrix-concepts/templates/concepts/page.html:131-136`, `evidence.html` | "How this meets Vultr Agent Rush · Challenge 1", with sha256 chips, on the owner's page | Move it to the judges' README or an operator page | A baker choosing a look doesn't need the hackathon's checklist |
| `agentrix-concepts/templates/concepts/page.html:43`, `:49-50`, `:81`, `:91-95` | GLM 5.3, DeepSeek v4.1 Flash, "sha256 …", "N writes", "fs_write refused ×2" | A planner picks three directions; three designers, one room each. Keep the chips for operators | Model and tool names on an owner screen |
| `agentrix-concepts/templates/concepts/page.html:17` | Console link to `/console/today` | `/os` | The first-release route; it now only redirects |
| `onboarding/plan.html:49`; `agentrix-onboarding/src/billing.rs:300` | $30 a month: your AI budget | The same budget in the site's currency, as the console says it | facts.md: set and shown in the site's currency |

## 3. Fix first

### 3.1 The 8 Nov list, re-checked

1. **No preview before sign-up.** Open. The concept stage (merged in 02904ab) designs three looks,
   but only after an account and a verified claim. Now §3.2 item 4.
2. **Two consoles.** Done in 49fc9f0, merged in 4c41b8a: the first release's owner pages answer
   with a 308 to `/os` (`crates/agentrix-node/src/pages/mod.rs:67-68`). The concept stage's header
   still links `/console/today`, which redirects (§2.3).
3. **Owner emails link to pages that don't exist.** Done in b02f889, merged in 656c471:
   `crates/agentrix-messages/src/links.rs` lists the pages a message may link to, and its tests
   check every link.
4. **AI spend shown as a USD daily limit.** Done in 7a5829b (ae174ba) and 82ce31f (b6a2f3b): the
   console, the brief, the Chief, Telegram and the owner app say this month's spend against a
   monthly budget in the site's currency. Setup and Billing still say dollars (§3.2 item 8).
5. **Home "Sign in" opens the old `/auth/signin`.** Done in d925bb7, merged in d0c1892
   (`crates/agentrix-home/templates/base.html:38`).
6. **The go-live card doesn't say what approving does.** Done in 3f90fd4, merged in 4c41b8a
   (`onboarding/move.html:171-187`, `os/decision.html:26-33`, `:133`). "Not yet" is a sentence
   rather than a button, and the live card still has two undo buttons.
7. **Cards are Approve or Decline only.** Partly done in db58503, e894240 and 7e8694b (merged in
   38dfdd9), with taught rules reaching the prompts in 40cb9c3 (0dce0e0). First-release cards
   (held orders, go-live, restore) still have Approve or Decline only, and no live raiser marks
   editable fields. Now §3.2 item 6.
8. **No privacy page or look-alike report on the samples.** Done in d0f68f1, merged in 656c471.
   The report is an email link, and the page's line about questions is no longer true (§3.2
   item 3).
9. **Promises the build can't keep.** Half done. 12 months at 20% off runs from `/prices` to
   PayPal (a5bc9fd, 99fdcaf, 2b5602e; merged in 724e19a). The home page still offers Shopify and
   React moves, and James put those words back (b4e141c), so the fix moves into the flow (§3.2
   item 5).
10. **Real owners have no Orders or Customers pages.** Done in 440e4f5 and 86dcd86, merged in
    e83b3fe. Orders come only from the agent door, and there's no refund button yet.

From the 8 Nov "next after these": Teach your site still has no answer box (§3.2 item 7); gift
plans and cake designs still can't be shared with an expiry; Billing still lists no invoices
(§3.2 item 8).

### 3.2 The ten gaps that hurt most now

Ranked by harm to a real owner, customer or operator today.

1. **Going live doesn't move the owner's domain, and no page says how to.** The card says the
   owner's address "keeps showing your old site until you point it here"
   (`onboarding/move.html:174`), but no page gives the record to set, so customers keep landing
   on the old site. Change the live card in
   `crates/agentrix-onboarding/templates/onboarding/move.html:190-199` and
   `crates/agentrix-shell/templates/os/website.html`. Done: both show the exact record for the
   owner's domain (type, name, value) and a Check now button, and say plainly whether the domain
   still shows the old site or now points at Agentrix.
2. **An owner can't sign out a device they don't recognise.** The new-device email says "The
   console can't show your devices or sign them out yet" and sends them to WhatsApp
   (`crates/agentrix-messages/templates/owner/new_device.txt:11`); owner sign-in has no session
   routes (`crates/agentrix-identity/src/owner/routes.rs:20-27`). Done: the Trust centre lists the
   owner's signed-in devices with Sign out and Sign out everywhere, as visitors already have at
   `/account/security`, and the email links there.
3. **The samples' privacy pages say something untrue, and Live waits on fixing them.** They say
   the site doesn't save what you ask the desks (`crates/agentrix-samples/templates/kilele/privacy.html:43`,
   `classy/privacy.html:33`, `tana/privacy.html:37`), but a Yes or No mark stores the question
   and the answer for the owner (`crates/agentrix-samples/src/ask.rs:1716`). Live stays hidden
   on every site until the desks record conversations, and that needs new wording first
   (235ded6). Done: James approves wording that says what is kept (marks, passed questions, Live
   conversations) and for how long; the Ask records turns with a retention sweep; Live comes back
   to the dock (`crates/agentrix-shell/src/live.rs:354`).
4. **Nothing of the new site shows before an account and a verified claim.** The Move and the
   concept stage both wait for DNS or file proof (`crates/agentrix-node/src/claim/mod.rs:3-4`,
   `crates/agentrix-concepts/src/lib.rs:5`). The concept shows the rebuild from the public crawl,
   with no account. Change
   `crates/agentrix-onboarding/templates/onboarding/assess.html`, with a preview route in
   `crates/agentrix-node/src/assess/mod.rs`. Done: the health report shows the rebuilt home page
   beside the old one, with a match score and a private 7-day link that is never indexed, before
   "Keep this site".
5. **A Shopify or React owner signs up for a Move that can't take their site.** The home page
   offers both (`crates/agentrix-home/templates/home.html:24`, James's words); the Move takes a
   WordPress export only (`onboarding/move.html:31`, `:48`). Change the keep section of
   `crates/agentrix-onboarding/templates/onboarding/assess.html`. Done: the health report names the
   platform it found, and for anything but WordPress says, before "Keep this site", that the Move
   can't take it yet. Or James changes the home line.
6. **Held orders, the cards owners see most, still have only Approve and Decline.** They are
   first-release cards, and the new verbs show only on v2 cards
   (`crates/agentrix-shell/src/control.rs:505`); chat can't decline them either
   (`crates/agentrix-channels/src/bot.rs:464`). No live raiser marks editable fields
   (`raise_editable` is called only in `crates/agentrix-shell/examples/shell-node.rs:107`).
   Change `HeldOrders::raise` in `crates/agentrix-node/src/decisions/mod.rs:783` to raise a v2
   card with the date and amount editable. Done: a held agent-door order shows Edit, Decline with
   a reason, and Allow, and tell me, and chat offers Decline.
7. **Owners can't answer the questions their site passes them.** Website and Back office list
   them with no answer box (`crates/agentrix-shell/templates/os/website.html:32-40`,
   `os/home.html:89-97`). The sample back office has one, and its answers reach the desks
   (`crates/agentrix-samples/templates/owner/questions.html:10-14`, d524443). Done: each question
   has "Your answer"; a saved answer reaches the Concierge and the desks for the next visitor,
   with a receipt, and the question leaves the list.
8. **Billing can't change a plan, list invoices or export, and says the budget in dollars.** "To
   change it now, cancel" (`crates/agentrix-onboarding/templates/onboarding/billing.html:70`); no
   invoices (`:55`); no export (`:95`); the budget is in dollars here and on setup
   (`crates/agentrix-onboarding/src/billing.rs:300`, `onboarding/plan.html:49`) while the console
   says KSh. Done: Change plan says what starts and stops and revises the PayPal plan, as the
   period switch already does (db73e0c); invoices list from PayPal's charges; the budget reads in
   the site's currency on every page.
9. **The chosen look reaches the Move as colours and fonts only.** The stage says "The Move
   rebuilds your site in the look you choose" (`crates/agentrix-concepts/templates/concepts/page.html:41`),
   but the Move takes seven colours and two font stacks (`crates/agentrix-concepts/src/look.rs:1-3`,
   `crates/agentrix-move/src/theme.rs:18-37`) and keeps its own page layout
   (`crates/agentrix-console/templates/site/page.html`). Done: the release's home page uses the
   chosen design's header, hero and sections, filled with the site's real content, and Prove it
   matches compares against it. Until then the stage's lead says only colours and fonts carry
   over.
10. **A refused operator form comes back with different choices.** A refused SEV1 comes back as
    SEV2, and a refused invite comes back as Editor on the stable ring, so sending it again files
    the wrong thing (`crates/agentrix-operator/templates/incidents.html:25-30`,
    `crates/agentrix-operator/src/incidents.rs:244-253`,
    `crates/agentrix-operator/templates/portfolio.html:57-58`,
    `crates/agentrix-operator/src/partners.rs:527`). Done: every select, checkbox and number keeps
    what was sent, with a test for each form.

Next after these: Back office and the brief show no visits, though moved sites count them
(02c98c3; `os/home.html:75-79`, `crates/agentrix-node/src/brief/mod.rs`); the look-alike report
is an email link that nothing records (`crates/agentrix-samples/src/common.rs:55-58`); gift plans
and cake designs can't be shared with an expiry (`classy/plan.html`, `kilele/studio.html`); the
portfolio's Approver lane is hard-coded empty (`crates/agentrix-operator/src/partners.rs:725-726`).

### 3.3 Found in passing

- The samples' PayPal return can charge an order the customer cancelled in another tab: it checks
  only for states after paid before it captures (`crates/agentrix-samples/src/commerce.rs:1542`;
  the pay start at `:1410` does check). It's PayPal's sandbox, so no real money moves.
- "Cancel this order" has no confirm step and no undo (`crates/agentrix-samples/templates/shop/order.html:28`).
- The overview reads billing without the sync Money runs first, so its figures can lag
  (`crates/agentrix-operator/src/overview.rs:126`).
- The incident's health card keeps the oldest 48 hours, so an incident open longer never shows
  its latest hours (`crates/agentrix-operator/src/reads.rs:261`).
