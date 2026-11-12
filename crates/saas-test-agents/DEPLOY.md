# Test customer agents: deploy notes

Three scripted shoppers (Amani, Baraka, Chiku), labelled "Test customer agent" everywhere they show, on sandbox PayPal only.

## The unit

- Binary: `agentrix-test-agents`, from `scripts/wcargo build --release -p saas-test-agents`.
- Run it as its own systemd unit beside the node. Give it the node's environment file plus the settings below.
- Smoke test: `agentrix-test-agents once amani` makes one visit now and prints each step;
  `agentrix-test-agents once chiku tana-valley-times-sample` visits that site (Tana subscribes,
  searches stories or asks; Kilele and Classy quote, check out and pay).

| Setting | Value |
|---|---|
| `TEST_AGENTS_PLATFORM_HOST` | `paypal.agentrix.website` |
| `TEST_AGENTS_LISTEN` | `127.0.0.1:18190` (default) |
| `TEST_AGENTS_DATA_DIR` | under the node's data dir, outside the repo. It holds keys (0600), saved buyers, model spend and `activity.jsonl` |
| `TEST_AGENTS_CONTROL_TOKEN` | a new random secret, for `/control/stop` and `/control/start` |
| `PAYPAL_ENV`, `PAYPAL_CLIENT_ID`, `PAYPAL_CLIENT_SECRET` | sandbox. Use Agentrix's app, the same one node registers for the samples' payments |
| `VULTR_INFERENCE_BASE_URL`, `VULTR_INFERENCE_API_KEY`, `AGENTRIX_TIER_FAST_VULTR` | the model |
| `TEST_AGENTS_DAILY_MODEL_USD` | each agent's model budget per UTC day (default 0.25). When it's spent, choices are random |
| `TEST_AGENTS_HOURS` | how long the run lasts (default 12). It stops by itself after that |
| `TEST_AGENTS_OFF_FILE` | default `<data dir>/off`. While the file exists, the agents pause |

## Caddy

Proxy these four hosts to `127.0.0.1:18190`, keeping the `Host` header:

- `test-agent-amani.paypal.agentrix.website`
- `test-agent-baraka.paypal.agentrix.website`
- `test-agent-chiku.paypal.agentrix.website`
- `test-agents.paypal.agentrix.website`, the control page

## What each host serves

- `https://test-agent-<name>…/`: the agent's labelled page.
- `/.well-known/did.json`: its `did:web:test-agent-<name>.paypal.agentrix.website`.
- `/.well-known/http-message-signatures-directory`: its key directory.
- `/paypal/save`: James approves the agent's sandbox buyer here, once, signed in to a PayPal sandbox **buyer** account.
- `https://test-agents…/`: what the agents did lately, and whether they are on.

## Off switch

- `curl -X POST -H "Authorization: Bearer $TEST_AGENTS_CONTROL_TOKEN" https://test-agents.paypal.agentrix.website/control/stop`, or `touch <data dir>/off`.
- `/control/start`, or removing the file, starts them again.

## What node must have for agents to pay

Agents pay only when node has these, and only after James has saved a buyer for that agent. Until then each checkout is logged with the reason it wasn't paid.

- `agentrix_samples::set_payments(SamplePayPal over Effects on ctx.pool, Agentrix's PayPal client)`
- `agentrix_samples::set_agent_proof(...)`
- the identity change that shows a test agent's code on the page
