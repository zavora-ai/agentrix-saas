# Zavora ERA (hosted) for Agentrix SaaS

Both companies' books and CRM run in one Zavora ERA instance, as two tenants: **Agentrix Ltd** and **Kilele Bakery**. ERA is proprietary; its code stays in its own repo and is run here as a service, never copied.

## Local (slice A ports)

```bash
deploy/saas/era/era.sh up       # Postgres :15433 (compose project saas-era), Redis :16380, ERA API :18180
deploy/saas/era/era.sh seed     # both tenants, idempotent
deploy/saas/era/era.sh restart  # once, after the first seed (the support portal serves Agentrix)
deploy/saas/era/era.sh status
deploy/saas/era/era.sh reset --yes   # both tenants afresh, e.g. before recording (drops the books)
```

- **Secrets:** `ERA_DATABASE_PASSWORD`, `ERA_JWT_ACCESS_SECRET` and `ERA_JWT_REFRESH_SECRET` come from the repo's `.env` (read line by line, never printed).
- **Tenant logins:** the seed generates them and keeps them in `~/.config/agentrix-saas/era/tenants.json` (0600). `saas-books` reads that file (`SAAS_ERA_TENANTS_FILE`).
- **ERA's build:** `era.sh` builds `zavora-erp-api` from `ERA_SRC` (default `~/Developer/projects/zavora-era`) with `--locked` into `~/Developer/hackathons/.agentrix-saas-target/era` and runs the binary from there.
- **Redis:** comes from compose when the `redis:7-alpine` image is present, and from a local `redis-server` otherwise.
- **Postgres:** any 16+ image works (`ERA_PG_IMAGE`). ERA needs only `pgcrypto` and `uuid-ossp`.

## What the seed makes, in each tenant

| What | Detail |
|---|---|
| Base currency | USD |
| GL 1050 | PayPal Clearing - USD. The bank account "PayPal clearing" sits on it |
| GL 8075 | PayPal Fees |
| Price list (Agentrix) | The WebAgent plans, monthly and 12 months (Essential 29.00 / 278.40, Business 79.00 / 758.40, Commerce 149.00 / 1,430.40) |
| Expense items (both) | Out of scope for VAT: General services, General expense, Cloud hosting, AI model usage, Contract services, Referral commission |
| Customers | The other company. Credit limit: Agentrix gives Kilele 800.00; Kilele gives Agentrix 300.00 |
| Vendors | The other company, Vultr, Anthropic, Wanjiru Kamau (contractor), Savannah Growth Partners (referral). PayPal emails go in the vendors' bank details |
| CRM | On |
| Judges' login | `judge+agentrix@agentrix.website` / `judge+kilele@agentrix.website`, ERA's own **Viewer** role |

## The account map `saas-books` posts with

| Role | GL |
|---|---|
| PayPal clearing | 1050 |
| Receivables | 1200 Trade Debtors |
| Payables | 3010 Trade Creditors |
| Revenue | 5100 Service Revenue |
| Sales returns (refunds) | 5400 |
| PayPal fees | 8075 |

## Books from Rust

`saas-books::EraBooks` is the `saas_core::Books` contract over this instance, one login per company: `EraBooks::connect(EraConfig::from_env(CompanyId::agentrix())?)`. Ledger writes go through mcp-erp (built with `--features zavora` into `~/Developer/hackathons/.agentrix-saas-target/mcp-erp`; `MCP_ERP_BIN`, `MCP_ERP_DIR`), everything else over REST. `EVIDENCE.md` has one real PayPal sandbox capture booked in both companies' books.

## Judges: read-only logins on books.agentrix.website

ERA's seeded `Viewer` role is read-only. Checked against the local instance (Tue 10 Nov):

| A judge can read | A judge is refused (403) |
|---|---|
| invoices, bills, payments, journal entries, the chart, the PayPal clearing bank account, customers, vendors, recurring invoices, the dashboard, AR and AP ageing, CRM leads, opportunities and tickets | every write (customers, journals, payments, settings, users), the audit log, the user list |

- **Password:** `SAAS_ERA_DEMO_PASSWORD` from the env file the seed is given, when set. Otherwise the seed generates one, keeps it in `<state>/tenants.json` under `judges`, and never prints it. Put it in the Devpost testing notes.
- **The public proxy must also close ERA's self-service tenant routes.** ERA lets *any* logged-in user, a judge included, call them: `POST /api/v1/auth/tenants` (a new tenant the caller owns), `/api/v1/auth/switch-tenant`, `/api/v1/auth/tenants/*/{archive,unarchive,leave}` (`leave` would drop the judge login and lock out the next judge), plus `/api/v1/auth/signup*`, `/api/v1/auth/register` and `/api/v1/platform/*`. Keep `GET /api/v1/auth/tenants`, which the UI reads. The node reaches ERA on loopback and needs none of them.
