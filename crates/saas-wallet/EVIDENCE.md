# A real PayPal sandbox capture in a wallet record and in Zavora ERA

**Run:** `scripts/wcargo run -p saas-wallet --example wallet_capture -- vault 19.00`, Tue 10 Nov 2026, 20:51:51 EAT, against the local Zavora ERA (hosted) instance. The wallet is SQLite in `~/.config/agentrix-saas/wallet` (0700); the signing key is 0600.

## PayPal sandbox (Agentrix SaaS's app)

| What | Id | Status |
|---|---|---|
| Orders v2, merchant-initiated, on Kilele's saved wallet (Vault `5an08729le1870910`) | order `58F11926XN127970R` | COMPLETED |
| Capture | `3KT78856M2504654G` | COMPLETED: gross USD 19.00, PayPal fee USD 1.04 |

## Wallet record `agentrix/cashier` (signed log `wallet:agentrix/cashier`)

| Seq | Kind | Amount | PayPal | Source |
|---|---|---|---|---|
| 0 | capture | USD 19.00 | `paypal:capture:3KT78856M2504654G` | read-back `GET /v2/payments/captures/3KT78856M2504654G` |
| 1 | fee | USD -1.04 | the same capture | the same read-back |

- **Acting agent:** `agentrix/receivables`.
- **Authority:** an operator-run approval (the example says so); on the node it is the mandate or the approval the governed path holds.
- **Checkpoint:** "Matches PayPal as of 20:52 EAT"; root `6cae2885635c873aa710881eee5b99faf2e041a9c5b97f6bc09ae0c846110128` over 4 records; the signed log verifies (4 records, 4 checkpoints). Verifier key `wallet.agentrix.agentrix.website+533e8456+ASvERhtUu/jc8trldPHTskwcl37nTPifzjsau0fwVnS+`.
- **Recorded again** (`-- record 3KT78856M2504654G`): "already recorded: true"; nothing new in the wallet or in ERA.

## Zavora ERA (Agentrix Ltd), posted by the wallet through saas-core's rules

| Document | ERA | Lines |
|---|---|---|
| Invoice for the order (a shop sale gets its own), reference `paypal:order:58F11926XN127970R` | `INV-2026-0009`, `95d825f6-4171-485e-91ec-0eae73cca5f2`, paid | USD 19.00 to Kilele Bakery |
| Receipt into PayPal clearing, applied to it | `PAY-2026-0009`, `b0154c7e-d656-4589-be69-6e136a6d420d` | USD 19.00 |
| The receipt's journal | `JE-2026-0029`, `9defb496-a64a-459b-b2b9-ef6ebaca0874` | Dr 1050 PayPal clearing 19.00, Cr 1200 Trade Debtors 19.00 |
| Fee journal, reference `paypal:capture:3KT78856M2504654G:fee` | `JE-2026-0030`, `811bd9b2-143e-490d-9381-5588e175ea22` | Dr 8075 PayPal fees 1.04, Cr 1050 1.04 |

## Three-way match

| Seq | Wallet | PayPal | ERA journal | State |
|---|---|---|---|---|
| 0 | capture USD 19.00 | `3KT78856M2504654G` COMPLETED | `JE-2026-0029` | matched |
| 1 | fee USD -1.04 | `3KT78856M2504654G` COMPLETED | `JE-2026-0030` | matched |
