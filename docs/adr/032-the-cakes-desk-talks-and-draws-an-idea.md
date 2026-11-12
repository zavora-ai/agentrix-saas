# ADR-032: The Cakes desk talks with a model and draws an idea of the cake

Status: Accepted by James, 8 Nov 2026. Amends decision S1 in [06 §11](../06-website-experience.md#11-what-the-experience-demands-of-the-webagents), [06 §5.1](../06-website-experience.md#51-make-it-with-me-personalised-product-creation) and the row "Present a generated image as the product" in [06 §10](../06-website-experience.md#10-what-we-will-not-do).

## Context

06 §5.1 has the visitor describe a cake in their own words while a desk maps the words to approved options. Decision S1 settled two things: the sketch is composed from declared parts without a model, and the `image` purpose is only an opt-in fallback for products without parts.

A review of Kilele Bakery's sample on 8 Nov found that the studio didn't do what its labels said:
- The chat was labelled "Cakes · AI", and the page said "You're talking to AI". But free text was matched by regular expressions on the page, and the questions were fixed strings. Calling that AI was not true.
- The picture was an SVG of flat shapes. It shows the choices, but not what the cake could look like.

On 8 Nov James asked for the opposite of S1, in his words: "there needs to be various input questions AI can use to create an approximation of what the cake looks like - make it an agent".

## Decision

### The desk talks through a model

- **`POST /cakes/talk`** calls the Model Gateway with the `concierge` purpose. No new purpose is added.
- **The questions vary and come one at a time.** The desk skips anything already known. It covers:
  - the occasion and who the cake is for;
  - the number of guests (giving the size);
  - the flavour, and any allergies;
  - colours or a theme;
  - the style (giving the finish);
  - the decoration;
  - the message;
  - collection or delivery;
  - the day.
- **The model answers in a fixed JSON shape**: what it says, the option codes it picked, the occasion and theme, anything asked for that the bakery doesn't make, a question for the owner, and the step it asked about.
- **The server checks every field before the page sees it:**
  - Only option codes on the live price list survive, and only in their own group. A sponge limited to some sizes is dropped for any other size.
  - A day survives only if the calendar has room for that size and that kind of hand-over.
  - The message must fit the plaque.
- **The model never states a price, a date or an allergen.** Its prompt doesn't contain prices. A reply that states one anyway is replaced by the step's fixed question (R8: blocked and replaced, never fixed by the model). Prices still come from `/cakes/quote`, built from rules, and days from the slots.
- **Refusals and allergens are worded by the server, from the site's own data:**
  - what the bakery doesn't make comes from its `not_offered` list;
  - the allergen note is quoted from the manifest.
- **A question the options don't cover is offered to the owner.** The visitor sees and edits it, and nothing is sent until they choose to send it. It goes through the existing `/ask/pass` path, which W13 delivers to the owner's Telegram.
- **"AI" appears only when a model answered.** Until the first model reply, the conversation is labelled "Cake studio" and says it matches words to the bakery's options. When the gateway can't answer, the page falls back to the old word matching, still labelled "Cake studio" and "without AI". The AI label and "You're talking to AI" appear before the first model reply is shown, so the disclosure still comes at first contact (06 §4.6).

### An idea of the cake, drawn with AI

- **`POST /cakes/preview`** draws an idea of the cake through `agentrix-media` with a new kind, `CakeIdea`, on the `image` purpose.
- **The server composes the brief from the checked options only**, never the visitor's own words. That shuts out prompt injection, and the cake shown can only be made of real options.
- **The model draws no lettering.** The brief asks for a blank plaque, and the page shows the message beside the picture.
- **The label is always on the picture:** "An idea of your cake, made with AI. Wanjiru bakes the real one."
- **Every idea goes through the same pipeline as any picture.** It is checked, drawn, screened by the content-safety model and described for its alt text.
  - If the safety check fails, or the picture shows a person, nothing is shown.
  - If the picture shows letters, it is drawn once more with another seed.
- **Ideas skip the owner's approval, and so stay out of the site's library.**
  - They live in their own table (`media_ideas`, migration 1202), never in `media_images`.
  - They never wait in Pictures, and never become a product or hero picture.
  - They are served at `/media/ideas/{hex}.{ext}`, only for digests kept as ideas.
  - The owner's library can't draw the kind.
- **Cost stays small:**
  - Each design is drawn once per site, cached by a hash of the brief.
  - Each visitor gets five new pictures a day, and each site sixty.
  - Every picture is drawn on the gateway's budget for the site (W12).
- **The held request carries the picture the customer saw.** The server attaches it only when it was drawn for exactly the requested options, and the owner's held-request card shows it.
- **The SVG sketch stays.** It is the instant view while a picture draws, and the fallback when none can be drawn.

## Consequences

- **06 §5.1, decision S1 and the risks row change** to the rules above. S1 keeps the sanitised SVG sketch as the instant view.
- **A generated picture of the product now reaches visitors.** That is acceptable only as labelled here:
  - it is always called an idea, made with AI;
  - it is never shown as the product on product pages;
  - the price, the days and the owner's confirmation remain the binding parts.
- **A visitor's conversation now costs model calls**, on both the `concierge` and `image` purposes. The per-visitor caps, the per-site cap and the gateway's daily budget bound it.
- **The pattern applies to other configurable products,** as long as their options carry words a picture can be drawn from. An option may give its own words in its rule's `looks` metadata, which the owner approves with the price list.
- **Open:**
  - The Cakes desk's prompt, mandate and budget belong in W1's roster, which isn't wired yet; the prompt lives with W2's studio for now.
  - An evaluation suite for the desk (ADR-016) should cover: no prices, no dates, no allergens, honest refusals, and only real codes.
