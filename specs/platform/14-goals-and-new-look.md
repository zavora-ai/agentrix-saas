# Your goals before the Move, New look after it (proposal)

**Status: proposed, waiting on James.** W0, Sun 8 Nov. Nothing here is built. It turns the research
report "Concept stage before site migration" (James has it; the notes are in
`research_notes/Concept stage before site migration/` beside the repos) into work the sessions can
pick up the moment James says yes.

## Why

James asked whether the owner flow is missing a concept stage: strategic goals, design concepts,
A/B testing and a round of iteration with the owner before the Move. The research says yes for
goals, and yes for design concepts but not before or inside the Move.

- Hosts (Kinsta, WP Engine, SiteGround) copy a site and ask nothing. Builders that import from
  WordPress (Squarespace, Wix, Webflow, Shopify, Duda) keep content and make the owner pick a
  template. AI builders show one design at a time; side-by-side directions exist only in tools for
  designers (Relume, Google Stitch). Only agencies set goals, with a 2 to 4 week discovery.
- Google's advice for a move is one change at a time: move first, then change the layout.
  Projects that moved and redesigned together lost 26 to 38% of traffic, mostly from changed
  content, links and URLs.
- At 1,000 visits a month and 3% conversion, a live A/B test can only detect a 67% difference in
  three months. Tests here catch disasters, not winners.
- Missions start after go-live with no starting count, so "60 Christmas cake orders" can't be
  read against anything.

So: decide goals before the Move, choose a look whenever the owner is ready, ship it only after a
faithful Move and four weeks of counts.

## The rule that keeps "Prove it matches" intact

Words, links, prices, URLs and page titles match at every stage. Look is the only thing a new
look may change. The Move keeps its 95% look bar, with only itemised repairs (malware gone, a
broken mobile layout fixed) as known differences. When a new look is approved, the same parity
check compares the build with the approved pages for look, and with the live site for everything
else.

## The flow

| Step | When | The owner | We produce | Who builds it |
|---|---|---|---|---|
| Assess (one line added) | Before sign-up | Pastes the URL | One faithful preview and the line "Same look, new staff. New looks come after you move." | W4 |
| **Your goals** (new) | After choosing a plan, while the domain claim runs; finished before the Move starts | Six short screens, two required | A one-sentence brief, a measurement plan, a draft first mission | W4 pages; W8 the tracked event and baseline; W5 the mission draft |
| The Move (two additions) | After the claim | Reviews the proof and approves the repairs list | A faithful site at 95% or more, the main action tracked from day one, a known-differences list | Move builder after the handover; W8 tracking |
| Go live | After the proof | Passkey | 30-day undo; the first mission offered from the brief | W4, W5 |
| Baseline weeks (passive) | Weeks 1 to 4 | Runs the first mission | Weekly counts of the main action on the same event | W8 |
| **New look: directions** (new) | Opens at go-live, whenever the owner wants | Picks one of three, or "Keep my look" | Three element collages from the owner's real content, each with one line tied to the brief | W11 pictures and drawing; W2/W7 theme variants; W5 the console app |
| Customer check (optional) | After a direction | Shares a link with 15 to 30 customers | Five-second comprehension, first click on the main action, preference with reasons | W5 page; W9 the share message |
| **New look: real pages** (new) | After a direction | At most two rounds: section comments plus a 1 to 5 rating, each round one batch | Staging versions of the home page and the main-action page | W2/W7, W5 |
| Prove it still works | After the last round | Passkey | Parity against the live site for content, against the approved pages for look; its own 30-day undo | Move builder's parity engine after the handover |
| Staged rollout | At least four weeks after the Move | Reads a weekly card | Home page first, then other page types; counts against the baseline; the alarm at the agreed floor | W8, W5 |

### Your goals: the six screens

1. **What should the website get people to do?** One of: order, call, WhatsApp, visit the shop,
   read or subscribe, advertising enquiry. Required.
2. **How many do you get now?** From the order book or WhatsApp for the last four weeks and the
   same weeks last year; optionally connect Google Analytics or Search Console. Required.
3. **Your most important customer, and what they come for**, in one sentence.
4. **How should it feel?** Three to five sliders (homemade to polished, playful to serious,
   affordable to premium, traditional to modern), with a recommended setting.
5. **Your pages.** The list from Assess: keep, not needed, missing. Recorded only; nothing is
   pruned during the Move, because pruning changes content.
6. **Who decides, and who else gets a say.**

Out: a brief such as "Kilele Bakery's website gets Nairobi families to order celebration cakes;
we take about 40 in a December and want 60." A measurement-plan row (goal, tracked event,
baseline, target, date, where it's checked). A draft mission.

### New look: the three directions

Every direction keeps the logo and brand colours and uses real pieces: the header, one product
card with its real price, the main-action button, a type and picture sample.

- **Your look, sharpened:** same layout; contrast, mobile spacing and the order button fixed.
- **Same brand, new layout:** layout and type change.
- **Bolder:** a wider palette and a new picture style (W11).
- **Keep my look** is always there and turns the stage into a health check.

An owner who already knows what they want gets one argued concept instead. Questions are yes/no
and tied to the brief ("Is it obvious where to order a Christmas cake?"), never "what do you
think?". A 1 or 2 out of 5 on a real page sends the owner back to the directions instead of
spending a round. After two rounds, further changes go to the Editor as approved missions.

## What gets measured

- Before launch: the customer check's three numbers. They help choose; they don't predict sales.
- After launch: the confirmed main action per week against the matched baseline. Taps and form
  starts are leading signs. Guardrails: search entrances, mobile conversion, page speed,
  returning visitors.
- The rollback floor is agreed before shipping, for example orders more than 30% below baseline
  over four weeks. At 30 orders a month, normal variation is about plus or minus 18%, so a
  tighter floor fires on noise.
- A split-URL test only for sites with a few thousand visits a month or more, for three to four
  weeks, with 302s and canonicals.
- No synthetic users as evidence.

## Data (additive; owners' ranges)

| Table | Holds | Range |
|---|---|---|
| `site_briefs` | The six answers, the sentence, who decides; one current row a site, history kept | W4 0400s |
| `site_goal_events` | The main action and the event that counts it | W8 0800s |
| `goal_counts` | Weekly counts, baseline weeks marked | W8 0800s |
| `look_directions` | Three directions a request: rationale, assets by digest, chosen or not | W5 0500s |
| `look_rounds` | Rounds: section comments, rating, submitted at, approved by receipt | W5 0500s |

Each decision is a card v2 with a receipt: the brief signed, the direction chosen, the new look
approved, the rollout started, the rollback.

## Timing, if James says yes

- Before the 17:00 freeze: nothing ships. At most, the Assess line and the spec.
- Next: Your goals (W4, W8, W5), because the Move needs the tracked action from day one.
- After that: New look (W11, W2/W7, W5), and Prove it still works once the Move builder is
  handed over.
- For Kilele: a faithful Move now, the Christmas mission on the unchanged site, New look in
  January.

## Don't

Redesign inside the Move. Show concepts at Assess. Offer three near-identical options. Stitch
three directions together. Leave rounds open. Trust an underpowered test. Change URLs, titles or
prices in a new look. Use a 301 during a split test.
