# Deck review against lablab's rules and judging criteria

Status: Sat 7 Nov 2026, 23:45 EAT. Submission lane (S3-08). It reviews the deck
[the earlier deck]((an internal board, not public)) at version
1794082454-4c3a, 12 slides. A copy of its source is in [`deck/`](deck/). None of the fixes below
is applied yet; James asked to see the review first.

## The rules, as the event page states them

Source: https://lablab.build/ai-hackathons/vultr-hackathon, fetched 7 Nov 2026.

**Judging criteria (verbatim):**
- **Application of Technology:** "How effectively the chosen model(s) are integrated into the
  solution."
- **Presentation:** "The clarity and effectiveness of the project presentation."
- **Business Value:** "The impact and practical value, considering how well it fits into
  business areas."
- **Originality:** "The uniqueness and creativity of the solution, highlighting approaches and
  ability to demonstrate behaviors."

**Submission:**
- Basic information: title, short description, long description, and technology and category
  tags.
- A cover image, a video presentation and a slide presentation. The page states no video length
  and no slide format.
- A public GitHub repository, the demo application platform and an application URL.
- A README explaining "the Vultr services and infrastructure used", the backend and deployment
  architecture, the agent's use case and how it executes real tasks.
- Challenge 1 entries must "demonstrate at least one containment moment" in the video.

**Challenge 1, Blast Radius Zero:**
- "Build a web-based agent that performs real work"; "every action stays inside an isolated
  sandbox running on Vultr"; containment first.
- Plan, dispatch and verify real executed work.
- Pattern A (code in a container) and Pattern B (a browser in a container, a vision check, human
  approval before final actions).
- Four focus points: process isolation, secret hygiene, resource limits, lifecycle discipline.
- Vultr Serverless Inference is required for the agent's model calls; Docker is "strongly
  recommended".

**Other rules:** "Submissions must be original and MIT-compliant." Teams of 1 to 6.

The rule "under 5 minutes, covering problem, market, revenue, competition and future" is not on
this page. It comes from our own requirements (R15.2) and doc 04 §4. The deck covers all five
anyway.

## Against the four criteria

| Criterion | How the deck does | Gap |
|---|---|---|
| **Application of Technology** | **Weakest.** The models appear only as a list of names on slide 8, plus "a model writes" and "a vision model checks" on slide 3 | No slide shows what each model decides, how its output is checked, or the gateway that routes calls by purpose, tier and budget. The best material for this criterion is only in the README |
| **Presentation** | **Good structure.** One idea per slide, a consistent look, 12 slides | Every screen is a mock-up, with about 15 bracketed placeholders, so it could read as a concept deck. Slide 3 (seven cards plus two pattern boxes) is dense |
| **Business Value** | **Adequate.** Plans, cost to serve and channels are on slide 9 | Who buys isn't framed: owners, agencies, and Vultr as a customer, which the brief stresses. Vultr gets one box. Nothing ties the product to business areas: support (Ask this site), sales (agent-door orders), security (the cleanup), operations (the brief) |
| **Originality** | **Strong.** Slide 10's "0 of 93 move a website" is backed by the 7 Nov scan. Slides 4 and 6 show behaviours: refusals, quarantine, the agent door | The new mechanisms are scattered: replaying the hacked site from the inside, a passkey that signs the exact card, a model router that never sees page content |

## Against Challenge 1's requirements

| Requirement | Slide | Status |
|---|---|---|
| A web-based agent doing real work | 3 | Covered |
| Every action in a sandbox on Vultr | 4, 8 | Covered |
| Plan, dispatch, verify (the challenge's core idea) | 3, footer only | **Weak:** buried in a footer |
| Patterns A and B | 3 | Covered, but small |
| The four containment focus points | 4 | Covered, one for one |
| Vultr Serverless Inference for model calls | 8 | Covered in one line |
| Docker | 4, 8 | Covered |
| Architecture and use case explained | 2, 3, 8 | Covered |
| "Original and MIT-compliant" | 1, 12 | MIT is stated, but the deck never says adk-rust existed before the event (the README does) |

## Fixes, in order of impact

1. **Add a "Models at work" slide after slide 3** (Application of Technology). One row per
   purpose, giving the Vultr model, what it decides and how that's checked:
   - Build: `qwen3.8-27b`, raised to `glm-5.3` on retries; errors go back to it.
   - Vision parity: `deepseek-v4.1-flash-1017`, a verdict with a one-line reason.
   - Screening: `nemotron-3.5-content-safety`, which flags text aimed at AI agents.
   - Decisions: systemone `clef`, calibrated risk scores for quarantine and cards.
   - Concierge: `qwen3.8-27b`, with citations, and it refuses discounts.
   - Embeddings: `qwen3-embedding-4b`.

   A strip underneath covers the gateway: the only key holder, a budget per site, and a tier
   router that sees a task descriptor, never page content.
2. **Promote plan, dispatch, verify** from slide 3's footer to a visible band: a signed spec, a
   sealed room, artifacts by digest, a signed receipt.
3. **Rework slide 9 around three buyers:** owners, agencies, and Vultr itself (the Marketplace
   app, its WordPress customers, steady Inference spend). Add one line on business areas.
4. **Add a disclosure line to slide 12:** adk-rust is Zavora's framework (Apache-2.0) and existed
   before the event; everything in the repo was built this weekend.
5. **After the live run:** replace the mock-ups on slides 5 to 7 with real screenshots (the
   go-live card, the Trust tab with the gauntlet score, the quarantine list), and fill every
   bracket. This helps Presentation and "demonstrate behaviors" most.

Fixes 1 to 4 can be made now; the deck becomes 13 slides. Fix 5 waits on the acceptance run.

## Placeholders in the deck

| Slide | Placeholder | Source |
|---|---|---|
| 4 Containment | `[n]/10` | The gauntlet run (S3-01) |
| 5 Go live | `[0.__]` parity, `[n]` items left behind, `[__]` seconds to live | The acceptance run's receipts |
| 6 The converted site | The visitor's question and the cited answer; the race outcomes and times | The live site; the race (S3-03) |
| 7 The owner console | The brief text | The live console |
| 8 Built on Vultr | `[if S3-04 lands]` for Object Storage; `[after the S3-10 switch]` on the claim that every model call goes to Vultr | S3-04; S3-10 |
| 9 The business | `[$__]` model cost per Move | The usage meter (S2-11) |
| 12 Close | `[video URL]`, `[repository URL]`, James's `[role]` | James |

The cover and close already show `agentrix.website` (DNS is live), and slides 5 and 7 name the
fixture site, Kilele Bakery, labelled as Zavora's fixture. Check that it serves the
platform over HTTPS before exporting the PDF.

## Editing the deck

The artifact is the source of truth. Read it before editing, because James may have changed it
in the editor. [`deck/project/`](deck/project/) holds the slide files as last published
(`deck.json` plus one HTML file per slide), in the Slides type's format. Republish changed files
to the same artifact URL with the Artifact tool, `root` set to `submission/deck`. Then export
the PDF from the deck's Share › Export menu.
