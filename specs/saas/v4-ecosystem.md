# The ecosystem run (spec v4)

**Written Wed 11 Nov 2026, 17:15 EAT**, from James's direction that afternoon. The hard deadline is
Thu 12 Nov, 23:00 EAT; we submit at 18:00. That leaves about 24 hours of working time.

**The pitch, in James's words:** "a payments ecosystem of autonomous agents using webagents and
adk-payments and paypal". Websites run by agents trade with other websites run by agents, for hours,
on real PayPal sandbox money. Everything is recorded and can be replayed. The Chief of a business
runs it for real and then presents what happened.

v4 builds on v3 and replaces its video plan. Identity, the passkey and the wallet stay; they are now
the rules of the ecosystem and not the story.

## What must be true, or it is not this pitch

1. **Nothing is emulated.** Every payment is a PayPal sandbox object read back from PayPal. Every decision is made by an agent on the governed path. Where PayPal's own sandbox error simulation is used, the screen says so.
2. **Websites pay websites.** A shop's agents buy from a supplier's agents with no person in the loop, inside mandates both owners signed.
3. **It runs unattended for hours**, and survives running out of cash.
4. **It can be replayed** from the records alone.

## The world

Every business is marked Sample. Each has an owner, a Chief, and the desks it needs.

| Business | Kind | Sells | Buys |
|---|---|---|---|
| Agentrix | Platform | Hosting plans to every site | Model usage, its server |
| Kilele Bakery | Shop (exists) | Bread and cakes to customers | Flour and supplies; hosting; staff |
| Classy Perfumes | Shop (exists) | Perfume to customers | Wholesale fragrance; hosting; staff |
| Tana Valley Times | Shop (exists) | Subscriptions and adverts | Cameras and gear; hosting; staff |
| A flour and baking supplier | Supplier (new) | To Kilele | Hosting |
| A fragrance wholesaler | Supplier (new) | To Classy | Hosting |
| A camera and gear supplier | Supplier (new) | To Tana Valley Times | Hosting |
| A courier | Vendor (new), with its own site | Deliveries to every shop, each with a tracking number | Hosting; its riders' share of every fee |
| Riders | People, each represented by an agent with its own badge | Deliveries, to the courier | – |
| Customers | Test customer agents (three exist) | – | From the shops |
| Staff | People with PayPal sandbox accounts | Their work | – |

## The money that moves

| Flow | PayPal | Who decides | State today |
|---|---|---|---|
| A customer's agent prepares an order; the customer pays | Orders v2 | Customer agent, within its limits | Sample sites take real sandbox PayPal (merged today) |
| A returning customer's agent pays from a saved PayPal | Vault, then capture | Customer agent, inside the customer's mandate | Vault rail exists between two companies |
| A shop runs low and reorders from its supplier | Invoice over A2A, settled by Vault capture or Payout | Shop's Cashier and Chief; supplier's Receivables | Exists for one pair (Agentrix and Kilele) |
| A shop books a delivery for a paid order | The courier's fee, captured from the shop's saved PayPal at booking; refunded by the courier if the delivery fails | Shop's agent books; courier's Dispatch quotes and accepts | New |
| The courier pays the rider's share on each confirmed delivery | Payouts, one per delivery | Courier's Cashier, by the published split; checked by the rider's agent | New |
| A shop needs equipment for an order | Same as above, larger, so it is held for the owner | Chief proposes, owner's passkey | Step-up exists |
| Hosting | Subscriptions: start, upgrade, downgrade, cancel, a failed renewal | Site's Chief; Agentrix's Revenue | Subscriptions client exists |
| Payroll at month end, and other admin bills | Payouts | Shop's Cashier, inside the mandate | Payouts rail exists |
| Refunds | Refunds on the capture | Support desk; never above the capture | Exists |
| Capital in | A payout from the owner's account to the business | Owner, with a passkey | New |

## Autonomy, and what the Chief really does

- **Standing duties of each Chief:** read the cash position every business hour; decide what gets paid first when cash is short; reorder when stock or equipment blocks an order; brief the owner each morning; escalate only what the mandate does not cover.
- **Cash running out is part of the run.** Shops start with small PayPal balances. When a payout or capture fails for lack of funds, the Chief must choose: pay staff before suppliers, ask a supplier for instalments (built), delay a reorder, or ask the owner for capital. The owner answers with a passkey, and the top-up is a real payment in.
- **The business calendar runs fast:** one business day every 30 minutes, so ten hours cover a month and one payroll. PayPal's own clocks are real; the screen shows both.
- **Owners are people.** Held decisions wait for a passkey. Overnight they pile up; in the morning the owner clears them. That is filmed.

## After the sale: delivery, refunds and complaints

**James, Wed 11 Nov, about 19:45 EAT:** "do we have deliveries done with tracking and also do we have refunds and complaints?" Checked in the code and on the server the same hour.

| Thing | What exists | What is missing for the run |
|---|---|---|
| Delivery | Orders have states (`awaiting_payment`, `paid`, `baking`, `ready`, out for delivery, `delivered`, `collected`, `cancelled`, `refunded`, `held`), delivery and collection slots, customer notices for "out for delivery" and "delivered", and a status check by order number on the site and at the agent door | Only the owner moves an order on, by hand, in the owner's view. Nothing does it in the run: the first paid order, KB-1052, was still `paid` half an hour later. There is no courier or rider, and no tracking beyond the status word |
| Refunds | The owner can cancel an order with a refund; each site has a refund rule; the PayPal refund call exists and never exceeds the capture | No refund has happened in the run. A customer's agent has no way to ask for one |
| Complaints | Nothing by that name. A customer can ask the desks a question or use "Talk to the owner" | No complaint flow, no escalation from a complaint, none in the run |

**What the run needs:**
- **Fulfilment by the shop's agents.** On the business calendar an order moves `paid` → `baking` → `ready` → out for delivery → `delivered` (or `collected`), in its slot. Each step is a record with a time and the agent that did it, and the customer's agent is told. The order's page shows that timeline; that is the tracking.
- **Delivery is bought from another business.** James, Wed 11 Nov, about 19:50 EAT: "let the tracking be another vendor and do proper payments - has to simulate real life - orders and payments made to the courier site". So:
  1. When an order is ready, the shop's agent books a delivery at the courier's agent door, as a signed request: pickup, drop-off zone, slot, parcel.
  2. The courier's Dispatch agent quotes from its own price list and accepts. The fee is captured from the shop's saved PayPal at booking, a real payment from one business to another, booked in both sets of books.
  3. The courier issues a tracking number and owns the tracking: booked, picked up, on the way, delivered or failed, each with a time. Its site has a tracking page; the shop's order page shows the same events, read from the courier, not copied by hand.
  4. "Delivered" needs the customer's agent to confirm it. A failed delivery is the courier's to answer for: it refunds the fee or re-attempts, by its own rule.
  5. **The courier splits each fee with the rider.** James, about 19:50 EAT: "vendors also need to split their payments with their riders (represented by agents)". Each rider is a person with a PayPal account and an agent that acts for them. The rider's agent has its own badge, takes or declines a job offered by Dispatch, and signs the pickup and delivery events that become the tracking. When the customer's agent confirms delivery, the courier's Cashier pays the rider's share by payout, the same hour, and keeps its commission. The rider's agent checks the amount against the agreed split and raises it with the courier if it is short. The shops have no rider.
  6. **The split is a rule the courier publishes** (for example 70% to the rider), so both agents compute the same number. A failed delivery pays the rider nothing, or an attempt fee, by that rule.
- **Customers who are not happy.** The test customer agents get three more things to do, at a low rate: cancel before baking, ask for a refund after delivery with a reason, and complain without asking for money.
- **The Support desk decides inside the refund rule:** refund in full, in part, or decline, and says why. Anything outside the rule, and any complaint that repeats, goes to the owner as a held decision.
- **Each refund is a real PayPal refund on the original capture,** booked in ERA and the wallet, and the customer's agent sees it land.

## PayPal coverage and edge cases

Each one is triggered for real in the sandbox unless marked.

| Case | How it happens |
|---|---|
| Verified webhooks driving every state change | Already the rule |
| The same webhook delivered twice, and out of order | Ask PayPal to resend an event |
| Subscription upgrade, downgrade, cancel, suspend | API calls by the agents; PayPal's events follow |
| A renewal that really charges | Daily plans started tonight renew once before submission |
| Insufficient balance on a payout | Small starting balances |
| A declined saved PayPal on a capture | Drained buyer account; or PayPal's sandbox error simulation, marked as such |
| A partial refund, and a refund the balance cannot cover | Support desk |
| A payout to an address with no PayPal account | Stays unclaimed; the Cashier cancels it |
| A crash in the middle of a payout | Kill the process; it pays once (proof exists) |
| A rate limit | Retried with backoff |
| A forged webhook; an impostor agent | Refused (proofs exist) |

## Recording and replay

- **The record is what already exists:** each company's signed audit log, the kernel's events, the wallet, ERA, and PayPal's ids. Nothing is written for the camera.
- **Replay** is the live activity page fed from those records with a time cursor and a speed control: the map of businesses, money moving between them, each business's cash, and the card for any moment (who, which badge, which mandate, which PayPal id, which journal).
- **The video is cut from the replay,** plus the owner clearing the morning's decisions. The Chief writes the narration from its own log; every number in it is checked against the record.

## The run grows while it runs

There is no time for one big switch-over, and an ecosystem that grows is the truer story.

| When (EAT) | State of the run |
|---|---|
| Wed 23:00 | **Start.** Whatever is green: Agentrix, Kilele, the test customer agents, real sandbox payments, alerts, the recorder. No restart wipes the record after this |
| Through the night | Suppliers and the other two shops join as they are merged and deployed. Each joining is itself in the record |
| Thu 07:00 | Owner clears held decisions. Cash-out and capital-in must have happened at least once by now |
| Thu 09:00 | **Stop adding.** Replay and narration from the record |
| Thu 13:00 | Video uploaded |
| Thu 18:00 | Submit |

## What has to be built

| Slice | What | Notes |
|---|---|---|
| One node per business | The node runs one business, named in its configuration, and reaches every other business only through that business's public door | Replaces "many companies in one process". See the next section |
| Supplier sites | A plain company front with a catalogue, a sales desk and an agent door for each supplier | Not the bespoke sample-site design |
| Chief duties | Cash review, payment priority, reorder, capital request, morning brief | On the existing kernel schedules |
| Stock and needs | A shop's stock and equipment needs raise reorder events | Sample sites already track stock |
| Subscriptions | Hosting as PayPal subscriptions with upgrade, downgrade and cancel by the agents | Client exists |
| Payroll and admin bills | Month-end payouts to staff; hosting and model bills | Model-spend bills merged today |
| Capital in | Owner's passkey, then a payout from the owner's account | New |
| Edge cases | The table above, on a schedule through the run | |
| Recorder and replay | Time cursor over the existing records; the economy map | From the live activity page |
| Narration | The Chief's script from its log, with every claim checked | |

## One isolated node per business

**James, Wed 11 Nov, about 17:20 EAT:** "how about using kubernetes architecture and adk-sandbox for
isolation and multi nodes?" Adopted as the architecture. The tooling for tonight is narrower.

- **Each business is its own node:** its own process, keys, kernel state, audit log, PayPal app and ERA tenant. Businesses meet only at their public doors, with signed requests. Adding a business is adding one entry to a manifest.
- **Why this is the smaller change.** The two desks already call each other over HTTP by door URL (`SettleHostConfig` holds a payee, a payer and each one's `door_url`). What is hard-wired is that both seats are built in one process under two fixed names. The change is: a node builds its own seats (payee towards its customers, payer towards its suppliers) and takes the other side's door URL from the manifest.
- **Isolation, in three layers, each with what really provides it:**

| Between | Provided by | State |
|---|---|---|
| One business and another | One container per business under gVisor (`runsc` is already installed on the server), its own data volume and secrets file, and an outbound allow-list (`agentrix-egress`): PayPal, its books, the model gateway, its counterparties' doors | To wire |
| An agent and the business's secrets | The credential broker and per-agent tokens | Built |
| An agent and what it reads or runs (a supplier's PDF, a customer's message, code) | `adk-sandbox`, with no network | Built for vendor documents |

- **Kubernetes, as a second site.** James, Wed 11 Nov, 17:36 EAT: "Yes, create it". A managed cluster on Vultr runs beside the server; the server is not moved into it.
  - **Cluster:** `agentrix-saas-eco`, Vultr Kubernetes Engine, Atlanta, Kubernetes 1.37.2, one pool `businesses` of three nodes with 2 vCPU and 4 GB each. Created 17:37 EAT.
  - **Registry:** `ord.vultrcr.com/agentrixsaas`, private, the free tier (10 GB).
  - **Who lives where:** Agentrix and Kilele stay on the server, which is recording. The suppliers, then Classy Perfumes and Tana Valley Times, run in the cluster, one pod per business with its own storage, secrets and network policy. ERA stays on the server; cluster businesses reach it over HTTPS.
  - **Not available there:** gVisor on the managed nodes, and `adk-sandbox`'s container mode inside a pod. Isolation in the cluster is pods, namespaces and network policy.
  - **It must stay up unchanged** until about 21 Dec, like the server. About USD 100 for that time.
  - **Access:** the cluster's access file and ids are in `~/.config/agentrix-saas/k8s/` on James's Mac, never in this repo.
  - **Edge:** Caddy 2.11.9 in the cluster (`deploy/k8s/edge/`), behind one load balancer, obtaining certificates itself. Proven at 17:55 EAT: `https://edge-check.paypal.agentrix.website` answers with a Let's Encrypt certificate. A business's host gets an explicit DNS record to the load balancer and one site block in `routes.yaml`. ingress-nginx is not used; its project archived it in March 2026.
  - **Until a business is seen trading from a pod,** the write-up says "designed to run one pod per business".
- **What it buys the video:** stop one business's container in the middle of the run. The others keep trading, its supplier's invoice waits, and when it comes back it catches up from its own records.
- **Cost:** seven nodes and ERA on 4 vCPU and 8 GB will be tight. Resizing the server is James's call.

## What James has to set up (only he can)

1. **PayPal sandbox business accounts with their own apps** for Classy Perfumes, Tana Valley Times, the three suppliers and the courier, each with a small starting balance (I suggest USD 300 for shops, USD 1,000 for suppliers). Agentrix and Kilele exist.
2. **Saved-PayPal approvals** (one sandbox sign-in each): each shop with its supplier, each shop with the courier, each shop with Agentrix. About ten. The three customer agents are done.
3. **Subscription approvals** for hosting: one per site.
4. **Sandbox personal accounts for staff** payroll. The three existing addresses can serve.
5. **The model budget for the run.** The fleet cap is USD 10 a day; seven businesses for ten hours need more.
6. **Disk.** 12 GB free against the 15 GB bar.
7. **The HeyGen key**, if the Chief is to present as the avatar.

## Honest risks

- **Time.** Twenty-four working hours for a multi-business runtime, an overnight run and a video. The plan survives only because the run can start small and grow.
- **The many-companies change touches every slice at once.** It is the likeliest thing to break a build that works today. It must land on a branch and be proven before it replaces the node that is recording.
- **Approvals need James at a keyboard** tonight for about 40 minutes.
- **The sandbox** has outages and rate limits; a long run will meet them. That is acceptable only because the record shows them honestly.
- **What v3 promised and is not yet seen working** (the owner console walk, a real payment end to end, the cloud visitors) is still open. Cloud visitors become one of the customer kinds if there is time.
