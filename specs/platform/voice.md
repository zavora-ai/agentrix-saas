# Words on screen

Every string a visitor, customer, owner or operator reads follows this page: templates, emails,
WhatsApp messages, error messages, empty states, seed data, alt text and button labels. It
builds on the shared vocabulary in [06 §4.8](../../docs/06-website-experience.md#48-words) and
the honesty rule in [01 §5](../../docs/01-product.md#5-trust-promises). Where they disagree,
06 §4.8 wins on terms and this page wins on style.

## The test

Read the string aloud to the person it is for: a baker in Nairobi at 6 a.m., a customer on a
bus with a slow phone, an operator in an incident. If it sounds like a brochure, a press release
or a chatbot, rewrite it. If it says less than the facts on screen, rewrite it.

## Rules

1. **Say what happened, with the number.** "3 orders overnight, KSh 8,400" beats "Your store had
   a great night!". If there is no number, don't invent one; say what is known.
2. **Say who did it.** "The Cashier refunded KSh 1,200 to Amina" beats "A refund was processed".
   WebAgents are named by role (the Concierge, the Cashier), never given human names.
3. **One idea per sentence. Short words.** "Pay" not "complete your payment". "Can't" not
   "is unable to".
4. **Buttons are verbs the person would say.** "Approve", "Send to Wanjiru", "Undo", "Pay KSh
   2,400". Not "Submit", "Proceed", "Continue your journey".
5. **Errors say what broke and what to do now.** "We couldn't reach PayPal. Nothing was
   charged. Try again in a minute." Never "Oops! Something went wrong."
6. **Empty states say why it's empty and the next step.** "No orders yet. They appear here the
   moment someone pays." Never a cheerful illustration with no instruction.
7. **Honest gaps.** "The site doesn't say, so I won't guess. Ask Wanjiru?" (06 §5). "Over budget
   by KSh 1,400". "Answers are paused". Never fill a gap to look complete.
8. **AI is labelled, never hidden.** Visitors always see that a WebAgent is answering. Never
   "staff", "chatbot", "bot", "assistant" or "pop-up" in visitor copy (06 §4.8).
9. **Real data only.** Screens show recorded activity. Sample sites and seed data say "Sample"
   wherever a person could mistake them for a real business (the honesty rule, 07 §1).
10. **Sentence case** for headings, buttons and menu items. No Title Case.
11. **Local by default.** KSh with a space and thousands separators ("KSh 2,400"), 24-hour or
    "6:58 a.m." consistently per surface, dates as "Sat 7 Nov". Swahili strings are written by a
    person, not machine-translated, before they ship.

## Never on screen

These mark text as machine-written or as marketing filler. The copy lint (below) fails on them
in user-facing strings.

- **Words:** seamless, seamlessly, effortless, effortlessly, elevate, empower, unlock, unleash,
  supercharge, revolutionize, game-changer, cutting-edge, state-of-the-art, next-generation,
  world-class, best-in-class, robust, leverage, utilize, delve, journey (outside travel),
  landscape (abstract), tapestry, vibrant, testament, pivotal, crucial, foster, harness,
  streamline, synergy, holistic, curated (as praise), magic, magical, delightful, stunning,
  breathtaking, nestled, "in today's world", "look no further", "rest assured", "we've got you
  covered", "at your fingertips", "take it to the next level".
- **Openers and closers:** "Great question!", "Absolutely!", "Certainly!", "I hope this helps",
  "Let me know if…", "Happy to help!", "Oops!", "Uh-oh!".
- **Shapes:** "It's not just X, it's Y" and "not X but Y" used for drama; lists of three
  adjectives ("fast, simple and secure"); a run of short dramatic fragments ("No forms. No
  waiting. Just answers."); "From X to Y" where X and Y aren't a range; questions that answer
  themselves ("Ready to grow?").
- **Punctuation:** em dashes (—) and en dashes (–) in prose; use a full stop, comma, colon or
  brackets. An en dash in a range of numbers, times, days or months ("9–5", "7:00–20:00", "Mon–Sat") is fine. No emoji in product UI except where
  a person typed it.
- **Exclamation marks**, except in a customer's own words or a real celebration the owner chose
  (a first order). At most one per screen.

## Before and after

| Don't | Do |
|---|---|
| Seamlessly migrate your website with AI-powered magic! | Move your site. We copy it, clean it and prove it matches before anything changes. |
| Unlock the full potential of your business with intelligent WebAgents. | Your site answers customers, takes orders and tells you what it did each morning. |
| Oops! Something went wrong. Please try again later. | The Move stopped at the replay step because the old site timed out. Start it again; it picks up where it stopped. |
| Your journey starts here | Paste your website address |
| Fast, secure and reliable. | Pages load from a signed copy, so they stay up even if the AI is down. |
| We're processing your request… | Checking 50 pages against the old site (31 done) |
| No data available | No visits counted yet. Counts appear the day after the site goes live. |
| Congratulations! 🎉 Your site is live! | Your site is live at kilelebakery.co.ke. The old site stays on standby for 30 days; Undo puts it back in one tap. |

## The copy lint

W7 owns `scripts/copy-lint` (tests in `scripts/tests/test_copy_lint.py`), and every workstream
runs it before committing UI:

```sh
scripts/copy-lint                 # the whole repo: exit 0 clean, 1 findings, 2 broken allow file
scripts/copy-lint crates/agentrix-onboarding/templates   # one folder or file
scripts/copy-lint --rules         # what each rule means
scripts/copy-lint --concepts      # also concept/ (presentation copy)
```

- It reads what a person sees: text between tags; `alt`, `title`, `placeholder` and `aria-label`;
  meta descriptions; words passed to kit macros (`ui.button("Approve")`); every string in words
  and seed JSON (keys starting with `_` are notes and skipped); Markdown and text templates
  (emails, WhatsApp); prose literals in JS islands. Template code, scripts, styles, `<code>`,
  `<pre>` and comments are skipped.
- It flags the words, phrases and openers above, em dashes in prose and en dashes outside a
  numeric range, exclamation marks, emoji, Title Case in headings, buttons, labels and page
  titles (names such as Night Time and Kilele Bakery keep their capitals), "not just X" shapes,
  runs of short dramatic fragments, self-answering questions, buttons that aren't verbs
  ("Submit", "Proceed"), and, on visitor pages, "bot", "chatbot", "assistant", "staff",
  "pop-up", "AI concierge", "chatting" and money not written as "KSh 2,400".
- Deliberate exceptions go in `scripts/copy-lint.allow`, one per line:
  `path glob | rule | text the line contains | why it is allowed`. An entry without a reason a
  reviewer can check is refused.
- `scripts/platform-merge.sh` runs it on every merge to `platform`, and CI runs it on every branch.

The lint catches the obvious tells. It doesn't make copy good: each workstream reads its own
screens aloud against **The test** above before it marks a page done, and W7 reviews copy across
surfaces before each merge to `main`.
