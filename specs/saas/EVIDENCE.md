# PayPal sandbox evidence

Real calls to PayPal's **sandbox** (`api-m.sandbox.paypal.com`), made through `crates/agentrix-effects`. Each row has the PayPal id and the status PayPal reported. Times are EAT, read from the machine's clock as each call ran. A range means the probe printed no time for that call, only for the calls on either side.

**Where these come from**

- **The probe.** Every row comes from `crates/agentrix-effects/examples/paypal_sandbox_probe.rs`, on branch `saas-s1-paypal-rails`.
- **Running it.** `scripts/wcargo run -p agentrix-effects --example paypal_sandbox_probe -- <command>`, where the command is `run`, `status`, `webhook-forged`, `lose-answer`, `capture`, `suspend`, `activate` or `payout`.
  - It reads the sandbox keys from `~/.config/agentrix-saas/paypal-sandbox.json` (mode 0600, outside the repo).
  - It keeps its own state in `~/.config/agentrix-saas/probe.db`.
  - It refuses any environment but the sandbox.
- **The sandbox app.** This is the shared one (the same one agentrix.website, console.adk-rust.com and sauti.cloud use).
  - Everything the probe made is named "Agentrix SaaS …".
  - No webhook was created, changed or simulated. The webhook check uses a made-up webhook id of the probe's own.

## Gate: the APIs answer in sandbox (due Tue 10 Nov 13:00 EAT)

**Met Mon 9 Nov, 22:52 EAT**, for every API on the list. Capture and refund answer, but a *completed* capture and refund wait on a buyer approval (see "Waiting on James").

| API | Answered | What it showed |
|---|---|---|
| OAuth token | yes | Client credentials accepted (every call below). |
| Catalog Products | yes | A product was made. |
| Subscriptions: plans | yes | The plan is ACTIVE. |
| Subscriptions: subscriptions | yes | The subscription is APPROVAL_PENDING, with an approval link. |
| Orders v2: create | yes | Two answers:<br>• with a PayPal wallet: PAYER_ACTION_REQUIRED<br>• with a card: refused, `PAYEE_NOT_ENABLED_FOR_CARD_PROCESSING` |
| Orders v2: capture | yes | `ORDER_NOT_APPROVED` until the buyer approves. Effects answers "You haven't approved the payment on PayPal yet. Nothing was charged." |
| Payments v2: refund | not yet | Needs a completed capture first. |
| Payouts: create, batch, item | yes | The batch went PENDING, then PROCESSING, then SUCCESS. The item went PENDING, then UNCLAIMED. |
| Payouts: idempotency | yes | PayPal refuses a second batch with the same `sender_batch_id`, whether the `PayPal-Request-Id` is the same or new. The refusal links the first batch. |
| Payouts: a lost answer, read back | yes | `Effects::reconcile_payout` found the same batch. There was no second batch. |
| Webhooks: verify-webhook-signature | yes | A forged signature gets FAILURE, and `Effects::handle_webhook` refuses it as NotVerified. |

## Rows

### Mon 9 Nov 2026: `run`

| When | API | What | PayPal id | Status |
|---|---|---|---|---|
| Mon 09 Nov 22:49:52 EAT | Catalog Products | product "Agentrix SaaS Starter (sandbox probe)" | `PROD-88C60228CE4036143` | created |
| Mon 09 Nov 22:49:53 EAT | Subscriptions | plan USD 19.00 a month, through `Effects::ensure_plan` | `P-5XJ129498C557980BNLZCHYA` | ACTIVE |
| Mon 09 Nov 22:49:54 EAT | Subscriptions | subscription `sub_01a1c1dc295673f692113c97ec6cbf08`, through `Effects::start_subscription` | `I-J53UUW5DF0KV` | APPROVAL_PENDING |
| Mon 09 Nov 22:49:54–22:50:00 EAT | Orders v2 | create order USD 12.00 with `payment_source.card`: PayPal's test cards …1111, …4658 and …4444 | none | 422 `PAYEE_NOT_ENABLED_FOR_CARD_PROCESSING` |
| Mon 09 Nov 22:50:00 EAT | Orders v2 | create order USD 12.00 with `payment_source.paypal`, through `Effects::create_payment` | `1EF274999Y578383Y` | PAYER_ACTION_REQUIRED |
| Mon 09 Nov 22:50:01 EAT | Payouts | payout USD 10.00 to `agentrix-saas-partner@example.com`, through `Effects::payout` (Executed) | batch `UGQN4VVND4DZJ` | PENDING |
| Mon 09 Nov 22:50:01–22:50:03 EAT | Payouts | the same request again: the same `sender_batch_id` and the same `PayPal-Request-Id` | none | 400 `USER_BUSINESS_ERROR`: "Batch with given sender_batch_id already exists" |
| Mon 09 Nov 22:50:03 EAT | Payouts | the same `sender_batch_id` with a new `PayPal-Request-Id` | none | 400 `USER_BUSINESS_ERROR`, linking `https://api.sandbox.paypal.com/v1/payments/payouts/UGQN4VVND4DZJ` |
| Mon 09 Nov 22:50:12 EAT | Payouts | GET the batch, through `Effects::payout_batch` | `UGQN4VVND4DZJ` | PROCESSING |
| Mon 09 Nov 22:50:13 EAT | Payouts | GET the item, through `Effects::payout_item` | item `AN6FP42Y3464L` | PENDING |

The payout's `sender_batch_id` and `PayPal-Request-Id` are both `a06ff50a97da3550b7174eec6a18669b4e9363a3619aac4d88eb2bbf814ddc48`, the hex of the approved effect's fingerprint.

### Mon 9 Nov 2026: `webhook-forged`, `capture`, `status`, `lose-answer`

| When | API | What | PayPal id | Status |
|---|---|---|---|---|
| Mon 09 Nov 22:51:46 EAT | Webhooks | verify-webhook-signature on a forged event, with the probe's own made-up webhook id | event `WH-AGENTRIX-SAAS-PROBE-FORGED` | `verification_status` FAILURE |
| Mon 09 Nov 22:51:47 EAT | Webhooks | `Effects::handle_webhook` on the same forged event | event `WH-AGENTRIX-SAAS-PROBE-FORGED` | refused: NotVerified (the route answers 401) |
| Mon 09 Nov 22:51:52 EAT | Subscriptions | GET the subscription, through `Effects::refresh_subscription` | `I-J53UUW5DF0KV` | pending_approval |
| Mon 09 Nov 22:51:52 EAT | Payouts | read back the payout, through `Effects::reconcile_payout` | batch `UGQN4VVND4DZJ`, item `AN6FP42Y3464L` | batch SUCCESS, item UNCLAIMED |
| Mon 09 Nov 22:52:07 EAT | Orders v2 | capture before the buyer approved, through `Effects::capture` | order `1EF274999Y578383Y` | NotExecuted: "You haven't approved the payment on PayPal yet. Nothing was charged." |
| Mon 09 Nov 22:52:33 EAT | Payouts | the payout's answer forgotten (row set to unknown, batch id cleared), then settled by `Effects::reconcile_payout` | `UGQN4VVND4DZJ` | Executed: batch SUCCESS, item UNCLAIMED, the same batch |

The item is UNCLAIMED because `agentrix-saas-partner@example.com` has no sandbox account. PayPal holds an unclaimed payout for 30 days, then returns it.

## What the sandbox taught us

- **This app takes no cards.** Orders v2 with `payment_source.card` answers `PAYEE_NOT_ENABLED_FOR_CARD_PROCESSING`. So a capture without a buyer login needs Advanced Credit and Debit Card payments switched on for the sandbox app. Until then, a sandbox buyer approves each order.
- **A repeated payout gets a refusal, not a replay.** For Payouts, PayPal does not replay the first `201` to a repeated `PayPal-Request-Id`. It refuses with 400 `USER_BUSINESS_ERROR` and links the first batch, and does the same for a new request id with the same `sender_batch_id`.
  - `Effects::payout` and `Effects::reconcile_payout` follow that link and read the first batch with GET.
  - The wiremock test `a_duplicate_sender_batch_id_returns_the_original_batch` uses the same error shape.
- **The lost-answer case holds against the real PayPal.** A payout whose answer was lost asks again with the identical request. PayPal links the first batch, and there is never a second one. This is the sandbox half of the "kill -9 mid-payout, paid exactly once" proof.

## Waiting on James

1. **Approve the subscription as the sandbox buyer.**
   - Link: `https://www.sandbox.paypal.com/webapps/billing/subscriptions?ba_token=BA-2LC80726P4890413P` (subscription `I-J53UUW5DF0KV`).
   - Then the probe runs `status` (it should read ACTIVE), then `suspend sub_01a1c1dc295673f692113c97ec6cbf08`, then `activate sub_01a1c1dc295673f692113c97ec6cbf08`.
2. **Approve the order as the sandbox buyer.**
   - Link: `https://www.sandbox.paypal.com/checkoutnow?token=1EF274999Y578383Y`.
   - Then the probe runs `capture 1EF274999Y578383Y`: a capture through `Effects::capture`, then a USD 5.00 refund through `Effects::refund`.
3. **Give a payout receiver**: the email of a sandbox *personal* account. Then `payout <email> 1000` shows the item reach SUCCESS rather than UNCLAIMED.
4. **Optional: card capture with no buyer.** Switch on Advanced Credit and Debit Card payments for the sandbox app, or for the dedicated "Agentrix SaaS" app. The probe's `run` then captures with PayPal's test card and refunds through `Effects::refund`.
5. **For a genuine signed webhook: the dedicated "Agentrix SaaS" sandbox app.** It needs its own webhook pointing at the demo node, with its id in the node's `paypal.json`. The shared app's webhooks stay untouched, so no real PayPal delivery reaches this code yet. The signed path is covered by the wiremock tests, and PayPal's check refused the forged event above.
