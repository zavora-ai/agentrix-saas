# Deploying the first release on Vultr

This folder creates the Vultr substrate for the Agentrix WebAgent Platform (first-release design §2, R14):
- three VMs in one region, on one VPC: `control`, `sandbox` (VX1) and `fixture`;
- the firewall groups and each host's nftables;
- Caddy, and Docker with gVisor;
- DNS records.

`deploy.sh` (task S1-12) then installs the platform on it and takes any commit to a running
stack, with a smoke test.

| File | What it is |
|---|---|
| `provision.sh` | Plan, apply, status, ssh-close and destroy, against the Vultr API |
| `deploy.sh` | Deploys a commit: build on control, install, restart, smoke test; rollback; one deploy at a time |
| `systemd/agentrix-{node,models,runner}.service` | The services (node and models on control, the runner on the sandbox) |
| `env/{node,models}.env` | Their non-secret settings, installed as `/etc/agentrix/*.env` on control |
| `cloud-init/{control,sandbox,fixture}.yaml` | First-boot configuration of each VM |
| `nftables/{control,sandbox,fixture}.nft` | Host firewalls. These, not the Vultr groups, limit VPC traffic |
| `caddy/{control,fixture}.Caddyfile` | TLS and reverse proxy |
| `docker/sandbox-daemon.json` | Docker on the sandbox: gVisor (`runsc`) as the default runtime |
| `smoke/stack.sh` | Stack smoke test, run by `deploy.sh` after every deploy |
| `smoke/sandbox-rooms.sh` | Containment smoke test, run on the sandbox: what a room and the proxies can and cannot reach, and a proxy's privileges |
| `.env.example` | The settings `provision.sh` reads from `deploy/.env` |

## Use

```sh
cp deploy/.env.example deploy/.env      # James fills VULTR_API_KEY (S0-02) and the domains (S0-03)
deploy/provision.sh plan                # dry run: prices, availability, rendered cloud-init
deploy/provision.sh apply --yes         # PAID: only after James says yes
deploy/provision.sh status              # instances, groups, VPC, and each host's cloud-init and facts
ssh -F deploy/.state/ssh_config agx-control   # or agx-sandbox, agx-fixture (through control)
deploy/provision.sh ssh-close --yes     # remove the admin SSH rule when deploys are done
deploy/provision.sh destroy --yes       # deletes the VMs, groups and VPC (James's yes again)
```

### Costs, snapshots, sleep and wake

The rules and the numbers are in [specs/platform/cost.md](../specs/platform/cost.md). Without
`--yes`, each command is a dry run that prints what it would do and what it costs or saves.

```sh
deploy/provision.sh costs                          # read only: this month's bill, run rate, snapshots
deploy/provision.sh snapshot [ROLE…] [--yes]       # changes nothing; outside the first release's windows A-D
deploy/provision.sh sleep sandbox fixture [--yes]  # fresh snapshot, then delete (James's yes)
deploy/provision.sh wake [control] sandbox fixture [--yes]   # recreate from the latest snapshots, control first (James's yes)
```

- **Only the sandbox and the fixture sleep.** Control serves agentrix.website, holds
  `control.db` and the secrets, and the other hosts' firewalls name its VPC address.
- **Vultr bills a stopped VM in full**, so sleeping means a snapshot and no instance.
- **`sleep --yes` refuses** before `SLEEP_NOT_BEFORE` in `deploy/.env` (set it to when judging
  ends) and while a deploy holds the lock. Every sleep takes a fresh snapshot and waits for it
  before deleting anything.
- **`wake` brings back** the same label, plan, firewall group and VPC, control first when it
  is asleep too (a full restore: specs/platform/cost.md §2.3). It keeps the snapshot's
  SSH host keys (cloud-init would delete them), rewrites the fixture's DNS records, updates
  `.state/inventory.env` and `.state/ssh_config`, and logs the restore time in
  `.state/snapshots.log`.
- **If the sandbox comes back at a new VPC address**, the next `deploy.sh` installs it. Node's
  `AGENTRIX_RUNNER` is taken from the inventory's `RUNNER_ADDR`, not from `env/node.env`.
- **A sleeping sandbox or fixture stops deploys:** the smoke test checks the runner, and the
  gauntlet reads the fixture. Wake both first.

`apply` also creates the DNS zone if Vultr does not have it, the platform and fixture records,
and a wildcard `*.<platform>` record for live sites' platform addresses. The registrar must
delegate the zone to `ns1.vultr.com` and `ns2.vultr.com`.

`plan` needs no key and creates nothing. `apply` matches everything by label (`agx-b1-*`), so
re-running it reuses what exists. The key is read from `deploy/.env` or the environment and
passed to curl on stdin, never on a command line, and it is never printed.

## Deploy

```sh
deploy/deploy.sh                 # HEAD; or deploy/deploy.sh <commit>
deploy/deploy.sh --secrets       # also (re)write control's provider keys from ./.env (never printed)
deploy/deploy.sh --quick         # skip the slower containment test in the smoke run
deploy/deploy.sh --no-gauntlet   # skip the gauntlet re-run (it runs by default)
deploy/deploy.sh --status        # the deployed commit, and who holds the deploy lock
deploy/deploy.sh --rollback      # the binaries from before the last deploy (units and settings stay)
deploy/deploy.sh --break-lock    # only if a deploy died holding the lock
```

What a deploy does:
1. Takes the lock on control (`/run/lock/agentrix-deploy`, naming who holds it), so two
   sessions can't deploy at once.
2. Ships the commit with `git archive`, never the working tree, to `/opt/agentrix/src` on control.
3. Sets up control idempotently: build tools, the `agentrix-models` user, the secrets folder, ufw.
4. Builds there with `cargo build --release --locked`. The target folder is
   `/opt/agentrix/target`, outside the source, so builds are incremental.
5. Installs `env/*.env`, the units, and the Caddyfile. The Caddyfile is rendered from
   `caddy/control.Caddyfile`, validated, and reloaded only if it changed.
6. Keeps the current binaries in `/opt/agentrix/bin.prev` on both hosts, installs the new ones,
   and records the commit in `/opt/agentrix/bin/DEPLOYED`.
7. Restarts dependencies first: the runner, then models (waiting until it answers), then node
   (waiting for `/healthz`). A Move that node resumes at start therefore finds them up.
   - It gives group `agentrix` write on `control.db` and its `-wal`/`-shm`. The gateway writes
     usage rows there, and SQLite plus the units' umask would leave them 0640.
   - The sandbox gets node's published job key, so the runner trusts only control.
8. Runs `smoke/stack.sh`:
   - the services and the deployed commit;
   - the runner from control;
   - on-demand TLS asking node;
   - the public pages over HTTPS, the redirect, HSTS and the certificate;
   - only 80 and 443 answering on control's public address;
   - the runner's capabilities;
   - `smoke/sandbox-rooms.sh`, unless you pass `--quick`.

   It exits non-zero on any failure, and `--rollback` is the way back.
9. **Re-runs the gauntlet on every deploy (R13.2)** for each live site (one with a
   `site_pointer`):
   - It runs `agentrix-gauntlet --record` on control.
   - It re-verifies the receipts from the assessment before the site's latest Move onwards
     (attacks 1-9).
   - It fires attack 10 at the live site's platform address, `{domain-with-dashes}.<platform>`.
   - It prints `n/10` and the `gauntlet.run` receipt digest.
   - Below 10/10, it says so loudly and exits 3; nothing is rolled back, because the owner decides.
   - With no live site, it prints one line and skips.

## What it creates

The default plans in Atlanta, at Vultr's list prices on 7 Nov 2026 (`plan` prints live prices):

| VM | Plan | Size | Monthly cap | Hourly |
|---|---|---|---|---|
| `control` | `vhp-4c-8gb-intel` | 4 vCPU, 8 GB, 180 GB NVMe; builds run here (D8), with 8 GB swap | $48.00 | $0.066 |
| `sandbox` | `vx1-g-4c-16g-240s` | 4 vCPU, 16 GB, 240 GB; two 2-CPU rooms at once | $111.69 | $0.153 |
| `fixture` | `vc2-1c-2gb` | 1 vCPU, 2 GB | $10.00 | $0.014 |
| **Total** | | | **$169.69** | **$0.233** |

Vultr bills by the hour up to the monthly cap, so the weekend costs about $5.60 a day. The VPC,
firewall groups, SSH key and DNS records cost nothing.

## Network and firewalls

```text
Internet ──80/443──▶ control (Caddy ▶ agentrix-node :8080, agentrix-models :8090 on loopback)
Internet ──22──────▶ control, from ADMIN_CIDR only
control ──VPC──────▶ sandbox :7443 (runner) and :22;  fixture :22
sandbox rooms ─────▶ agentrix-egress on 172.30.0.1:3128-3191 only ──▶ Internet (per-job allowlist)
Internet ──80/443──▶ fixture (Caddy ▶ WordPress on 127.0.0.1:8081)
```

- **Three layers must all accept a packet:** the Vultr group (public interface only), ufw
  (Vultr's Ubuntu image enables it with only 22 open; cloud-init and deploy.sh add exactly the
  ports below), and our `inet agentrix` nftables table.
- **Vultr firewall groups filter only the public interface.** `control` opens 80 and 443, and 22
  from `ADMIN_CIDR`. `sandbox` opens nothing. `fixture` opens 80 and 443.
- **Each host's nftables limits the VPC.**
  - The sandbox accepts the runner port and SSH from control's VPC address only.
  - The fixture accepts SSH from control only. Neither the fixture nor its containers can open
    connections into the VPC.
  - Each file replaces only its own `inet agentrix` table, so Docker's rules survive a reload.
- **Rooms (sandbox):**
  - Rooms run on the **internal** Docker network `agx-rooms` (bridge `agx-r0`, `172.30.0.0/24`),
    with no masquerade and no inter-container traffic. Being internal, Docker's embedded DNS
    forwards no outside names, so DNS is no side channel; the proxies resolve names.
  - Room slot `i` has the fixed address `172.30.0.(10+i)` and its own proxy port `3128+i`; each
    proxy serves only its room's address. Docker assigns dynamic addresses only from
    `172.30.0.128/25`, so the fixed ones never collide.
  - A room can reach only the egress proxies on the gateway, `172.30.0.1:3128-3191`. Forwarding
    from any `agx-r*` bridge is dropped, so the internet, the VPC and the metadata service are
    reachable only through `agentrix-egress`.
  - The proxies run as `agentrix-egress` (uid and gid 65533: no shell, no home, not in the
    `docker` group). nftables drops their new connections to private, link-local, metadata,
    CGNAT and loopback addresses (except the DNS stub on 127.0.0.53), in case the proxy has a bug.
  - gVisor is Docker's default runtime there.
  - If the firewall fails to load at boot, the sandbox powers off before Docker starts.
- **SSH.** Keys only, root login off: `00-agentrix.conf` must sort before Vultr's
  `50-cloud-init.conf`, because sshd keeps the first value it reads. R14.2 allows only 80/443 on control. SSH from one admin address is the exception that
  deploys need, and `provision.sh ssh-close --yes` removes it. Sandbox and fixture have no public
  SSH; reach them with `ProxyJump` through control (`.state/ssh_config`).

## Contracts other lanes build to

| What | Value | Used by |
|---|---|---|
| `agentrix-node` listens | `127.0.0.1:8080` (`NODE_PORT`) | Caddy on control |
| `agentrix-models` listens | `127.0.0.1:8090` (`MODELS_PORT`) | agentrix-node |
| node's internal listener | `127.0.0.1:8079` (`INTERNAL_PORT`), loopback only: `GET /internal/tls-ask?domain=` | Caddy's on-demand TLS |
| `agentrix-runner` listens | `0.0.0.0:7443` on the sandbox (`RUNNER_PORT`), reachable from control's VPC address only | The broker client in agentrix-node |
| Rooms network | Internal Docker network `agx-rooms`, bridge `agx-r0`, `172.30.0.0/24`; Docker assigns from `172.30.0.128/25`; room slot `i` is `172.30.0.(10+i)`; further room bridges must be named `agx-r*` | agentrix-runner |
| Egress proxies | Listen on `172.30.0.1`, port `3128+i` for room slot `i` (`3128-3191`), as uid and gid 65533 (`AGENTRIX_EGRESS_USER`) | agentrix-runner, agentrix-egress |
| Fixture upstream | WordPress published on `127.0.0.1:8081` | fixtures/wordpress-infected |
| Non-secret host facts | `/etc/agentrix/host.env` (the runner reads `AGENTRIX_ROOMS_NETWORK`, `AGENTRIX_ROOMS_GATEWAY`, `AGENTRIX_EGRESS_PORTS`, `AGENTRIX_EGRESS_USER`, `AGENTRIX_RUNNER_LISTEN`); `/var/lib/agentrix/host-facts.json` (KVM present, runsc, Docker, nftables) | deploy.sh, status |
| Secrets | `/etc/agentrix/secrets/` on control only (0700, root; files 0600). The sandbox and fixture hold none (N6). `models.env` holds the provider keys (written by `deploy.sh --secrets` from `./.env`). `gauntlet.token` is generated by deploy.sh. `objects.json` is optional: it holds the Object Storage keys (R14.4). Create a bucket (Vultr console or API), then write `{endpoint, bucket, region?, access_key, secret_key}` there, root 0600; the top-level README's *Run it yourself* has the steps. If it doesn't exist, deploy.sh writes it empty, which node reads as Object Storage off (systemd 255 won't take an empty `SetCredential=` default). Node gets both files through `LoadCredential`; for any other credential the node unit names, deploy.sh stops before changing anything if the file is missing | agentrix-models, agentrix-node |

**After first boot**, run the containment smoke test on the sandbox; every check must pass
before the first job:

```sh
ssh -F deploy/.state/ssh_config agx-sandbox 'sudo bash -s' < deploy/smoke/sandbox-rooms.sh
```

**The runner's unit** (`systemd/agentrix-runner.service`, agreed with the architecture lane):
- `User=agentrix`, `SupplementaryGroups=docker`, `NoNewPrivileges=yes`, work directory
  `/var/lib/agentrix-runner`.
- `AmbientCapabilities` and `CapabilityBoundingSet` of exactly `CAP_SETUID CAP_SETGID CAP_CHOWN
  CAP_DAC_OVERRIDE CAP_KILL`. CAP_KILL lets the runner stop each job's proxy, which runs as
  another user; without it, destroy-and-confirm fails.
- Each proxy starts through `setpriv --reuid=65533 --regid=65533 --clear-groups --inh-caps=-all
  --ambient-caps=-all --no-new-privs`.

The containment smoke test starts a proxy that way, with the unit's user and capabilities. It
checks the proxy's `/proc/<pid>/status` (uids and gids 65533, no other groups, inheritable,
permitted, effective and ambient sets empty, no_new_privs) and that the runner's user can stop it.

**Live sites over HTTPS:** Caddy's catch-all `https://` block gets certificates on demand,
after asking node's internal listener whether it serves the host. That covers owners' own
domains and the platform addresses `{domain-with-dashes}.<platform>`, through the wildcard record.

**Found at first boot (7 Nov):** the VX1 sandbox exposes `/dev/kvm` (`"kvm": true` in its host
facts), so microVM rooms are possible later.
