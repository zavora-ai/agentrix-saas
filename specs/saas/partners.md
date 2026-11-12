# Partners and how to set them up

Every key goes into `agentrix-saas/.env`, which is git-ignored and mode 0600. Never paste keys into chat.
- Each key is **new and used only by this entry**, so it can be capped and revoked without touching the Vultr entry.
- The variable names are already in `.env`. The ERA secrets are generated.

| Partner | Role | Long-term for Agentrix | Hackathon prize |
|---|---|---|---|
| **PayPal** | Money rails: Subscriptions, Orders, Invoicing, Payouts, Vault, refunds, webhooks | Yes. The payment rail for Agentrix plans and for agent-to-agent settlement | The main prizes and Best Use of Agentic Commerce |
| **AG Grid** (sponsor partner) | Treasury board: AG Studio dashboard, grids and charts, with an adk-rust harness | Yes. Accounting work is grids, and ERA's React interface and the Agentrix console both use them | Best Use of AG Grid ($5K / $2K / 3 × $1K) |
| **Anthropic** | Claude Sonnet 5.5 and Haiku 5.5 | Yes, already the platform's second provider | — |
| **Vultr** | Inference (desks, extraction, embeddings, voice), the `saas-node` VM, DNS | Yes, already the platform's home | — |
| **HeyGen** | The Chief's presenter video (Avatar IV) | Possibly: the Chief's video presence on sites | Best Demo Delivery (indirectly) |
| **Zavora ERA + mcp-erp** | The books and CRM | Ours: ERA is proprietary, and mcp-erp is open source | — |

## 1. PayPal (developer.paypal.com, sandbox): about 15 minutes

Each company gets its own sandbox business account and its own REST app. Leave the shared app (the one agentrix.website uses) alone.

1. **Sandbox accounts.** Go to **Testing Tools → Sandbox Accounts → Create account**.
   - **Business**, country **United States**, named "Agentrix SaaS". (Entrants report buyer approvals failing for some non-US sandbox merchants.)
   - **Business**, United States, named "Kilele Bakery".
   - **Personal**, three accounts: two demo customers and one referral partner who receives a payout. Under *Edit account*, give each personal account a PayPal balance of about $5,000.
2. **App for Agentrix.** Go to **Apps & Credentials → Sandbox → Create App**.
   - Name it "Agentrix SaaS", type *Merchant*, and link it to the Agentrix SaaS business account.
   - Copy the Client ID and Secret into `PAYPAL_CLIENT_ID` and `PAYPAL_CLIENT_SECRET`.
   - Under the app's **Features**, turn on:
     - Accept payments, including **Vault** ("save payment methods")
     - Invoicing
     - **Payouts**
     - Subscriptions
     - Disputes
     - Transaction search
   - Leave webhooks empty. I register `https://pay.agentrix.website/paypal/webhook` once the VM is up, then fill `PAYPAL_WEBHOOK_ID`.
3. **App for Kilele.** Go to **Create App** again.
   - Name it "Kilele Bakery", link it to the Kilele Bakery business account, and turn on Accept payments and **Payouts**.
   - Copy its keys into `KILELE_PAYPAL_CLIENT_ID` and `KILELE_PAYPAL_CLIENT_SECRET`.
4. **Emails into `.env`.** Put the five sandbox emails in `.env`: `PAYPAL_MERCHANT_EMAIL`, `KILELE_PAYPAL_MERCHANT_EMAIL`, `PAYPAL_BUYER_EMAIL_1`, `PAYPAL_BUYER_EMAIL_2` and `PAYPAL_PARTNER_EMAIL`.
   - Their passwords stay in PayPal's dashboard.
   - When a demo step needs a sandbox buyer to approve, you sign in yourself.

## 2. AG Grid: about 10 minutes, on or after Tue 10 Nov

- **AG Studio trial.**
  - Start it at <https://www.ag-grid.com/studio/license-pricing/>: 45 days, full Enterprise, no watermark. Starting on 10 Nov runs it to about 25 Dec, past the winners' announcement.
  - Put the emailed key in `AG_STUDIO_LICENSE_KEY`.
- **AG Grid Enterprise.** If the Studio key doesn't cover it, start the Enterprise trial (30 days) and put that key in `AG_GRID_LICENSE_KEY`.
- **Reference.** PayPal's hackathon boilerplate is at <https://github.com/paypaldev/hackathon-paypal-ag-grid-boilerplate>. It uses Studio's built-in agents only. We go further and swap in our own harness, which is the prize's stated bar.
- **Devpost.** On the submission form, tick **AG Grid** under the sponsor tools.
- **After the hackathon.** AG Grid Community is MIT and free. Enterprise is about $999 per developer (perpetual, one year of updates). AG Studio is priced by quote.

## 3. Anthropic (console.anthropic.com): about 3 minutes

1. Go to **Settings → Workspaces → Create workspace** and name it `agentrix-saas`. Set a **monthly spend limit**; $50 is plenty.
2. Go to **API keys → Create key** in that workspace and put it in `ANTHROPIC_API_KEY`.

## 4. Vultr (my.vultr.com): about 5 minutes

1. **Serverless Inference.** Create a key for this entry and put it in `VULTR_INFERENCE_API_KEY`.
   - If the console allows only one key per subscription, tell me.
   - We can then reuse the existing key, with its spend shown together with the Vultr entry's, or add a second subscription.
2. **Account API key.** Go to **Account → API**. Put the key in `VULTR_API_KEY`, and allow my current IP if access control is on. I use it for two things, each only after you approve the spend:
   - **Create `saas-node`.** 4 vCPU / 8 GB, roughly $40 a month; I couldn't confirm Vultr's current price.
   - **Add four A records** in the `agentrix.website` zone: `money`, `kilele.money`, `pay` and `books`. Nothing else in the zone changes.

## 5. HeyGen (heygen.com): about 5 minutes, needed Wednesday

1. Take the **Creator** plan ($29 a month, about 30 minutes of 1080p). Cancel it after December.
2. Go to **Settings → API**, create a key and put it in `HEYGEN_API_KEY`.
3. I prepare the Agentrix mascot image for a Photo Avatar (Avatar IV). The video carries an "AI presenter" label, and the YouTube upload is marked as synthetic content.

## 6. Nothing to sign up for

- **Zavora ERA.** The demo instance is built from `~/Developer/projects/zavora-era` and runs on `saas-node` at `books.agentrix.website`. Its secrets are already generated in `.env`. Its code stays private.
- **mcp-erp.** Open source (Apache-2.0). It's the bridge from the agents to ERA.
- **GitHub.** I create the public repo `zavora-ai/agentrix-saas` (MIT, with the licence shown in About) when it's time to publish, and only with your OK.
- **Devpost.** You're already registered. The submission is draft 1283686.

## Optional

- **ElevenLabs Starter** (about $6): only if Vultr's text-to-speech voice isn't good enough for the Chief.
- **APIMatic:** skipped. One project can win only one sponsor prize, and AG Grid is ours.
