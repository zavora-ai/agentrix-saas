# Run the platform on this Mac

While the Vultr VMs are offline (James, Sun 8 Nov, 07:25), everything is checked locally from
the `platform` worktree. This is the node W0 runs after each merge.

## Start node

```sh
cd /Users/jameskaranja/Developer/hackathons/agentrix-platform
export CARGO_TARGET_DIR=/Users/jameskaranja/Developer/hackathons/.agentrix-platform-target
export AGENTRIX_DATA_DIR=$PWD/../.agentrix-local      # any empty folder; keys are made on first start
export AGENTRIX_ORIGIN=http://localhost:18050
export AGENTRIX_LISTEN=127.0.0.1:18050
export AGENTRIX_INTERNAL_LISTEN=127.0.0.1:18049
scripts/wcargo run -p agentrix-node --locked
```

Wait for `curl http://127.0.0.1:18050/healthz` to answer `ok`. With no runner set, sealed rooms
use a fake runner; with no Model Gateway on :8090, Ask says "Answers are paused right now".

## What to open

Browsers resolve any `*.localhost` name to this Mac, so hosts work without editing `/etc/hosts`.

| Page | Address | Notes |
|---|---|---|
| Home page (W10) | http://localhost:18050/home | Becomes `/` after the first release's handover |
| Paste an address (W4) | http://localhost:18050/start | Then Assess |
| Owner sign-up (W4, W6) | http://localhost:18050/account/new | Passkey first; email codes go to the outbox |
| Owner console (W5) | http://localhost:18050/os | After sign-in; the first release's console stays at `/console/today` |
| Messages outbox (W9) | http://localhost:18050/console/messages | Every message, marked Not sent until a provider is set |
| Status (W8) | http://localhost:18050/status | Health, gauntlet, spend |
| Operator console (W3) | http://ops.localhost:18050/ | Needs `operators.txt` in `$AGENTRIX_DATA_DIR/agentrix-operator/` |
| Kilele Bakery sample (W2) | http://kilele-bakery-sample.localhost:18050/ | Sample band, desks, Ask, sign-in |
| Classy Perfumes, Tana Valley Times | `classy-perfumes-sample.localhost`, `tana-valley-times-sample.localhost` | Answer once W2's next slices land |

## Model calls

Run `agentrix-models` in replay mode (`AGENTRIX_MODELS_MODE=replay`) to answer from
recordings; tests never call live models (cost.md §4). Live calls need the provider keys, which
live only in the gateway's settings, never in node.

## After the VMs come back

W12's restore runbook (cost.md) brings the stack back. Then the deploy order in
handover-wiring.md §0b: Deploy A (`platform-deploy-a`), then the checked `platform` head.
