# The submission, ready to paste

For the PayPal AI Hackathon form on Devpost. Deadline Thu 12 Nov 2026, 23:00 EAT (12:00 PST).
Every number below was read from the live build or from PayPal's sandbox on Thu 12 Nov at 08:20
EAT ([`video/record.json`](video/record.json)). Read them again before submitting: the agents are
still working.

## Name

Agentrix

## Tagline

A payments ecosystem of autonomous agents: websites run by AI agents you can identify, with an
owner who answers for them, and money you can check.

## Links

| Field | Value |
|---|---|
| Try it | https://paypal.agentrix.website |
| For judges | https://money.paypal.agentrix.website/judges |
| The night, replayed | https://zavora-ai.github.io/agentrix-saas/specs/saas/video/replay.html |
| Code | https://github.com/zavora-ai/agentrix-saas (MIT) |
| Video | (the YouTube link, once uploaded) |

## About the project

### Inspiration

A small business's website waits. It can't answer at night, chase an invoice, pay a supplier or
notice that cash is short. AI agents can do all of that, and soon one company's agents will pay
another company's agents. Before anyone lets that happen, three questions need answers that don't
depend on trusting the agent: **who is this agent, who answers for it, and where did the money
go?**

### What it does

Agentrix turns a business's website into a team of AI agents (WebAgents) and gives each one what
it needs to handle money safely.

- **A badge.** Every agent has a W3C DID rooted in its site's own domain and signed by the owner.
  Agents sign every request to another company (Web Bot Auth, RFC 9421). Anyone can verify a
  badge; an agent that can't prove itself is refused before a PayPal order exists.
- **An owner with a passkey.** The owner signs a mandate: how much an agent may pay, per payment
  and per day, and to whom. Inside it the agents act alone. Outside it (a new payee, a larger
  amount, a custom order) the decision waits for the owner.
- **A wallet you can check.** The money stays in PayPal. The wallet is a signed record of every
  move, matched three ways: the wallet entry, PayPal's id, and the journal in the books.

Then we let it run. From Wednesday 19:13 EAT, with nobody at the keyboard, three shopper agents
placed 15 orders at Kilele Bakery's site and paid 13 of them with their saved PayPal (USD 99.41).
Two custom cakes came through the agent door with verified signatures and were held for the
owner. Every three hours Agentrix's agents invoiced Kilele's agents, who checked the invoice
against the owner's mandate and paid on PayPal: four settlements, each posted to both companies'
books from one PayPal id. Staff payroll and the cost of the models were raised as bills. The
Cashier's wallet holds 52 entries, every one matched against PayPal.

On the judges' page one button, **Run the day**, plays five scenes live on the sandbox: a
subscription, an invoice settled agent to agent, an instalment plan the two companies' agents
agree when one is short of cash, a payout inside the mandate with a larger bill held for the
owner, and a "our bank details changed" email that is refused and written to the signed audit
log.

### How we built it

- **adk-rust** for every agent: a kernel per company, graph flows, guardrails, memory, governed
  tool calls and a signed audit log. Rules decide whether money may move, never the model's word.
- **PayPal**: Orders v2 for checkout and agent-to-agent invoices; Vault so a customer's agent or
  another company pays without a person signing in each time; Payouts for payroll, suppliers and
  bills; Subscriptions for Agentrix's own plans; refunds only to the person who paid; webhooks
  verified with PayPal over the exact bytes received.
- **Zavora ERA** (hosted) for the books and CRM, reached through the open-source `mcp-erp`.
- **Identity**: `did:web` badges, HTTP message signatures, and each site as an OpenID Connect
  issuer for its agents. We checked the same identity live against AWS IAM, Amazon Bedrock
  AgentCore Identity, Google Cloud workload identity federation and Microsoft Entra Agent ID.
- **Mandates** are AP2-style: they follow the shape of the Agent Payments Protocol's intent and
  cart mandates. We don't claim conformance.

### Challenges

- Making "the agent paid" provable. A model saying it paid is worth nothing, so every claim is
  scored from PayPal, the books and the signed audit log.
- Approvals that can't be bent. The owner's signature covers the exact call. In our attack suite
  the founder approved USD 800 and the agent then asked for USD 750: the edit was held as a new
  call and the approval was void.
- Two companies' agents agreeing a debt plan inside two owners' mandates, with neither side able
  to promise what its owner hadn't signed.

### What we're proud of

- It ran unattended overnight and the record adds up.
- Ten attacks, from a fake supplier email to a forged PayPal webhook: ten refused.
- The identity work is upstream in adk-rust (zavora-ai/adk-rust#1362 and #1366), so any adk-rust
  agent can use it.

### What we learned

Autonomy is easy to demo and hard to trust. The useful work was in the limits: what an agent may
do alone, what waits for a person, and how a stranger checks both.

### What's next

Suppliers, a courier and riders as their own businesses, each on its own isolated node, meeting
only at their public agent doors (designed in `specs/saas/v4-ecosystem.md`). Then real customers:
plans start at USD 29 a month.

## Built with

adk-rust, Rust, PayPal (Orders v2, Vault, Payouts, Subscriptions, Webhooks), Zavora ERA, mcp-erp,
W3C DID (did:web), HTTP message signatures (RFC 9421), OpenID Connect, WebAuthn passkeys, AG Grid,
Vultr (compute, Kubernetes, inference), Anthropic models, Caddy.

## Testing notes for judges

1. Open https://money.paypal.agentrix.website/judges and press **Run the day**. No sign-in is
   needed. Each scene shows the PayPal ids and journals it left.
2. Open https://paypal.agentrix.website/samples/kilele-bakery-sample/ for the owner's view of a
   shop. The sample's sign-in is filled in on the page.
3. To check an agent's badge or try an impostor request from a terminal, run
   `specs/saas/video/verify.sh` and `specs/saas/video/impostor.sh` from the repository.
4. Everything is the PayPal sandbox. To pay as a customer, use this sandbox buyer, never your
   own PayPal account: **(James: the sandbox buyer's email and password go here, in Devpost's
   private testing field only, never in the repository.)**

## The video on YouTube

- **Title:** Agentrix: a payments ecosystem of autonomous agents (PayPal AI Hackathon)
- **Description:** Agentrix turns a business's website into a team of AI agents that sell, bill,
  collect, pay and settle with other companies' agents on PayPal, inside limits the owner signs.
  Presented by Agentrix's Chief, an AI agent. Everything runs on the PayPal sandbox; no real
  money moves, and the businesses and shoppers are made up. Narration: a text-to-speech model.
  Try it: https://paypal.agentrix.website · Code: https://github.com/zavora-ai/agentrix-saas
- **Visibility:** Public or Unlisted (Devpost accepts either). Not "made for kids".
