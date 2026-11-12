# ADR-012: Autonomy is per job on the adk-kernel ladder; standing grants are signed

Status: Proposed, 7 Nov 2026. Amended 7 Nov 2026: James approved all four amendments (amendment 4 later the same day).

## Context

The console shows an autonomy dial per WebAgent. adk-kernel's `AutonomyLadder` keeps a level per job (`Observe`, `Propose`, `ActWithApproval`, `ActWithinBudget`, `ActReversibleOnly`), compiled to harness policy by `adk-harness`'s `LadderPolicy`. Only an owner's decision raises a level (A-I13). "Act within budget" runs through standing rules created by that decision, and adk-harness records a rule's creator but does not sign the rule. The Effects Gateway must verify unattended effects independently ([ADR-004](004-two-key-holders.md), [ADR-006](006-passkey-signed-approvals.md)).

On 7 Nov James asked for the dial to be checked against Anthropic's published advice on autonomous agents. Most of the design already matched. Four gaps were found:

- **Irreversible effects could run unattended.** At "Do it and tell me", undo was offered only "where reversible". The website harness lets `ActWithinBudget` run any non-draft tool under the job's standing rules (`level_decision`, `adk-harness/src/website/ladder.rs:66`), including messages and payments. The framework triage found it worse: at that level `LadderCompiler::declarative` allows every job tool, `wallet_pay`'s only bound is the charter's `spend_cap`, so a job without one pays without limit, and the website ladder never checks K-I9 (`is_never_auto_allowed`), which only paired drivers do (05 §10, U26). Anthropic recommends "requiring human oversight and approval of any model actions with irreversible consequences" ([Agentic misalignment](https://www.anthropic.com/research/agentic-misalignment), Jun 2025).
- **Agents had no sanctioned way to stop and ask.** Anthropic found Claude Code asks for clarification more than twice as often on the most complex tasks as on minimal-complexity ones. It calls agent-initiated stops "an important kind of oversight" ([Measuring AI agent autonomy in practice](https://www.anthropic.com/research/measuring-agent-autonomy), Feb 2026). On our ladder, an owner's rejection of a card the agent chose to raise would count towards demotion.
- **Promotion evidence could be rubber stamps.** Users approved roughly 93% of Claude Code's permission prompts ([How we contain Claude across products](https://www.anthropic.com/engineering/how-we-contain-claude), May 2026). So "20 unchanged approvals in 30 days" can be 20 taps on a notification.
- **Recurring work needed a card per step,** which feeds the same fatigue.

## Decision

- **The dial is a view of the ladder, one dial per job:**
  - Watch (`Observe`);
  - Suggest (`Propose`);
  - Ask me first (`ActWithApproval`);
  - Do it and tell me, and Handle it within budget (both `ActWithinBudget`; they differ only in how actions are reported);
  - Undo only, for the Guardian only (`ActReversibleOnly`).
- **Starting levels** are written into charter version 1 and signed by the owner on the go-live card.

### 1. Each job's ceiling follows its effect class

A job's dial cannot rise above the ceiling of the most consequential tool it holds. The class is the catalogue's `EffectClass` (`adk-kernel/src/drivers.rs:134`).

| Work | Classes | Ceiling |
|---|---|---|
| **Reversible.** Drafts; changes to the site's own pages, which a signed release reverts (`content_revert`, `site_rollback`); the edge's read-only switch | `read`, `write_local`; `publish_public` on the site's own pages only | Handle it within budget |
| **Irreversible or outward-facing.** Posts and review replies off the site; messages to customers or third parties; meetings with others; moving money; deletions; any tool the catalogue does not class | `send_external`, `schedule_with_others`, `financial`, `delete`, `other`; `publish_public` off the site | Ask me first |

- **The one exception is an owner-approved standing rule with explicit limits,** signed with the owner's passkey or attested after the step-up. It must state all four limits:
  - an amount;
  - the recipients;
  - a frequency;
  - an expiry.

  The Effects Gateway verifies the signature or attestation and every limit before it runs the effect unattended, and refuses a rule that lacks any of them. `StandingRule` (`adk-harness/src/approvals/rule.rs:386`) can already express an amount and recipients as `AtMost` and `InSet` predicates, and has an optional `expires_at`. U6 adds a signature, a use cap per window, and a check that refuses a rule for an irreversible class unless it has all four limits. Examples are the Cashier's refunds under the owner's threshold, and the Caretaker's yearly renewal of one domain up to a set price.
- **Value given away is money.** Discounts, refunds, store credit and free items share one value-given cap per site, across the Concierge, Sales and the Cashier. In Anthropic's Project Vend, curbing discounts by about 80% was followed by refunds tripling and store credits doubling ([Project Vend: Phase two](https://www.anthropic.com/research/project-vend-2), Dec 2025). One cap stops the giving moving from one channel to another.
  - **What counts:** everything given away without a per-call owner decision, whatever the reason, including refunds for damaged goods. A reason is the agent's word, and exempting one would reopen the channel. A refund or discount the owner approves on its own card does not count.
  - **Size and window:** the owner sets both in the signed rule. The window is weekly by default, and the console suggests 2% of the previous four weeks' order value.
  - **A rule's frequency limit counts site-wide**, as in "5 refunds a week". Recipients are named in the rule, for example "the order's original payer". A per-customer limit may be added later if abuse shows up.
- **The public desks still have no dial.** Their controls are the capability manifest, amount thresholds and consent scopes. A desk's reply in the thread of the person who wrote is governed by the manifest, not by this ceiling.

### 2. Ask anyway

- At every level, a WebAgent that is unsure, or meets something new, raises a decision card instead of acting. New things include a new kind of request, a new recipient, an amount outside the job's history and a page type it has not handled.
- **The card is marked as raised by the agent.** The owner's answer to it never counts as a rejection and never triggers demotion. The weekly review counts asking in the job's favour.
- **The evaluation suite ([ADR-016](016-evaluation-gates.md)) tests both directions:** cases where the right move is to ask, and cases where asking would be needless.

### 3. Promotion needs reviewed approvals, and cards are rationed

- **Promotion is offered when four conditions hold:**
  - 20 unchanged approvals in 30 days;
  - no breach;
  - a passing evaluation suite ([ADR-016](016-evaluation-gates.md));
  - **only reviewed approvals count.** A reviewed approval is one the owner gave with the card's detail open in the console, however it was confirmed: a tap, a key, the passkey sheet, the step-up code or a spoken "approve that". Every approval records its kind (`reviewed`, `one_tap`, `spoken`, `batch`, `digest`; [07 §5.3](../07-owner-console.md)).
- **What does not count:**
  - one-tap approvals from a notification, or WhatsApp's Allow once;
  - spoken approvals with no card open;
  - approvals given in a batch or from a digest;
  - a review by another WebAgent. In Project Vend, a supervising agent "shared many of the deficiencies and blind spots" of the worker on the same model.
- **Every promotion is approved with the owner's passkey, or the MFA step-up without one** ([ADR-006](006-passkey-signed-approvals.md)). The grant carries the owner's signature, or control's attestation of the step-up. The Effects Gateway verifies it, its bounds and its expiry before running an unattended effect. Signed standing rules are upstream enhancement U6.
- **Demotion stays automatic:** a breach, two rejections in a row or an incident. Owner edits, rejections and stops are tracked per job and shown in the weekly review.
- **Trivial cards are batched.** A trivial card is low risk, reversible and inside the job's limits, and it goes into one digest card a day.
  - Individual cards per site are capped per day; the attention manager owns the cap ([07](../07-owner-console.md)). An agent-raised card is never trivial, so it is never digested, but it counts against the cap.
  - Over the cap, cards are held for the next brief. Two kinds still arrive individually and count against the cap: P1, and a decision that a waiting customer is blocked on. The platform knows the second from the visitor's thread parked on the approval, never from the agent's say-so.
  - The cap limits what reaches the owner, not what the owner can see: held cards stay in a collapsed row that opens on demand.
  - No card is held past its `expires_at` minus one hour, the same limit as snoozing ([05 §7](../05-webagents.md#7-surfaces)). It then arrives, without ringing in quiet hours, so the cap never turns into silent expiries.
  - Approvals from a digest do not count towards promotion, so a job whose cards are mostly digested is promoted only when the owner raises its dial.

### 4. Approve the plan, not each step

- A job may propose a bounded plan, such as the Editor's weekly content plan. A plan states:
  - its items;
  - the pages or recipients they touch;
  - its limits;
  - an expiry.
- **Once the owner approves the plan with the passkey (or the step-up), its items run within its bounds without a card each.** This follows Claude Code's Plan Mode.
- **A change outside the plan falls back to the job's dial.**
- **An approved plan compiles to signed standing rules (U6), one per tool,** with no new upstream work. Each rule carries:
  - an `InSet` predicate over the plan's items;
  - a use cap equal to the item count;
  - the plan's expiry.
- It covers irreversible items only if it states the four limits of amendment 1.
- **The plan's approval counts once as a reviewed approval.** The items it runs are not decisions and are not promotion evidence. The console lists them under "Done for you" as "under plan" ([07](../07-owner-console.md)).

### Mandates

**Mandates are written as duties with limits, not targets:** "keep every page true", not "maximise sales". Agentic misalignment advises caution before strongly instructing a model to follow particular goals.

## Consequences

- The owner controls each kind of work separately. For example, an Editor can fix typos alone and still ask before new pages.
- **Nothing irreversible happens without an owner's decision**, either per call or through a signed rule whose limits the Effects Gateway checks.
- **Promotion is slower and means more.** Fewer, richer cards replace a stream of taps.
- **The console needs four changes:**
  - Watch and Suggest positions, or a "lowered" state, because demotion can land there;
  - positions above a job's ceiling shown but locked, with the reason;
  - a mark on cards the agent raised itself;
  - a record of whether each approval was reviewed.
- **Upstream U26 carries the kernel and harness changes** ([05 §10](../05-webagents.md#10-upstream-enhancements-to-adk-rust-v3)). Until it lands, the platform enforces the ceilings in its promotion path and in the Effects Gateway.
- See [05-webagents.md §2.3](../05-webagents.md#23-the-owners-dial-and-the-ladder).
