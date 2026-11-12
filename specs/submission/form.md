# The lablab form, field by field

Vultr Agent Rush, Challenge 1: Blast Radius Zero. Closes **Mon 9 Nov 2026, 02:00 EAT**.
Owner: W14 (the submission session). James pastes and submits.

Copy each field from between its markers. The character counts come from
`python3 specs/submission/count.py` and are checked against lablab's limits. `TODO(live)` marks a value
that waits on the live run on Vultr routes. Never submit one unfilled, and never fill one by
hand without the receipt behind it.

## Title (50 characters at most)

[field: title]
Agentrix: Upgrade your website into WebAgents
[end of field]

It's the home page's own headline. Alternatives, if James wants the containment in the title:
- `Agentrix: Move a hacked site, leave the hack behind` is 51 characters, one too many.
- `Agentrix: move hacked sites, leave the hack behind` is 50.

## Short description (50 to 255 characters)

[field: short]
Agentrix turns a WordPress, Shopify or React website into an autonomous AI business: WebAgents that work 24/7 for the owner, built and checked in secure sandboxes on Vultr, and live only on the owner's passkey.
[end of field]

James picked this one on Sun 8 Nov; it's his README's wording.

The earlier option, not used: "Agentrix moves a small business's WordPress site, hacked or not, into a new site run by AI WebAgents. Every untrusted step runs in a sealed room on Vultr, and nothing goes live until the owner approves with a passkey."

Tonight's live Move is of a WordPress site. No Shopify or React site has been moved live yet, so
judges who try one may not see the same result.

## Long description (600 to 2,000 characters)

[field: long]
Most websites just sit there. They answer whoever happens to find them, nobody follows up, and nobody is around at night. Search now sends fewer clicks, and more customers send AI agents that a passive site can't serve. The software underneath is a risk as well: WordPress runs 39.4% of all websites, and its ecosystem had 11,334 new vulnerabilities in 2025. A small business can't keep a site patched, or staffed around the clock.

Agentrix WebAgent Platform turns a WordPress, Shopify or React website into an autonomous AI business. Each site gets WebAgents that work 24/7 for the owner and the owner's customers: a Chief WebAgent with Concierge, Sales, Editor, Reputation, Cashier, Guardian and Caretaker. They answer visitors from the site's own pages, take orders and bookings, talk to customers' AI agents through an agent door, and write the owner a brief every morning. Anything with a consequence reaches the owner as one decision card, which they approve with a passkey.

Getting there is the Move: Assess, Claim, Replay, Clean, Rebuild, Prove and Go live. We never trust the old site. Every step runs as a signed job in a sealed room on a separate sandbox machine. Each room is a gVisor container with no secrets, caps on CPU, memory, processes and time, and an egress proxy that lets it reach only the job's own hosts. When the job ends, the room is destroyed and a signed receipt records it. A real browser reads the old site, the old WordPress replays with no route out, a model writes the new pages, and a vision model checks each one against the original, so the hack is left behind.

It all runs on Vultr: Cloud Compute for the control plane and the sandbox, Object Storage for the receipts, and Serverless Inference for every model call. Moving is free, plans start at $29 a month through PayPal, and the code is Rust on ADK-Rust, MIT licensed.
[end of field]

## Mode, challenge, categories and technologies

| Field | Value |
|---|---|
| Mode | Online |
| Track | **C1-Blast Radius Zero** (lablab's own label for Challenge 1) |
| Categories (pick from lablab's list) | **Security**, **Web Application**, **Cloud Application**, **Business**, **Customer Support** |
| Technologies (pick from lablab's list, then type the rest if it allows) | **VULTR**, **Qwen3**. Typed: Vultr Serverless Inference · Vultr Cloud Compute · Vultr Object Storage · Docker · gVisor · Playwright · Rust · ADK-Rust · WebAuthn passkeys · Ed25519 · MCP · PayPal |
| Vultr models (if the form asks) | `qwen3.8-27b` (build, Ask) · `glm-5.3` (build retries) · `deepseek-v4.1-flash-1017` (vision parity) · `nemotron-3.5-content-safety` (screening) · systemone `clef` (decisions) · `qwen3-embedding-4b` (retrieval) · `qwen3-tts-12hz-1.7b` (the video's narration) |
| Team size | 1 (James Karanja, Zavora Technologies), unless James adds people |

The pick-list values are the ones the 93 entries of 7 Nov used (lablab's public submissions):
the track was `C1-Blast Radius Zero` for 53 of them, and the commonest categories were Security
(53), Developer Tools (43), Cloud Application (23) and Web Application (16).

## Links and files

| Field | Value | State |
|---|---|---|
| Public GitHub repository | https://github.com/zavora-ai/agentrix-webagent-platform | Live since Sun 8 Nov, one commit, MIT. Later changes reach it through `scripts/publish-public.sh`, with James's go-ahead each time |
| Demo application platform | **Other**, if Vultr isn't in the list (84 of the 93 entries chose Other); say Vultr in the description | Ready |
| Demo application URL | https://agentrix.website | Control answers again (the old build); W0 deploys the platform on Vultr routes |
| Video | TODO: the upload link | `video/agentrix-vultr-demo.mp4`, 4:51, built by [`video/build.py`](video/build.py) from [`video/beats-demo.json`](video/beats-demo.json) |
| Slide presentation (PDF) | [`deck/agentrix-vultr-deck.pdf`](deck/agentrix-vultr-deck.pdf), 19 slides | Ready |
| Cover image (16:9) | [`cover/cover.png`](cover/cover.png), 1920 × 1080 | Ready |

## Facts the texts rest on

Each number in the texts above, and where it comes from. Re-check any marked live before
submitting.

| Claim | Source |
|---|---|
| WordPress runs 39.4% of all websites | W3Techs, 39.4% (James's "Why Agentrix" note, 8 Nov 2026) |
| 11,334 new vulnerabilities in the WordPress ecosystem in 2025 | Patchstack, *State of WordPress Security in 2026* (James's note) |
| Search sends fewer clicks | Pew Research: 8% of visits click a result when Google shows an AI summary, 15% without (James's note) |
| Customers now send AI agents | Adobe: traffic from AI sources to US retail sites up 393% year on year in Q1 2026; Imperva: 53% of web traffic is automated (James's note) |
| Each job is a signed spec; receipts are signed (Ed25519, RFC 8785 JSON) | README, *How the agent does real work*; `crates/agentrix-sandbox` |
| Firefox through Playwright in the fetch room | README Pattern B; `room-fetch-browser` image pinned in `deploy/env/node.env` |
| The replay has no route out | The first live Move's `job.replay` receipt (sha256:2d03d23a…): empty allowlist, the phone-home refused (board row S2-05) |
| The metadata address is refused | Live, Sat 7 Nov 21:17 EAT: the fixture's redirect to 169.254.169.254 refused by the job's egress proxy (board row S1-03) |
| (Not in the long description now) n/10 contained | The `gauntlet.run` receipt after the new account's Move; add it to the Containment paragraph if James wants it |
| Every model call goes to Vultr Serverless Inference | TODO(live): the `routing.changed` receipt "on vultr" after the switch (S3-10). Until then, the platform routes to Anthropic, and this sentence is not true |
| Plans from $29 a month, PayPal, moving free | `crates/agentrix-billing/data/plans.json`, the site's Prices page |
