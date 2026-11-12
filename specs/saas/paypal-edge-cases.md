# PayPal edge cases for the ecosystem run

**Written Wed 11 Nov 2026, 18:35 EAT**, for `v4-ecosystem.md`. Each row says how the case is made to
happen in the sandbox, which call does it, and what the agents should do. "Have" means
`agentrix-effects`, `saas-settle` or `saas-test-agents` already makes that call.

**Three ways a case can be real, in order of preference:**
1. **It just happens** in a long run (a deploy while PayPal is delivering, a token expiring).
2. **We arrange the conditions** (a small balance, an address with no account) and PayPal answers as it would in production.
3. **PayPal's own sandbox error simulation:** the request header `PayPal-Mock-Response: {"mock_application_codes": "<CODE>"}`. PayPal documents it for Orders v2 (create, update, show, authorize, capture) and Payments v2 (capture, void, refund). The record must mark these "sandbox error simulation". It is not documented for Payouts or Subscriptions.

## Webhooks

| Case | How it is triggered | Call | What should happen | Have? |
|---|---|---|---|---|
| The same event delivered twice | Ask PayPal to send an event again | `POST /v1/notifications/webhooks-events/{event_id}/resend` | Handled once; the second delivery is recorded as a duplicate and changes nothing | Resend: no. Dedupe by event id: yes |
| Events out of order | Resend an older event (an order approval) after its capture has completed | Same call | State never moves backwards | Check the handlers |
| Delivery while the node is down | Happens on every deploy; PayPal retries (its docs say up to 25 times over 3 days) | – | Late events are applied in order of what they say, not when they arrive | Yes, by read-back |
| A forged event | Post a body with a bad signature to our endpoint | `POST /v1/notifications/verify-webhook-signature` (ours, on receipt) | Refused; nothing changes | Yes (proof exists) |
| Finding an event we missed | List what PayPal sent | `GET /v1/notifications/webhooks-events` | The hourly reconcile closes the gap | List: no |

## Customers paying shops

| Case | How it is triggered | Call | What should happen | Have? |
|---|---|---|---|---|
| A saved PayPal is declined | A buyer account with too little balance and no backup funding; or the error simulation with `INSTRUMENT_DECLINED` on capture | `POST /v2/checkout/orders/{id}/capture` | No goods promised; the customer's agent is told why and may retry later; nothing booked | Capture: yes. Handling: check |
| A checkout link nobody pays | A customer agent with no saved buyer, or a person who walks away | – | The order expires; stock that was held is released; the shop's agent does not chase more than once | Check |
| A partial refund | Support refunds one line of an order | `POST /v2/payments/captures/{id}/refund` with an amount | Refund booked against the same capture; the wallet shows both | Yes |
| A refund larger than what is left | Ask for a second refund above the remainder | Same call | Refused by our rule before PayPal is called; if it reaches PayPal, PayPal refuses too | Rule: yes |
| A refund the balance cannot cover | Refund when the shop's balance is near zero | Same call | The Chief sees it as a cash problem, not a support problem | Not tried |

## Businesses paying businesses, staff and bills

| Case | How it is triggered | Call | What should happen | Have? |
|---|---|---|---|---|
| Not enough balance for a payout | Shops start with small balances; payroll and a supplier bill fall due together | `POST /v1/payments/payouts` | PayPal refuses or denies the batch. The Chief chooses: staff first, then ask the supplier for instalments or the owner for capital | Payout: yes. The choice: new |
| A payout to an address with no PayPal account | One staff address is not a sandbox account | `POST /v1/payments/payouts`, then `GET /v1/payments/payouts-item/{id}` | Item stays `UNCLAIMED`; the Cashier cancels it and tells the owner | Show item: yes. Cancel (`POST /v1/payments/payouts-item/{id}/cancel`): no |
| The same payout asked for twice | Repeat a call with the same batch id | `POST /v1/payments/payouts` | PayPal answers with the first result; paid once | Yes (proof exists) |
| A crash in the middle of a payout | Kill the node after the call leaves | – | On restart the outcome is unknown, then read back from PayPal; paid once | Yes (proof exists) |
| A supplier invoice the saved PayPal cannot cover | The shop's PayPal balance is short at settlement time | Vault capture | Settlement fails cleanly; both sides' agents agree an instalment plan inside both mandates | Instalments: yes |
| Capital in | The owner approves with a passkey; the owner's account pays the business | `POST /v1/payments/payouts` from the owner's app | Booked as owner's capital, not income | New |

## Hosting subscriptions

| Case | How it is triggered | Call | What should happen | Have? |
|---|---|---|---|---|
| Start, with a first real charge | A site subscribes; PayPal charges on activation | `POST /v1/billing/subscriptions`; events `BILLING.SUBSCRIPTION.ACTIVATED`, `PAYMENT.SALE.COMPLETED` | Plan active only after the signed event | Yes |
| Upgrade and downgrade | The site's Chief changes plan | `POST /v1/billing/subscriptions/{id}/revise` | PayPal may ask the payer to approve a price increase; if it does, that is a held decision for the owner | Yes. Whether approval is asked: not tried |
| Cancel | The Chief cancels when cash is short | `POST /v1/billing/subscriptions/{id}/cancel` | Agentrix's Revenue records the churn and stops service at period end | Yes |
| Suspend after a failed payment, then resume | `BILLING.SUBSCRIPTION.PAYMENT.FAILED`, or Agentrix suspends for non-payment | `.../suspend`, `.../activate` | Service paused, not deleted; resumes on payment | Yes |
| A renewal | The shortest billing cycle is a day, so a plan started tonight renews about 24 hours later | – | It will land after the video is cut. It can still be in the record before the deadline | – |

## Things a long run meets by itself

| Case | What should happen |
|---|---|
| The access token expires (sandbox tokens last hours) | Refreshed before expiry; no failed call |
| A rate limit (HTTP 429) | Backed off and retried. Do not provoke it on purpose |
| A sandbox outage | Calls fail, are recorded as failed, and are retried; nothing is marked paid without PayPal's word |

## Not verified here

- The exact error PayPal returns for a payout without funds, and whether it is at the call or on the batch.
- The request body of the resend call.
- Which error codes the simulation header accepts beyond the one in PayPal's example.
- Disputes: the sandbox needs a buyer to open one by hand, so it is out unless James does it.
