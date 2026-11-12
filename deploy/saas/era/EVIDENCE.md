# One PayPal sandbox capture, booked in both companies' books

A real call to PayPal's **sandbox**, posted to both tenants of the local Zavora ERA (hosted) through `saas-books::EraBooks`. Ledger writes went through the open-source mcp-erp server, reached with adk-tool's MCP client.

**Run:** `scripts/wcargo run -p saas-books --example book_paypal_capture`, Tue 10 Nov 2026, 18:59 EAT (PayPal's `create_time` 15:59:20 UTC).

## PayPal (sandbox, Agentrix SaaS's app, through agentrix-effects' client)

| What | Id | Status | Amounts |
|---|---|---|---|
| Orders v2, merchant-initiated, on Kilele's saved wallet (Vault `5an08729le1870910`, saved by slice C) | order `64E97721RB574792F` | COMPLETED in one call, no buyer step | |
| Capture | `2H853595A8077800U` | COMPLETED | gross USD 29.00, PayPal fee USD 1.43, net USD 27.57 |

PayPal `invoice_id`: `agentrix-saas/era/INV-2026-0006/7818da62`.

## Zavora ERA, posted from `paypal:capture:2H853595A8077800U`

| Books | Document | ERA id | Result |
|---|---|---|---|
| Agentrix Ltd | invoice `INV-2026-0006` to Kilele Bakery, WebAgent Essential plan, monthly | `7818da62-ea71-4aad-b54b-5e0450473ac2` | paid, USD 0.00 owed |
| Agentrix Ltd | receipt `PAY-2026-0007` into PayPal clearing (GL 1050), applied to the invoice | `f570195e-8e73-4244-b7ee-b945c4159213` | USD 29.00 |
| Agentrix Ltd | fee journal `JE-2026-0024`, Dr 8075 PayPal fees, Cr 1050, reference `paypal:capture:2H853595A8077800U:fee` | `bc52cd0f-95d4-496e-90d8-f2398eff9502` | USD 1.43 |
| Kilele Bakery | bill `BILL-2026-0004` from Agentrix Ltd | `1a0a44c6-55bf-496c-9f30-baecf689b906` | paid, USD 0.00 owed |
| Kilele Bakery | vendor payment `PAY-2026-0003` out of PayPal clearing, applied to the bill | `e7da32f8-6ea1-4f00-84f0-7a911fe37003` | USD 29.00 |

Posting the same capture again posted nothing ("already posted = true"): the receipt is found by its PayPal reference, and the fee journal by ERA's unique journal reference.

## Notes

- **The first attempt** at 18:58:58 failed in the TLS handshake to PayPal (`got ApplicationData when expecting Handshake`), before any order existed. Its invoice `INV-2026-0005` and bill `BILL-2026-0003` (USD 29.00 each) stay open in the books. The retry seconds later made the capture above.
- **Test residue:** the books also hold `tests/era_live.rs` runs (ids starting `LIVE…`). `era.sh reset --yes` starts both tenants afresh before recording; that also drops the rows above.

# A recurring invoice, paid through PayPal and booked against it

**Run:** `scripts/wcargo run -p saas-books --example book_recurring_payment -- vault`, Tue 10 Nov 2026, about 19:36 EAT (16:35 UTC).

PayPal Subscriptions takes no saved wallet: only a buyer's approval, or a card, and this sandbox app has no card processing. So the closest event with no buyer step is the one this entry uses between companies: the monthly plan as an ERA recurring invoice, collected by a merchant-initiated capture on Kilele's saved wallet.

| Step | Where | Id | Result |
|---|---|---|---|
| Recurring invoice, Kilele Bakery, WebAgent Essential plan, monthly, USD 29.00, reference `agentrix-saas/recurring/kilele-bakery/webagent-essential-monthly` | Agentrix's ERA | `20b17fb6-1036-4330-8418-68f6df744224` | run 1 of a monthly schedule; next 2026-12-10 |
| This cycle's invoice, raised by **ERA's own scheduler** (its boot run), carrying the schedule's reference | Agentrix's ERA | `INV-2026-0007`, `cc5c9d4c-b5b0-4cd6-b1e4-5453aaa20df1` | posted by `raise_due_recurring`, then paid |
| Orders v2 on Kilele's saved wallet | PayPal sandbox | order `9GJ259545L100982B`, capture `4DU31502R45847116` | COMPLETED; gross USD 29.00, PayPal fee USD 1.43 |
| Receipt into PayPal clearing, applied to `INV-2026-0007` (found by `saas_core::recurring_invoice_for`, not oldest-first) | Agentrix's ERA | `PAY-2026-0008`, `5983fe6f-f38a-4c3a-94e5-e47c2db44e24` | USD 29.00 |
| Fee journal, Dr 8075, Cr 1050 | Agentrix's ERA | `JE-2026-0027`, `6cd7ac80-647c-4c0c-a9f0-bd053a343a9f` | USD 1.43 |

## The PayPal Subscriptions path, waiting on one buyer approval

`… -- subscribe` made a real sandbox subscription and its ERA schedule:

| What | Id |
|---|---|
| PayPal product "Agentrix SaaS WebAgent Essential" | `PROD-0KB88595X87602035` |
| PayPal plan "Agentrix SaaS WebAgent Essential plan, monthly", USD 29.00 a month | `P-4EH32417F7917980RNLZUP7Q` |
| PayPal subscription | `I-MLUSV3RB9R1B`, APPROVAL_PENDING |
| ERA customer Wanjiru Designs (the first sandbox buyer's email) | `dbe63498-8d89-42c2-9be8-e89714db34b4` |
| ERA recurring invoice, reference `paypal:subscription:I-MLUSV3RB9R1B` | `923c94b8-fc6d-4dba-a609-26b9eea9ba2e`, first run 2026-11-10 |

Once a sandbox buyer approves it (`https://www.sandbox.paypal.com/webapps/billing/subscriptions?ba_token=BA-0BE554087U498534V`), PayPal takes the first month. Then `… -- subscription I-MLUSV3RB9R1B` books each completed sale (`paypal:sale:<id>`) against the invoice this schedule raised.
