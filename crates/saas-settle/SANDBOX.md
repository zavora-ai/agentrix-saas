# Settlement on PayPal's sandbox: evidence

These are real calls to PayPal's **sandbox** (`api-m.sandbox.paypal.com`). Agentrix SaaS is the payee and Kilele Bakery is the payer. Times are EAT, Tue 10 Nov 2026, read from the machine's clock.

**Where these come from**

- **The example.** `scripts/wcargo run -p saas-settle --example sandbox_settle -- <command>`. The commands are `run vault`, `run payout`, `instalments`, `vault-start`, `vault-token`, `vault-finish` and `status`.
  - Both agent doors run on 127.0.0.1:18182, as `agentrix.localhost` and `kilele.localhost`.
  - Each door fetches the other's key directory over HTTP, and every A2A request is signed with Web Bot Auth.
- **Keys.** Agentrix's app uses `PAYPAL_CLIENT_*` and Kilele's app uses `KILELE_PAYPAL_CLIENT_*`, read from `.env` and never printed. The example refuses any environment but the sandbox.
- **State.** Kept in `~/.config/agentrix-saas/settle/` (mode 0700). Each line of `evidence.jsonl` is one row below.
- **Mandates.** No phone is at hand here, so each owner's mandate is approved with the owner key file (`local-owner-key`), which this example accepts by name.
  - On the node the owner approves with a passkey (`mandate::seal_owner_approved` through agentrix-host `Approvals`), and only `passkey` is accepted.
  - The tests run the passkey path.
- **Books.** Both companies use saas-core `FakeBooks` here. The desks post through `era::EraPayee` and `era::EraPayer`, which take any `Books`.

## Route check (first hour)

| What | PayPal id | Status |
|---|---|---|
| Agentrix's app token scopes | app `APP-9M690043TT1126458` | includes `vault/payment-tokens/readwrite` |
| Setup token for Kilele's PayPal wallet (Agentrix's app) | `37R143433F051950D`, customer `GHeVXOJkXI` | PAYER_ACTION_REQUIRED, then VAULTED after James approved as Kilele's sandbox business account |
| Payment token from that setup token (`vault-finish`) | `vault_id` `5an08729le1870910` | saved |
| Payout fallback probe: USD 1.00 from Kilele's app to Agentrix's merchant email | batch `7QY6XKX4LFYR6`, item `BKPXCT3MBE24W`, transaction `11C23624Y5977831G` | SUCCESS |

**Both routes work.**
- **Vault (primary):** merchant-initiated Orders v2 on the `vault_id` comes back COMPLETED in one call, with no buyer step.
- **Payout (fallback):** works too.

## Phase 3: refusal receipts and attacks (library `4b69913`)

`scripts/wcargo run -p saas-settle --example node_settle -- 3000` runs through the same routers and kernel calls, then sends the eval suite's four attacks against the live doors. Tue 10 Nov, EAT:

| When | What | PayPal ids, or the refusal receipt | Result |
|---|---|---|---|
| 20:36:16 | `settle_invoice`, INV-0001, USD 30.00 | order `1YG74815NW306683S`, capture `95K75878RE452384D`, fee USD 1.47 | COMPLETED. Both books posted from `paypal:capture:95K75878RE452384D` |
| 20:36:19 | `agree_plan`, then `collect_instalment(0)`: INV-0002, instalment 1 of 2, USD 30.00 | order `5DE26817E2175234E`, capture `9BM123824M013210E` | COMPLETED. Both books posted, instalment 2 is on the wake list |
| 20:36:19 | attack: unsigned request | `rr_01a1c6883d9b74af83ded4cbf5f92400`, sealed by Agentrix | refused, `signature_required`, nothing moved |
| 20:36:20 | attack: wrong-key agent | `rr_01a1c6883e0174648af7e5f34e698344`, sealed by Agentrix | refused, `signature_invalid`, nothing moved |
| 20:36:20 | attack: replayed mandate (INV-0001's cart mandate) | `rr_01a1c6883f437217b743a70776faf071`, sealed by Agentrix | refused, `mandate_used`, nothing moved |
| 20:36:20 | attack: instalment terms outside Kilele's mandate (6 instalments over 150 days) | `rr_01a1c688400f7129b11892bd2ab9a895`, sealed by Kilele | refused, `plan_refused`, nothing moved |

Each refusal receipt's seal checked against the refusing company's published key directory. "Nothing moved" was read from the settlement store, not from the conversation.

**The capture webhook.** The PAYMENT.CAPTURE.COMPLETED path is covered by the tests: a capture whose answer was lost is completed by the webhook, through both books, exactly once. A real delivery to `https://pay.agentrix.website/paypal/webhook` (webhook `2V1660749U035510R`) waits on the deployed node.

## Through the node's routers (library `714f61c`, example `ee96091`)

`scripts/wcargo run -p saas-settle --example node_settle -- 4800` composes `SettleHost::pay_router` and `SettleHost::kilele_router` by `Host` on one listener, as the SaaS node does. The kernel's calls drive every step. Tue 10 Nov, EAT:

| When | Kernel call | What | PayPal ids | Both books posted from |
|---|---|---|---|---|
| 19:00:26 | `offer_invoice` | INV-0001, USD 48.00, offered over A2A. Kilele's door answered `will_settle`, and its desk emitted `bill_offered` | none (no money) | none |
| 19:00:27 | `settle_invoice` (key `node-run-20261110T160025`) | Kilele's Payables settled INV-0001 by Vault capture | order `5PT82111NG640900N`, capture `8B0024691L091615T`, PayPal fee USD 2.17 | `paypal:capture:8B0024691L091615T` |
| 19:00:32 | `settle_invoice` with the same key | the same settlement returned | `8B0024691L091615T` again, no second capture | none (no second posting) |
| 19:00:32 | `agree_plan` | INV-0002, USD 96.00, on Agentrix's terms: 2 × USD 48.00, due 10 Nov and 10 Dec. Kilele's Payables accepted inside its owner's mandate | none (no money) | none |
| 19:00:33 | `collect_instalment` (position 0) | instalment 1 collected on its due date | order `6TS16387BE772200U`, capture `6Y60670932197892C` | `paypal:capture:6Y60670932197892C` |

After the run, Agentrix's books show INV-0001 open USD 0.00 and INV-0002 open USD 48.00. The second instalment stays on the wake list for 10 Dec.

## Settlements on commit `9a52170`

| When | Route | What | PayPal ids | Status | Both books posted from |
|---|---|---|---|---|---|
| 00:36:11 | Vault | invoice INV-0001, USD 36.00 | order `9U848634HA4506231`, capture `6NT30195937284033`, PayPal fee USD 1.70 | COMPLETED | `paypal:capture:6NT30195937284033` |
| 00:36:18 | Payout | invoice INV-0001, USD 36.00 | batch `U7SJU6KHZ4HZN`, item `VRZ59J5EZB2E6`, transaction `39X79412XK774462Y` | SUCCESS | `paypal:payout:VRZ59J5EZB2E6` |
| 00:36:51 | Vault, instalment 1 of 3 | invoice INV-0001, USD 60.00 in 3 × USD 20.00 | order `0KF53260AD3904907`, capture `7RX7461283952571R`, fee USD 1.08 | COMPLETED | `paypal:capture:7RX7461283952571R` |

Each run goes the same way:
1. Agentrix's Receivables agent offers the invoice to Kilele's door, signed.
2. Kilele records the bill.
3. Kilele's Payables agent signs a cart mandate inside its intent mandate and asks Agentrix's door to settle.
4. Agentrix checks both mandates and moves the money, by capturing or by checking Kilele's payout.
5. Agentrix seals the receipt, and both books clear: the invoice and the bill each show USD 0.00 open.

**Asking again.** Asking to settle the same invoice again is refused before anything moves: "Invoice INV-0001 is already settled (USD 36.00 paid). Nothing was paid."

**The instalment run:**
1. Kilele, with USD 20.00 on hand, proposed 4 instalments over 80 days.
2. Agentrix countered, because its owner allows at most 3 within 60 days with the first at least 25%. The counter was 3 × USD 20.00, due 9 Nov, 9 Dec and 8 Jan (UTC dates).
3. Kilele accepted with 3 pre-signed cart mandates.
4. Agentrix collected instalment 1 on its due date and sent the receipt to Kilele's door, and Kilele's books posted it.

## Earlier runs (same flows, before the last fixes)

| When | Route | PayPal ids | Status |
|---|---|---|---|
| 00:30:54 | Payout, USD 24.00 | batch `P4FQHS74UAK9C`, item `553R7A4YQWRE4`, transaction `5CG9480516730803N` | SUCCESS |
| 00:32:09 | Vault, USD 24.00 | order `0S254744VC661984C`, capture `85C113877E350781J`, fee USD 1.24 | COMPLETED |
| 00:33:46 | Vault, instalment 1 of 3, USD 20.00 | order `6RF56180F3206343J`, capture `48E32517N1574904Y`, fee USD 1.08 | COMPLETED |

**A refusal that changed the code.** At 00:32:52 an instalment acceptance was refused by the deterministic check.
- **What happened:** the intent mandate expired before instalment 3 fell due, so Kilele's cart mandates couldn't cover it.
- **The fix:** plans are now checked against both mandates' expiry dates, and the negotiator's bounds are cut so a counter falls due before either mandate expires.

## What the sandbox taught us

- **Setup token status.** PayPal reports an approved PayPal-wallet setup token as `VAULTED`, not `APPROVED`. `/v3/vault/payment-tokens` then turns it into the `vault_id`.
- **Vault capture is one call.** An Orders v2 order on a `vault_id` with intent CAPTURE returns COMPLETED, with the capture and PayPal's fee in `seller_receivable_breakdown`.
- **Who pays the fee.** On the Vault route the payee pays PayPal's fee, and it posts as Agentrix's fee. On the Payout route the payer pays the payout fee, and the payee receives the gross amount.
- **PayPal `invoice_id` must be unique per merchant account.** Settlement uses `agentrix-saas/<invoice>[/<i>-of-<n>]/<cart mandate>`, so re-seeded demo books never collide with an earlier run.
