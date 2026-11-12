# saas-node: hosting Agentrix on one Vultr VM (spec v3)

One VM, `saas-node` (64.177.42.252, vc2-4c-8gb in atl), serves the whole `agentrix.website` zone:
- the platform (agentrix-node) and the money layer (agentrix-saas), as one binary or as two
  processes split by host in Caddy (the join decides; `SAAS_SHAPE` in `agentrix-saas.env`)
- Zavora ERA (hosted) under Docker
- sandbox containers
- Caddy

The judged build lives under one platform host, `SAAS_PLATFORM_HOST=paypal.agentrix.website` (`agentrix-saas.env`):

| Host | Served by |
|---|---|
| `paypal.agentrix.website` | The platform (agentrix-node, 127.0.0.1:8080): home, sign-in, Agentrix OS, Agentrix's own door |
| `ops.paypal.agentrix.website` | The platform: the operator console |
| `<site>-sample.paypal.agentrix.website` | The platform: each sample site's pages, checkout, door, `did.json`, passports |
| `pay.paypal.agentrix.website` | The money layer (agentrix-saas, 127.0.0.1:18190 in shape two): PayPal webhooks, Agentrix's agent door |
| `books.paypal.agentrix.website` | ERA's UI container on `127.0.0.1:18181`, which forwards `/api` to ERA's API on `127.0.0.1:18180` |
| `money.`, `kilele.money.`, `pay.`, `books.agentrix.website` | Spec v2's names, still routed until they're retired (the coordinator's call) |

**DNS.** The apex, the wildcard, `paypal` and `*.paypal` point at saas-node (on James's go-ahead, 10 Nov). `apply` added explicit A records for `money`, `kilele.money`, `pay` and `books`. `provision.sh zone` is the only command that changes records it didn't create, and only those still on an address no VM holds. MX, TXT, CNAME and NS records are never touched.

**Certificates.** Every host above is named in the Caddyfile and gets its certificate at start. Any other name under the wildcard gets one on demand, and only when agentrix-node's `/internal/tls-ask` (loopback) says it serves that host. On-demand TLS is never open.

`deploy/provision.sh` and `deploy/deploy.sh` are the ended Vultr entry's three-VM scripts and stay disabled here.

## Steps

```bash
deploy/saas/provision.sh plan             # read only: the account, the zone, the price, what apply adds
deploy/saas/provision.sh apply --yes      # PAID, after James approves: key, firewall, VM, 4 A records
deploy/saas/deploy.sh                     # ship HEAD, build on the VM, install, smoke test
deploy/saas/deploy.sh --owner-link        # the owner-console link, into a local 0600 file
deploy/saas/deploy.sh --snapshot --yes --wait   # after submission
```

- **More commands:**
  - `deploy.sh --smoke`, `--status`, `--rollback`, `--break-lock`
  - `deploy.sh --era` to rebuild ERA, `--seed` to seed again, `--books era|fake`, `--models live|record|replay`
  - `provision.sh status`
  - `provision.sh ssh-allow --yes`, when this Mac's address changes
  - `provision.sh destroy --yes`, after judging
- **The ERA seed:** the first deploy seeds both ERA tenants. The tenants' logins stay on the VM, in `/var/lib/agentrix-saas/era/tenants.json`.
- **Caddy waits for DNS.** `deploy.sh` installs the Caddyfile only once every named host answers with the VM's address at `ns1.vultr.com`, and with no AAAA, so Let's Encrypt never sees another address.

## Files

| File | What it is |
|---|---|
| `provision.sh` | Vultr API v2: `plan` (default, read only), `apply`, `status`, `ssh-allow`, `snapshot`, `destroy`, `zone` (spec v3: apex and wildcard to saas-node, stale records out). Refuses when a `saas-*` object or one of the four names exists with other values |
| `cloud-init.yaml` | First boot: ufw (22, 80, 443), swap, Docker, gVisor `runsc` (extra runtime), Caddy, Node.js 22, Rust 1.95.0, users `deploy` and `agentrix-saas` |
| `deploy.sh` | Ships a commit with rsync. Builds `agentrix-saas` (with the model gateway inside), mcp-erp and the board on the VM; ERA under compose `saas-era`. Installs, then runs the smoke test; `--smoke-model` makes one model call through the node |
| `agentrix-saas.service` | The money layer as its own process (shapes v2 and two). User `agentrix-saas`, data in `/var/lib/agentrix-saas` |
| `agentrix-node.service` | Shape one: the platform with the money layer mounted (`SAAS_MODULE=1`), 8080 and 8079, data in `/var/lib/agentrix-node` and `/var/lib/agentrix-saas`. Conflicts with `agentrix-saas.service` |
| `agentrix-models.service` | Shape one: the Model Gateway on 8090, the only holder of model keys, metering into agentrix-node's `control.db` |
| `paypal-webhook.sh` | Points our sandbox app's webhook (`PAYPAL_WEBHOOK_ID`) at `https://pay.<platform host>/paypal/webhook`; read only without `--yes` |
| `agentrix-saas.env` | Non-secret settings, appended after the `.env` keys in `/etc/agentrix-saas/agentrix-saas.env` (0600) |
| `era.compose.yml` | ERA's API and UI, built from ERA's own Dockerfiles, added to `era/docker-compose.yml` (the books slice's file) |
| `Caddyfile` | Spec v3's zone, a template for `SAAS_SHAPE` one or two: named hosts, on-demand TLS only for hosts agentrix-node's `tls-ask` approves, ERA's tenant routes closed |
| `Caddyfile.v2` | Spec v2's four hosts, installed while `SAAS_SHAPE=v2` |
| `COST.md` | The price, the model caps, judge mode, no polling, snapshots |
| `tests/provision-checks.sh` | Offline checks of the refusal rules (fixtures, no API calls) |

## Secrets

**Where they come from:** the repo's `.env`, read line by line, never sourced or printed.

**Where they go:**
- `VULTR_API_KEY` stays on the Mac.
- The node gets the `SAAS_`, `AGENTRIX_`, `PAYPAL_`, `KILELE_`, `ANTHROPIC_`, `VULTR_INFERENCE_` and `AG_*` keys. The model keys are there because the Model Gateway runs inside the node.
- ERA gets its three `ERA_*` secrets in its own root-only file.
- The VM makes two values once, on its first deploy:
  - the owner-console token
  - ERA's notification key

**ERA stays private:** its committed sources are rsynced to `/opt/zavora-era` on the VM only, never into this repo.
