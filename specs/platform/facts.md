# One fact, one value

Owner: W7. The facts that disagree across screens (01b §6.3), settled once. Pages, seeds, emails
and concepts follow this sheet; where a seed is the source, the seed wins and this sheet follows
it. Every number a page shows still comes from records at render time: this sheet says what the
records hold, never a figure to type in.

## Kilele Bakery (sample)

Source: `fixtures/samples/tools/kilele.py` (W2), which writes `fixtures/samples/kilele-bakery/`.

| Fact | Value |
|---|---|
| Host | `kilele-bakery-sample.agentrix.website`, marked Sample on every page |
| Domain | `kilele-bakery.example` (reserved, so it can never be a real business). Never `kilelebakery.co.ke` or any other real-looking name, in pages, messages or test data |
| Owner | Wanjiru Kamau (fictional), Wanjiru to customers |
| Address | Woodvale Grove, Westlands, Nairobi; parking at the back, off Woodvale Grove |
| Hours | Mon–Sat 06:30–19:00; Sun 07:00–14:00 |
| Delivery | Westlands and Kilimani only, KSh 300, one fee per slot. Sat 10:00–12:00 and 12:00–14:00; Sun 10:00–12:00. So the site does deliver on Sundays (the old fixture site doesn't say; that gap is the Assess finding) |
| Cakes | 1 tier KSh 3,800 (serves 12–15, 2 days' notice); 2 tiers KSh 4,500 (25–30, 2 days); 3 tiers KSh 6,900 (40–45, 3 days). Gluten-free sponge +600, 1–2 tiers only, with the wheat-flour warning |
| Price list | v14 live; v15 is the Editor's draft (two cake prices) waiting for Wanjiru. Rate 1 USD = KSh 129.00. Visitors see "the price list of {the date it took effect}", never "v14" or "release v14": version numbers stay in owner and machine views |
| Collection | Collection slots as checkout offers them: Mon–Sat 08:00–12:00 and 14:00–16:00, Sun 08:00–12:00. Call it "collection" everywhere (not "pickup") |
| Orders | `KB-` plus four digits, never `K-`. KB-1042: Njeri, 2 sourdough and a weekend bake box, delivered to Kilimani, KSh 2,550 (USD 19.77), paid. KB-1043: Njeri for Amina, 2-tier lemon sponge, forest-green ombré, berries, collected (no delivery fee), KSh 5,250 (USD 40.70), approved and paid. KB-1044: Amani W., 2-tier vanilla, KSh 4,500, collect Sat 14 Nov 10:00, waiting for Wanjiru |
| Prices | Country sourdough KSh 450, butter croissant 180, weekend bake box 1,350 (03 §5.3 has the full list). The pending change is price list v15: 1 tier KSh 3,800 to 4,000, 2 tiers KSh 4,500 to 4,800 |
| AI budget | KSh 3,870 a month: the USD 30 of AI included with Commerce, at KSh 129.00. A monthly cap, never "a day" |
| Mission | "Christmas cakes": 60 preorders by 18 Dec; 23 so far |
| WebAgents | Chief WebAgent, Concierge (the Front desk to visitors), Sales, Editor, Reputation, Cashier, Guardian, Caretaker, Mover. Never "the Seller" |
| Confirmations | Wanjiru confirms, usually within 2 hours, 7:00–20:00, on WhatsApp. No other wait time is promised (not "48 hours") |
| Messages come from | On the sample, order messages and payment links only ever point to `kilele-bakery-sample.agentrix.website`, the host the visitor is on; `kilele-bakery.example` is the business's own domain in the story |
| Changes | Free until 24 hours before the slot |
| Standing rules | g-2 holds: up to 3 loaves until 12:00. g-3 refunds: up to KSh 2,000 an order, 5 a week, until 31 Dec 2026, to the original method. (03 §5.3 also says 3,000 once; 2,000 is the value) |
| Desks | Visitors: Front desk, Bread, Cakes, Orders, Visit. Owners call the router the Concierge |
| Plan | Commerce, USD 149 a month (about KSh 19,221 at 129.00), with USD 30 of AI included |
| Releases | The first release after a Move is v1. Kilele has been live since v1 and is at v14 |

## The platform

| Fact | Value | Screens that disagreed |
|---|---|---|
| Plan prices | USD: Essential $29, Business $79, Commerce $149 a month, with $5, $15 and $30 of AI included (`crates/agentrix-home/data/plans.json`). James still has to confirm them (board ask w10-plans). A local amount may sit beside a USD price, with the rate | assess.html said "KSh 3,700/month" |
| AI budget | Set and shown in the site's currency, as a monthly cap. The daily limit is an internal guard, never the owner's number | KSh 1,500 vs $30 vs a USD daily limit |
| Who holds orders | The Orders desk holds the request; the Cashier takes payment after the owner approves | "Cashier asks" vs "Orders asks" |
| Agency role on Kilele | Editor (edits pages; Wanjiru approves) | Editor vs Approver |
| Platform version | One scheme for owners: "Platform update 2026.11.3" (year, month, number) | 3.4, 2026.11.3, 0.4.3 |
| Ladder | Watch, Suggest, Ask me first, Do it and tell me, Handle it within budget. Guardian alone has Undo only | "Do it & tell me", "Within budget", "Handle it" |
| Quarantine | "Left behind" to owners | "quarantined" |
| Urgent findings | Whatever the Assess run found, from its records; never a fixed number | "2 urgent" vs "1 urgent" |
| Visitor resting line | "Answers are paused right now. The page and the form below still work." (`agentrix-marks`, 06 §4.3) | |
| Money in prose | "KSh 2,400" (a space, thousands separators); "USD 19.77" where money moves in dollars | "KES 2400" |
| Payment methods | PayPal only (James, Sun 8 Nov), in its sandbox on the samples. No M-Pesa, card or bank rail on any screen, seed, message or desk answer. "Never asks for a card number" lines are fine | M-Pesa in seeds, the Cashier's lines, delivery pages |
| Paying in dollars | Before PayPal opens: amount, payee and rate, e.g. "Pay USD 2.33 to Tana Valley Times (KSh 300 at 1 USD = KSh 129.00)" (01a §6.4 rule 9) | "Pay KSh 300 with PayPal" |
| Desk sources | Say where an answer came from in the visitor's words, with a time when it's live: "From today's stock list, 09:42", "From the returns policy". Never "no AI model used" or "answered instantly" | "Answered instantly … · no AI model used" |

## Tana Valley Times (sample)

Source: `fixtures/samples/tools/tana.py` (W2, on its branch), which writes
`fixtures/samples/tana-valley-times/`.

| Fact | Value |
|---|---|
| Host | `tana-valley-times-sample.agentrix.website`, marked Sample on every page |
| What it is | A local weekly, online every day, since 2006 (fictional). Brand: marks.html's Tana palette, Libre Caslon Text; layouts from the news concept |
| Editor | Nduku Mwikali (fictional), the owner; answers by email, usually within a day. Not "Otieno" |
| Colours | Accent `#b3261e`, second `#2f5d8a`, ink `#14213d` (tokens.css `[data-host="tana"]`) |
| Sections | Transport, Council, Farming and weather, Business, Community |
| Topics | The northern rail extension, the county budget, Ndovu market's move |
| Digest | "Friday five": every Friday at 07:00, five stories, two lines each, with links. The two lines are each story's own summary, as the newsroom wrote it. Keeps the email and topics only; one tap stops it and deletes the topics |
| Subscription | "Supporter", KSh 300 a month as a sample placeholder until the editor (and James) set it; one payment for one month, no auto-renewal |
| House rules | Desks never write news; every claim links to a dated, bylined story; subscriber stories show headline and link only; corrections reach answers and digests within minutes |
| Release | v3 |

## Classy Perfumes (sample)

Source: `fixtures/samples/tools/classy.py` (W2), which writes `fixtures/samples/classy-perfumes/`.
Where the seed and 03 §5.4 disagree, the value below is the one to use.

| Fact | Value |
|---|---|
| Host | `classy-perfumes-sample.agentrix.website`, marked Sample on every page. Not the real classyperfumes.shop: none of its names, products or people |
| Domain | `classy-perfumes.example` |
| Owner | Makena Wafula (fictional), Makena to customers; replies on WhatsApp usually within 3 hours, 9:00–19:00 |
| Where | Westlands, Nairobi. No street name: Mpaka Road is a real street |
| Delivery | Nairobi (12 named areas) KSh 300: same day if ordered before 14:00, otherwise next day. Mombasa and 18 named towns KSh 650, 2–3 working days by courier. Other towns: ask Makena |
| Returns | Unopened bottles within 14 days; opened bottles can't be returned. Through the Orders desk |
| Gift wrap | Free on every order, plum paper and gold ribbon, with a handwritten card |
| Samples | A free 2 ml sample of any scent with any order |
| Catalogue | 21 scents and 5 gift sets, KSh 4,000 to 8,900; Vanilla Gift Set KSh 4,500 (30 ml), Citrus Dawn 4,900 (50 ml), Amber Noir 6,400 (50 ml), Rain Oud 5,800 (30 ml, limited run). Stock shown as "{n} in stock", never "Only {n} left" |
| Price list | v31 live, 1 USD = KSh 129.00; visitors see "the price list of {date}" |
| Orders | `CP-` plus four digits: CP-2001 to CP-2015. Refund R-2008-1, KSh 2,300 (CP-2008, leaked in transit to Mombasa), waiting for Makena |
| Desks | Front desk (visitors), Fragrances, Gifts, Orders, Journal. Not "Scents" (03 §5.4 is out of date) |
| Plan and AI | Commerce, USD 149 a month with USD 30 of AI included: an AI budget of KSh 3,870 a month (not 3,000) |
| People | Achieng is a customer: the bride who makes the gift plan for her bridesmaids. One name is one person across all three samples, so the bridesmaids use names no other sample uses (not Njeri, Muthoni, Atieno or Zawadi) |
| Journal | "How amber wears in cold air"; "Scents for the short rains" (November is the short rains; the long rains are March to May) |
