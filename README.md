# Agentrix

**A payments ecosystem of autonomous agents, on WebAgents, adk-rust and PayPal.**

Agentrix turns a small business's website into a team of AI agents (WebAgents). Each agent can be
identified by anyone, answers to a human who holds a passkey, and leaves a PayPal record anyone can
check. The agents sell, bill, collect, pay staff and suppliers, and settle with other companies'
agents, inside limits their owner signed.

This is Zavora's entry for the PayPal AI Hackathon (November 2026). Everything runs on the PayPal
sandbox: no real money moves.

## See it running

| Open | What it is |
|---|---|
| [paypal.agentrix.website](https://paypal.agentrix.website) | The platform's home page |
| [For judges](https://money.paypal.agentrix.website/judges) | The story in five scenes, each with the PayPal ids and journals it left |
| [Kilele Bakery](https://kilele-bakery-sample.paypal.agentrix.website) | A sample shop run by its WebAgents. Customers' agents order here |
| [Kilele's agents](https://kilele-bakery-sample.paypal.agentrix.website/agents) | Each agent's badge: who it is, who answers for it, and its wallet record |
| [Kilele's owner view](https://paypal.agentrix.website/samples/kilele-bakery-sample/) | What the owner sees: the morning brief, orders, and the decisions that wait for her |
| [Agentrix's console](https://money.paypal.agentrix.website/console) | Agentrix is itself a company run by agents: its bills, mandate and settlements |
| [Proofs](https://money.paypal.agentrix.website/proofs) | The signed audit log, the agents' public keys, and ten attacks that were refused |
| [Status](https://paypal.agentrix.website/status) | Whether everything is working now |

The businesses are made up, to show what Agentrix does. The three shoppers (Amani, Baraka and
Chiku) are test agents we run; each has its own site, keys and saved sandbox PayPal.

## What happens without anyone at the keyboard

The agents have run unattended since Wednesday 11 November 2026, 19:13 EAT.

- **Customers' agents buy.** A shopper agent signs its request at the shop's agent door, orders,
  and pays with its saved PayPal. The shop's Cashier captures the payment and the books post it.
- **Companies' agents settle with each other.** Agentrix bills Kilele for its WebAgents. Kilele's
  Payables agent checks the invoice against the mandate its owner signed and pays it. Both
  companies' books post from the same PayPal id.
- **Bills fall due.** Staff payroll, and what the models cost, are raised as bills and paid
  through PayPal Payouts inside the same mandate.
- **The owner is asked only when it matters.** A custom cake, a new payee or an amount above the
  mandate waits for the owner. Nothing is charged until she decides.
- **Everything is written down.** Each company keeps a signed audit log. Each wallet entry carries
  PayPal's id and the journal in the books.

## Three ideas

1. **Know your agent.** Every agent has a badge: a W3C DID (`did:web`) rooted in its site's own
   domain and signed by the owner's key. Agents sign every request to another company (Web Bot
   Auth, RFC 9421 HTTP message signatures). An agent that can't prove who it is is refused before
   any PayPal order exists. Each site is also an OpenID Connect issuer for its agents, so the same
   identity is accepted by AWS IAM, Amazon Bedrock AgentCore Identity, Google Cloud workload
   identity federation and Microsoft Entra Agent ID. We checked each one live:
   [`specs/saas/identity-validation.md`](specs/saas/identity-validation.md).
2. **A human answers for it.** The owner's passkey is asked for at three moments only: linking
   PayPal, signing or raising the mandate, and stepping up for something outside it. The signature
   covers exactly what is shown. Change the amount afterwards and the approval is void.
3. **The money is checkable.** The money stays in PayPal. The wallet is a signed record of every
   move an agent made with it, matched three ways: the wallet entry, PayPal's id, and the journal
   in the books (Zavora ERA, hosted).

## How PayPal is used

| PayPal product | Where |
|---|---|
| Orders v2 | Checkout on the sample sites; agent-to-agent invoices |
| Vault (saved payment methods) | A customer's agent, or another company, pays without a person signing in each time |
| Payouts | Payroll, supplier bills, and the fallback when a saved PayPal can't be used |
| Subscriptions | Agentrix's own plans (USD 29, 79 and 149 a month) |
| Refunds | Only ever to the person who paid, read from PayPal's capture record |
| Webhooks | Verified with PayPal over the exact bytes received. A forged event changes nothing |

The sandbox ids behind each of these are in [`specs/saas/EVIDENCE.md`](specs/saas/EVIDENCE.md), and
the edge cases we planned for are in
[`specs/saas/paypal-edge-cases.md`](specs/saas/paypal-edge-cases.md).

## How AI is used

Every agent is an [adk-rust](https://github.com/zavora-ai/adk-rust) agent running under a kernel,
with guardrails, memory, a mandate and a signed audit log. The Chief of each company plans and
answers its owner; the desks (Revenue, Receivables, Payables, Support, Cashier and the site's
front desk) do one job each. Larger models handle the Chief and negotiation; smaller ones handle
routine turns. A tool call that moves money is checked by rule, never by the model's word:

- the payee must be the one the books hold;
- the amount must sit inside the owner's mandate, per payment and per day;
- a refund goes only to the person who paid;
- a cart mandate moves money once.

The attack suite tries to break each rule and scores the result from PayPal, the books and the
audit log, never from the conversation: [`specs/saas/PROOFS.md`](specs/saas/PROOFS.md).

## Check it yourself

```sh
git clone https://github.com/zavora-ai/agentrix-saas && cd agentrix-saas
cargo run -q -p saas-node --bin agentrix-saas --locked -- audit verify specs/saas/proofs/audit/agentrix.jsonl --key-file specs/saas/proofs/audit/agentrix.vkey
```

Flip one byte in the log and run it again: it fails. You need [Rust](https://rustup.rs); the first
build takes a few minutes.

## What is in this repository

| Path | What |
|---|---|
| `crates/saas-*` | The money layer: books, company agents, settlement, badges, federation, wallet, test agents |
| `crates/agentrix-*` | The WebAgent platform: sites, owner console, kernel, sandboxed jobs, models gateway |
| `specs/saas/` | The specification, the evidence and the proofs for this entry |
| `deploy/saas/` | The single server the judged build runs on |
| `deploy/k8s/` | A Kubernetes cluster for one isolated node per business (edge proven; no business runs there yet) |
| [`docs/platform.md`](docs/platform.md) | The WebAgent platform's own guide, including how to run it on your machine |

## What we gave back to adk-rust

Work this entry needed went upstream first, each with an issue, tests and a changelog entry:

- Agent identity federation in `adk-auth`: an agent token issuer, and exchanges with AWS IAM,
  AgentCore Identity, Google Cloud and Microsoft Entra Agent ID
  ([zavora-ai/adk-rust#1362](https://github.com/zavora-ai/adk-rust/pull/1362)).
- A fix so the single sign-on token validator honours every configured algorithm
  ([zavora-ai/adk-rust#1366](https://github.com/zavora-ai/adk-rust/pull/1366)).

## Honest limits

- Sandbox only. The businesses, customers and staff are made up.
- The mandates are AP2-style: they follow the shape of Google's Agent Payments Protocol, and we
  don't claim conformance.
- The owners' passkeys for the two money consoles were not yet set up when this was written, so
  those pages say so.
- Suppliers, a courier and riders as separate businesses are designed
  ([`specs/saas/v4-ecosystem.md`](specs/saas/v4-ecosystem.md)) and not built.

## Licence

MIT. Copyright (c) 2026 Zavora Technologies Ltd.
