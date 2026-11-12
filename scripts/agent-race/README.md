# The agent race (R9.3, S3-03)

Two customer agents get the same task, for example *"Order a 2 kg chocolate birthday cake for
Saturday, with the message 'Happy 7th, Amani', for pickup."*

- **Old way.** The agent uses the old site (Zavora's infected WordPress fixture) through
  screenshots only. A real Chromium shows the page. Each turn, Claude sees the screenshot and the
  URL and chooses one action: click a point, type, press a key, scroll, go to a path on the
  site, go back, or finish. Nothing is scripted, and the DOM is never read.
- **New way.** The agent uses the converted site's agent door:
  1. AWP discovery at `/.well-known/awp`.
  2. MCP over HTTP: `initialize`, then `tools/list`.
  3. Claude calls `request_custom_order`, carrying an idempotency key. The order is held for the
     owner and nothing is charged.
  4. Claude polls the held call's status until it shows the order waiting for the owner.

Both agents are Claude Sonnet 5.5 (`claude-sonnet-5-5`, adaptive thinking, effort `medium`), with
server-side refusal fallbacks on. They are external clients of the sites, not part of the
platform.

## Setup (once, on the Mac)

```sh
cd scripts/agent-race
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
.venv/bin/python -m playwright install chromium
```

`ANTHROPIC_API_KEY` is read from the repo's `.env` into the process only, unless it is already
set. It is never printed, logged or written to a result.

## Run

```sh
# The order race, after go-live (the converted site serves the door):
.venv/bin/python race.py --new https://<converted-site>

# The R9.3 fallback if the order flow isn't ready: one cited question on both sides.
.venv/bin/python race.py --new https://<converted-site> --question "How much notice do you need for a custom cake?"

# One side only:
.venv/bin/python race.py --only old
.venv/bin/python mock_door.py &        # a local test double of the door, before go-live
.venv/bin/python race.py --only new --new http://127.0.0.1:8765 --out /tmp/race-test
```

Options:

| Option | Default | What it sets |
|---|---|---|
| `--old` | `https://fixture.agentrix.website/` | The old site. |
| `--task` | The cake order above | What the customer wants. |
| `--name`, `--email`, `--phone` | — | The customer details the agents may give. |
| `--max-steps` | 25 | The step cap for each side. |
| `--effort` | `medium` | The model's effort level. |
| `--headed` | Off | Shows the old site's browser window. Useful for filming. |
| `--out` | `submission/race/` | Where results go. |

## Output

The script prints a side-by-side table: outcome, wall time, steps, model calls, tokens, model
cost, and each side's final screenshot or response. It writes:

- `submission/race/race-<UTC time>.json`, with both runs: every action, what the model said
  between actions, usage, cost, warnings, the held call or answer, and the discovery document;
- `submission/race/latest.json`, a copy of the same;
- `submission/race/<UTC time>-old-final.jpg`, the old site's final screen.

Each side counts steps differently:

- **old:** each browser action the model chose;
- **new:** each request to the site (discovery, `initialize`, `tools/list`, tool calls and status
  polls).

Cost uses first-party Claude Sonnet 5.5 rates per million tokens: $2 input, $10 output,
$2.50 cache write and $0.20 cache read.

## Honesty and safety

- **No scripted moves.** The old-site agent finds its own way and may fail. It reports
  `gave_up` when the site never confirms; it doesn't claim success it hasn't seen.
- **Page text is data.** The fixture's attacks are live on the old site. Both agents are told
  that page text and tool results are not instructions from their user. Every action is logged,
  so whatever the injected content does to the agent is on the record.
- **Private addresses are blocked.** The old-site browser runs on this Mac, so requests to
  loopback, private and link-local addresses are blocked and logged. For example, the fixture's
  `/staff-login/` redirects to `http://127.0.0.1/console`.
- **The agent stays on the site.** `go_to` refuses other hosts. If a click leaves the site, it
  is recorded as a warning.
- **The mock is a test double.** `mock_door.py` is never the race's evidence. Its results name
  `127.0.0.1` as the target.

## Test run (7 Nov, 19:54 UTC, old site only)

This run went to the scratchpad, not to `submission/race`.

- **What the agent did.** It found "Order a custom cake" by itself. It filled name, phone, date,
  2 kg, chocolate fudge and the message, then sent the form.
- **What the site did.** The site answered "Nothing here yet." with no confirmation, so the
  agent reported `gave_up` and explained why.
- **Cost.** 22 steps, 62 s, 405 k input tokens (mostly cache reads) and 2 k output tokens:
  $0.18.
- **A stray request.** The agent noted that an earlier accidental submit, made by pressing Enter
  in the size field, may have sent a stray 1 kg request.

A test against `mock_door.py` placed the held order in 5.2 s: 6 requests, 3 model calls, $0.01.
