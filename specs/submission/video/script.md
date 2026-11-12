# The demo video: script v3

Target 4:30, hard limit under 5:00. Challenge 1 asks for at least one containment moment; this
cut has three (beats 4, 8 and 9). Built by [`build.py`](build.py) from screen captures of the
live platform, the deck's slides and a narration track.

## Honesty rules

- **Live means live.** Every product shot is the deployed platform at https://agentrix.website,
  captured as it ran. Slides are labelled as slides by looking like slides.
- **Sped-up footage says so.** When the Move is sped up to fit, the corner shows the speed (for
  example "4x") and the real elapsed time from the Move page stays on screen. Nothing is cut
  out of the middle of a step.
- **Numbers come from receipts or the usage meter.** `[[…]]` marks a value read at recording
  time. Never type one in or round it up.
- **The fixture and samples are labelled.** The infected site is Zavora's own test site; the
  three businesses are samples.
- **The narration voice** is a Vultr text-to-speech model (`qwen3-tts-12hz-1.7b`), and the end
  card says so. If James records his own voice, the card drops that line.
- **The agent race** was run live on 8 Nov 2026 at 07:18 EAT; the clip says so and shows the
  recorded result. The customer's agent is ours and isn't part of the platform.

## Beats

| # | Time | Picture | Narration |
|---|---|---|---|
| 1 | 0:00 | Cover slide | Agentrix. Upgrade your website into WebAgents. This is our entry for Challenge 1, Blast Radius Zero. |
| 2 | 0:07 | Problem slide | About two in five websites run on WordPress. For a small business that often means a site that waits. Questions go unanswered at night, plugins go unpatched, and some sites are quietly hacked. Move one the usual way and you copy the hack across. |
| 3 | 0:25 | Live: agentrix.website home page, scrolled; the guide opens | This is agentrix.website. You paste your site's address, and the site's own guide walks you through what happens next. |
| 4 | 0:38 | Live: Assess on Zavora's infected test site. Frames from the sealed browser, the receipt ticking, the refusal, then the report | A real Firefox reads the old site from a sealed room on a separate Vultr VM. The room holds no keys, and it can reach only this site. Watch the receipt: this page tried to send our browser to Vultr's metadata address. Refused, and written down. Then the honest report: hidden spam links, a disguised miner script, and text aimed at AI agents, each with its evidence. |
| 5 | 1:12 | Architecture slide | Vultr is the whole stack. A control VM plans the work and signs every job. A separate VX1 sandbox VM runs each job in its own gVisor room, with no secrets and no public inbound. Every model call goes to Vultr Serverless Inference, through one gateway that holds the only key. |
| 6 | 1:32 | Plan, dispatch, verify slide | Every job is a signed spec going in, and a signed receipt coming out: what ran, what it touched, what was refused, and proof the room was destroyed. |
| 7 | 1:44 | Live, sped up and labelled: the Move. Replay, Clean, Rebuild, Prove, then the go-live card | Now the Move. The old WordPress runs from its own database in a sealed room with no route out. Its code tries to phone home. That fails, and lands on the receipt. Everything infected is quarantined, with the reason in plain words. A Vultr model writes the new pages, a room with no network renders them, and a Vultr vision model checks each one against the old site. This Move reached its go-live card in [[move time]], at [[parity]] parity. |
| 8 | 2:26 | Live: James approves the card with his passkey; the new site is live | Nothing goes live until the owner says so. My passkey signs this exact card. The new site is live, the old one is kept, and undo lasts thirty days. |
| 9 | 2:42 | Live: a receipt opened and verified in the browser; the gauntlet score | Here's one room's receipt: gVisor, a read-only root, caps on memory and processes, and destroyed, true. Ten attacks are planted in this test site, including a fork bomb and a webshell. [[n]] of ten contained, scored from receipts. |
| 10 | 3:02 | Models slide | Each Vultr model has one job, and something checks its work. Qwen builds the pages, DeepSeek checks them by eye, Nemotron screens for instructions aimed at AI, and systemone scores the risk. |
| 11 | 3:18 | Live: the new site. Ask gives a cited answer; then the agent race result | The new site answers visitors from its own pages, with citations. And customers' AI agents get a door of their own. In our race, an agent ordered a cake through the new site's door in seven seconds. On the old site it gave up after twenty-two steps. |
| 12 | 3:40 | Live: a sample owner's console | Every morning the owner gets a brief and a short list of decisions, each one a tap and a passkey. |
| 13 | 3:52 | Business slide | Moving is free. Plans start at twenty-nine dollars a month, paid with PayPal, and agencies pay per site. Every site moved is steady Vultr compute, storage and inference. |
| 14 | 4:07 | Competition slide | We checked all hundred and eighty-nine entries. Containment is table stakes. None of them moves a business. |
| 15 | 4:16 | What's next slide | Next: a throwaway Vultr VM for every Move, payments live, and ten pilot sites in Kenya. |
| 16 | 4:25 | Close slide, with the links | Agentrix. Upgrade your website into WebAgents. |

About 560 words. If a take runs long, trim beats 12 and 15 first. Never trim beats 4, 7, 8 or 9.

## Values to fill at recording time

| Placeholder | From |
|---|---|
| `[[move time]]` | The Move page: Start to the go-live card |
| `[[parity]]` | The go-live card (the Prove step's receipt) |
| `[[n]]` | The `gauntlet.run` receipt, on Vultr routes |

## What James does live

1. Sign in on Safari with his passkey and open the fixture's Move page. W14 records the screen.
2. Press Start a new Move, and leave the window alone until the go-live card shows.
3. Approve with his passkey. Open the live site.
4. Open the Trust tab and one room receipt, and press Verify.
