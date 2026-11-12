# Agentrix SaaS: status

**Written Tue 10 Nov 2026, about 19:45 EAT.**
- **Submission:** Thu 12 Nov, 18:00 EAT. The deadline is 23:00 EAT (12:00 PST).
- **Features cut:** Wed 11 Nov, 12:00 EAT.
- **Freeze:** Wed 11 Nov, 18:00 EAT.

## Where `main` is

`main` carries every slice and the deploy scripts. The check below was run on `6d94a26`; the deploy merge after it adds only shell and config files.
- `scripts/wcargo clippy --locked -D warnings` is clean.
- `scripts/wcargo test --locked` passes **226 tests**, with 3 ignored (live-ERA integration).
- The tests cover saas-core, saas-books, saas-company, saas-settle, saas-web, saas-node, agentrix-effects, agentrix-models and agentrix-decisions.
- Run on Tue 10 Nov at 19:24 EAT.
- `cargo fmt --check` flags one file outside this work: `crates/agentrix-node/src/launch/tests.rs`, inherited from the platform.

| Slice | Branch | What it delivers |
|---|---|---|
| Rails | `paypal-rails` | Payouts, subscription suspend and activate, wiremock tests, a sandbox probe (`EVIDENCE.md`) |
| A. Books | `books` | `saas-core` contracts; `EraBooks` over Zavora ERA through `mcp-erp` and ERA REST; a local ERA for both tenants (`deploy/saas/era/`) |
| B. Company | `company` | Kernel per company, graph flows, agents on the governed path, guardrails, memory, identity, token exchange, model mix. `saas-node` (binary `agentrix-saas`) serves everything on one port |
| C. Settle | `settle` | AP2-style mandates, signed agent doors (Web Bot Auth), payee and payer desks, Vault and Payout rails, posting to both books |
| D. Face | `face` | One server for the four hosts; feed from the books; PayPal subscribe pages; owner passkeys; the AG Studio board with an adk-rust harness |
| Deploy | `deploy` | `deploy/saas/`: single-VM provision (plan and apply), deploy, Caddy, systemd, the ERA compose file, `COST.md` |

## Sandbox evidence

Every id below was read back from PayPal or ERA.

| What | Id |
|---|---|
| Vault (Kilele's PayPal saved with Agentrix) | setup token `37R143433F051950D` (VAULTED) → `vault_id 5an08729le1870910` |
| Agent-to-agent settlement through Vault | order `9U848634HA4506231`, capture `6NT30195937284033` (USD 36.00, COMPLETED). Coordinator read it back from PayPal. |
| Instalment capture through Vault | `7RX7461283952571R` |
| Payout fallback (Kilele → Agentrix) | batch `7QY6XKX4LFYR6`, item `BKPXCT3MBE24W`, transaction `11C23624Y5977831G` (SUCCESS) |
| A capture booked in both ERA books | `2H853595A8077800U` |
| Our webhook | `2V1660749U035510R` → `https://pay.agentrix.website/paypal/webhook` |

More are in `EVIDENCE.md` and `crates/saas-settle/SANDBOX.md`.

## Infrastructure

- **Server:** James approved it on Tue 10 Nov, about 19:30 EAT: `vc2-4c-8gb` in `atl`, $40.00 a month, about $55 through 22 Dec.
- **Deploy:** the deploy session is creating `saas-node` and four A records (`money`, `kilele.money`, `pay` and `books` under agentrix.website), then deploying `main`.
- **Untouched:** the Vultr entry's `agx-*` VMs and its existing records.

## Left to do

1. **Phase 3, in progress:**
   - the `agentrix-saas story` run of the five scenes
   - about ten attacks, committed by hash first and scored from ERA, PayPal and the audit log, including `adk-rust audit verify`
   - instalments on kernel due dates
   - the `/proofs` page and the judges' path
   - a read-only view of ERA at `books.`
2. **From James:**
   - the AG Studio trial key (`AG_STUDIO_LICENSE_KEY`)
   - the HeyGen Creator plan and key
   - the 15-second cameo
   - disk: builds stop under 15 GB free
3. **Wednesday evening:** the Chief's presenter video, as in the spec: mascot on HeyGen Avatar IV, voice from Vultr text-to-speech, real product footage.
4. **Thursday:**
   - export the public repo `zavora-ai/agentrix-saas` (MIT, licence shown in About)
   - the Devpost write-up, submitted by 18:00 EAT
   - snapshot the server
   - freeze until about 21 Dec
