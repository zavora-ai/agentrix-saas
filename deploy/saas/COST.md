# saas-node: what it costs, and what keeps it cheap

The demo must stay up, unchanged, from submission (Thu 12 Nov 2026) until winners are announced (about 21 Dec). This page holds the numbers and the levers.

## The VM

`deploy/saas/provision.sh plan` reads the price from Vultr's API every time it runs. On 10 Nov 2026 it read:

| | Plan | Shape | Price |
|---|---|---|---|
| **Default** | `vc2-4c-8gb` (Cloud Compute, Atlanta, like the `agx-*` VMs) | 4 vCPU, 8 GB RAM, 160 GB SSD, 4 TB transfer | **$40.00 a month**, $0.055 an hour |
| Faster builds | `vhp-4c-8gb-amd` (`SAAS_VULTR_PLAN=vhp-4c-8gb-amd`) | 4 vCPU, 8 GB, 180 GB NVMe, 6 TB | $48.00 a month, $0.066 an hour |

saas-node was created on **Tue 10 Nov 2026, 17:34 UTC** (20:34 EAT): `c108befb`, 64.177.42.252, `vc2-4c-8gb` in Atlanta. Vultr bills by the hour, capped at the monthly price for each calendar month. Destroyed on 21 Dec, it comes to:

| Period | Hours | `vc2-4c-8gb` |
|---|---|---|
| 10 Nov 17:34 UTC – 30 Nov | 486 | $26.75 |
| 1–21 Dec | 504 | $27.72 |
| **Total** | 990 | **about $54.50** |

Kept to the end of December instead, it's $26.75 + $40.00 = **$66.75**.

These cost nothing:
- the firewall group `saas-fw`
- the SSH key `saas-admin`
- the four A records

Traffic stays far inside the 4 TB included.

## Models: a hard cap per day

The model gateway (`crates/agentrix-models`) runs inside `agentrix-saas` on the VM and meters into `/var/lib/agentrix-saas/models/control.db`. It enforces two limits, both set in `.env` and carried to the VM on every deploy:

| Setting | Value | What it limits |
|---|---|---|
| `SAAS_DAILY_MODEL_CAP_USD` | 3 | Both companies together, per UTC day |
| `AGENTRIX_SITE_DAILY_CAP_USD` | 2 | Each company, per UTC day |

- **Worst case**, if every day hit the cap: $3 × 41 days ≈ $123. The live demo won't come close.
- **Provider backstop:** the Anthropic key lives in its own workspace, with a $50 monthly limit (`specs/saas/partners.md`).
- **Vultr Inference** (the desks, extraction, embeddings and speech) goes through the same gateway and the same cap.

## Judge mode: the scripted model

- **The attack demos** (the bank-details PDF, the forged webhook, the crash mid-payout) replay with the scripted model, at zero model cost.
- **The board's Treasurer** answers from the feed by rules in judge mode. No model is called, and each read shows as a tool step.
- **The whole node** can switch to the scripted model with one deploy. That's the lever if spend ever runs ahead:

  ```bash
  deploy/saas/deploy.sh --models replay
  ```

  This sets `AGENTRIX_MODELS_MODE=replay`. `--models live` switches back.

## Nothing polls

- **Agents wake on events:**
  - PayPal's signed webhooks, pushed to `pay.agentrix.website/paypal/webhook`
  - ERA's events
  - due dates, which the kernel's timers handle (collections and instalments)
  - the founder's messages
- **Nothing outside the VM runs on a schedule:**
  - no cron jobs
  - no uptime pingers
  - no health checks from outside
  - no deploy watcher
- **Background work on the VM:**
  - Caddy renews its four certificates itself, a few ACME calls every 60 days.
  - ERA's own scheduler (recurring invoices, reminders) runs inside its API.
  - Ubuntu's unattended upgrades check once a day.
- **Logs are bounded**, so a month unattended can't fill the disk:
  - Docker keeps at most 3 × 10 MB per container.
  - The journal is capped at 500 MB.

## Snapshots

**After submission** (Thu 12 Nov, after 18:00 EAT), run:

```bash
deploy/saas/deploy.sh --snapshot --yes --wait
```

- **Cost:** Vultr bills a snapshot's compressed size at $0.05 per GB a month. The dry run (without `--yes`) shows the disk in use and the most it would cost. Expect about 25–35 GB in use, mostly build caches, so at most $1.25–1.75 a month.
- **The VM keeps running.** The snapshot is a restore point; nothing changes on the demo.
- **Shrinking it:** `sudo docker builder prune -f` on the VM drops ERA's build stages before a snapshot. Leave the Rust `target` folders, so a restore can rebuild quickly.

**After judging** (about 21 Dec, with James's yes), run:

```bash
deploy/saas/provision.sh destroy --yes
```

- It deletes only what `apply` created:
  - the VM
  - the four records
  - `saas-fw`
  - `saas-admin`
- Each object is checked against the state file's ids first.
- The snapshot stays until it's deleted in the console.
