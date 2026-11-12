# S3-10 step 1: the build and vision-parity bake-off

Sun 8 Nov 2026, early morning EAT. Move lane. Live calls through a local Model Gateway
(`agentrix-models`, mode `live`), one candidate per gateway, with routes set by
`AGENTRIX_ROUTE_BUILD` and `AGENTRIX_ROUTE_VISION_PARITY`. Production routes were not changed.

## Method

- **Pages:** the five clean pages of Zavora's dev fixture (`fixtures/wordpress-infected/exports/dev`:
  `/`, `/about/`, `/contact/`, `/custom-cakes/`, `/sourdough-starter-turns-five/`), after the
  Move's own Replay (R4.4 fallback) and Clean steps.
- **Build:** each page goes through the pipeline's `rebuild::build_page`: the builder's
  instructions, the page check (allowlisted HTML, no invented links or images), up to three
  tries with the error fed back, and `x-agentrix-tier: auto`. "Text parity" is the Prove step's
  word similarity between the clean source and the rebuilt body.
- **Vision:** each candidate judges the same five screenshots twice: once against the page's
  own text (`match`; a good judge scores high) and once against another page's text
  (`mismatch`; a good judge scores low). Screenshots are of the Sonnet build in the site theme,
  served over loopback and taken with headless Chrome.
- **Cost:** from the gateway's own usage rows (`usage.cost_micro_usd`, catalogue prices),
  per candidate Move. It includes the gateway's tier-router calls.
- **Runs:** three passes. Vision figures are from pass 3, after the two fixes below. Total
  spend for all three passes: **$0.14**.
- **Reproduce:** `crates/agentrix-move/tests/bakeoff.rs` (ignored test; needs the provider
  keys in the environment, a built `agentrix-models` and Google Chrome).

## Build

Fifteen page builds per candidate (five pages, three passes).

| Provider | Model | First-try pass | Flagged | Text parity | Cost per page | Seconds per page (median / max) | Why a page needed another try |
|---|---|---|---|---|---|---|---|
| Vultr | `qwen3.8-27b` | **15/15** | 0 | 0.984 | **$0.00089** | 4 / 7 | — |
| Vultr | `deepseek-v4.1-flash-1017` | 8/15 | 0 | 0.984 | $0.00066 | 8 / 11 | wrapped the page in `<main>` (refused by the page check) |
| Vultr | `glm-5.3` | 13/15 | 0 | 0.956–0.976 | $0.00157 | 6 / 9 | added a `mailto:` link the source page did not have |
| Anthropic | `claude-haiku-5-5` | 14/15 | 0 | 0.986 | $0.00039 | 4 / 7 | — |
| Anthropic | `claude-sonnet-5-5` | 15/15 | 0 | 0.984 | $0.00396 | 4 / 5 | — |

Every candidate finished every page within its three tries, and no page was flagged. Text
parity stays near 0.98 because the builder must keep every sentence. What differs is first-try
reliability (each retry is another paid call and a wait) and cost.

## Vision parity

Ten checks per candidate: five matches and five mismatches.

| Provider | Model | Mean score, match | Mean score, mismatch | Errors | Cost per check | Median seconds |
|---|---|---|---|---|---|---|
| Vultr | `deepseek-v4.1-flash-1017` | **1.00** | **0.03** | 0 | **$0.00023** | 2 |
| Vultr | `qwen3.8-27b` | 1.00 | 0.09 | 0 | $0.00056 | 4 |
| Anthropic | `claude-haiku-5-5` (Phase 1 baseline) | 0.96 | 0.06 | 0 | $0.00014 | 1 |

All three separate a right page from a wrong one by about 0.9. DeepSeek Flash separates them
most clearly, at under half of Qwen's price.

## The pick

| Purpose | Model on Vultr | Measured cost |
|---|---|---|
| `build` | **`qwen3.8-27b`** (15/15 on the first try, same parity as Sonnet) | **$0.0009 a page** (Sonnet: $0.0040) |
| `vision-parity` | **`deepseek-v4.1-flash-1017`** (cleanest match/mismatch separation) | **$0.0002 a check** |

**Together: about $0.0011 a page, or about $0.06 for the build and vision parity of a 50-page
Move.** Screening, decide and retries on harder pages come on top.

These two are already the defaults for `AGENTRIX_PROVIDER=vultr`: `build` defaults to the
standard tier (`AGENTRIX_TIER_STANDARD_VULTR=qwen3.8-27b`) and `vision-parity` to the fast
tier (`AGENTRIX_TIER_FAST_VULTR=deepseek-v4.1-flash-1017`). So the switch needs no
`AGENTRIX_ROUTE_*` override: the provider variable and its `routing.changed` receipt are
enough. With `x-agentrix-tier: auto`, a retry or a large page can lift a build to the deep
tier (`glm-5.3`). That tier also passed every page here, at about twice the cost.

## What the bake-off found and fixed

1. **Vultr's reasoning models ran out of tokens on vision checks.** In pass 1, 7 of 20 Vultr
   vision checks came back with no verdict (Qwen 5, DeepSeek 2). They spent 350–480 reasoning
   tokens each against a 600-token cap, as design §4.3 warned. The vision call now asks for
   2,400 tokens (`agentrix-move/src/gateway.rs`), and the errors went to zero. Claude
   doesn't reason by default, so it never hit the cap.
2. **The harness's own screenshots were clipped.** Headless Chrome on macOS lays pages out
   wider than a 390 px window, so every judge saw text cut off at the right edge. Match
   scores fell to 0.79–0.92, which would have looked like a parity problem. The site's CSS is
   fine (`max-width` containers, no fixed widths), and room-render's Playwright viewport is a
   true 390 px. The harness now uses 500 px.
3. **The page check works as intended against real models.** It refused DeepSeek's `<main>`
   wrapper, which would nest a second `<main>` inside the theme's, and GLM's invented
   `mailto:` link. Each model fixed its page on the next try once the error was fed back
   (R6.3).
