# lablab.ai submission form: Agentrix

Status: draft v1, Sat 7 Nov 2026. Owner: submission lane (S4-03). James fills and submits the
form (S4-04). Copy each field as it stands. `TODO(live)` marks a value that waits on the
deployed build; never submit one unfilled.

## Project title

Agentrix: Upgrade your website into WebAgents

## Short description (one line)

Agentrix moves a small business’s WordPress site, hacked or not, through sealed rooms on Vultr
into a new site run by AI WebAgents, and goes live only on the owner’s passkey.

## Challenge

Challenge 1: Blast Radius Zero.

## Long description

About two in five websites run on WordPress (W3Techs, 7 Nov 2026). For many small businesses
that means a site that waits: questions go unanswered at night, plugins go unpatched, and some
sites are quietly hacked without the owner knowing. Moving such a site the usual way copies the
infection across, and the new site still can’t serve the AI agents customers now send.

Agentrix does the Move. The owner pastes a URL and gets an honest health report, read by a
browser in a sealed room. They sign in with a passkey and prove they own the site. Then the
old WordPress is replayed from its own database in a sealed room with no route out. The
content is cleaned, and anything infected is quarantined with the reason in plain words. A
builder model writes the new site’s code, which is rendered in a room with no network and
checked page by page against the old one, with a vision model’s verdict. Nothing goes live
until the owner approves a go-live card with a passkey that signs that exact card. The old site
is kept, and undo lasts 30 days. The new site answers visitors from its own pages, with
citations. It says when it doesn’t know, and it has a door for customers’ AI agents (AWP and
MCP), whose orders wait for the owner.

**Containment.** Every fetch, replay, build and render runs on a separate Vultr VX1 sandbox
VM that holds no secrets. Each job gets its own gVisor container with:
- a read-only root, a non-root user and no capabilities;
- CPU, memory, process and time caps;
- an egress proxy that allows only the job’s hosts and refuses Vultr’s metadata address.

Each room is destroyed after its job, and a signed receipt confirms it. The control plane
signs every job spec and receipts every result. On the live platform, the first Assess of
Zavora's infected fixture read 49 pages in gVisor rooms in about 38 seconds. It caught the
hidden spam links, the hidden instructions to AI agents and the disguised footer script, and the
egress proxy refused a redirect to Vultr's metadata address. A gauntlet of ten attacks planted
in a hacked WordPress copy scored TODO(live) n/10 contained, computed from receipts.

**On Vultr.** The platform runs on three Cloud Compute VMs (control, a VX1 sandbox and the
fixture) on a VPC, with firewall groups and DNS. Every content-store blob and signed receipt is
mirrored to Vultr Object Storage each minute, and read back by digest if the control VM loses a
file. Every model call
goes through a Model Gateway to Vultr Serverless Inference: `glm-5.3`, `qwen3.8-27b`,
`deepseek-v4.1-flash-1017`, `nemotron-3.5-content-safety`, the systemone decision model `clef`
and `qwen3-embedding-4b`. The gateway
is provider-agnostic, and Claude was used to validate the pipeline before the demo. Switching
provider is configuration plus a signed receipt the owner sees. The demo runs on Vultr.

**The business.** Moving is free with a plan: $29 or $79 a month, or per site for agencies.
These prices are a hypothesis we will test with pilots. Each site moved is steady Vultr
compute and inference, and the same single-node build is headed for Vultr’s Marketplace.

Built this weekend in Rust on adk-rust, Zavora’s open-source agent framework, which predates
the event. The README says what existed before and what we built.

## Technology tags

Vultr · Vultr Serverless Inference · Vultr Cloud Compute · Vultr VX1 · Rust · adk-rust ·
Docker · gVisor · WebAuthn passkeys · Ed25519 · MCP · AWP · Caddy · SQLite

## Category tags

AI agents · Sandboxing and containment · Cybersecurity · Website migration · WordPress ·
Small business

## Links and files

| Field | Value | State |
|---|---|---|
| Public GitHub repository (MIT) | TODO: the public repo URL (no remote is set in this checkout yet) | James creates and pushes |
| Demo application platform | Vultr | Ready |
| Application URL | https://agentrix.website | Live, deployed from a committed tree by `deploy/deploy.sh` |
| Video | TODO: the upload link (S3-09) | Waits on recording |
| Slide presentation (PDF) | Export the earlier deck ((an internal board, not public)) to PDF from its Share › Export menu, after filling its bracketed numbers | Deck drafted |
| Cover image | [`submission/cover.png`](cover.png), 1920 × 1080 (source: [`cover.svg`](cover.svg)) | Ready |

## Before pasting

- [ ] Every TODO(live) above replaced with a value from the live run.
- [ ] The gauntlet score matches the Trust tab, the deck and the video.
- [ ] The model list matches the routes the demo actually used (S3-10, the bake-off result).
      Until the switch gate, only embeddings and rerank run on Vultr; if S3-10 doesn't pass,
      rewrite the *On Vultr* paragraph to say so.
- [ ] The W3Techs figure re-checked on the day.
