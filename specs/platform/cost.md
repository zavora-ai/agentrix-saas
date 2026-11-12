# Cost: what we pay for, and how we keep it low

Owner: W12 · Cost management. Written Sun 8 Nov 2026, 00:50 EAT, from Vultr's API and the
gateway's meter (read only). Re-run `deploy/provision.sh costs` for today's numbers.

The aim: spend as little as we can without ever taking down what the submission and the judges
need.

## 1. What we pay for

### Vultr, today

Vultr bills each VM by the hour up to its monthly cap. For control and fixture the hourly price
is the monthly cap divided by 672 (28 days), so it runs about 8% above the list's
`hourly_cost` until the cap is reached. The README's $0.233 an hour is the list price; the bill
says $0.239 for the VMs.

| Piece | Billed per hour | Per day | Monthly cap | What it does | When we need it |
|---|---|---|---|---|---|
| `control` (`vhp-4c-8gb-intel`, 17 GB of 180 GB used) | $0.0714 | $1.71 | $48.00 | agentrix.website, every site, the console, the Model Gateway, `control.db` and its keys, the content store, builds | Always |
| `sandbox` (VX1 `vx1-g-4c-16g-240s`, 26 GB of 240 GB used) | $0.1530 | $3.67 | $111.69 | The runner and the sealed rooms: Assess, the Move, the replay. Four room images pinned by ID in `deploy/env/node.env` | Assess, Moves, every deploy (the smoke test checks the runner) |
| `fixture` (`vc2-1c-2gb`, 13 GB of 55 GB used) | $0.0149 | $0.36 | $10.00 | Zavora's infected WordPress at fixture.agentrix.website | Every deploy (the gauntlet re-runs against it), the demo Move |
| Object Storage, Standard, `ams1` | $0.0268 | $0.64 | $18.00 | Mirror of every blob and receipt; restores a lost release file | Always (it is the only copy off control) |
| VPC, firewall groups, SSH key, DNS zone `agentrix.website` | $0 | $0 | $0 | | Always |
| Snapshots | none yet | | $0.05 per GB a month of compressed size | Sleep and wake; the only backup of `control.db` until W8's lands (§5.3) | See §2 |
| Reserved IPs | none | | $3 a month each | Not needed (§2.2) | |
| Bandwidth | included | | 2 to 6 TB a month per plan | | |
| **Total now** | **$0.266** | **$6.39** | **$187.69** | | |

**The bill so far** (Sun 8 Nov, 00:38 EAT): $1.37 pending this month, for about 5 hours since
the VMs were created on Sat 7 Nov at 20:19 EAT. The account holds $10.00 of credit, which covers
about a day and a half at this rate; after that the card on file pays the monthly invoice.

**One snapshot of each VM** costs at most $2.80 a month (56 GB in use × $0.05; Vultr bills the
compressed size, so less).

### Model calls

The first release's gateway meters every call into `control.db`'s `usage` table. W8 is building the read
API (`agentrix_store::metrics`); until it lands, the numbers come from that table.

| | Calls | Cost |
|---|---|---|
| `build` | 102 | $0.570 |
| `router` | 100 | $0.009 |
| `vision-parity` | 52 | $0.006 |
| `brief` | 8 | $0.005 |
| `screen` | 108 | $0.004 |
| `decide` | 2 | $0.0003 |
| **All, Sat 7 Nov (UTC)** | **372** | **$0.59** |

- All 372 calls ran on Anthropic routes (Phase 1). They are paid to Anthropic, not on the Vultr
  bill. There have been no Vultr Inference calls yet; after the S3-10 switch every purpose runs on
  Vultr, at about $0.0011 a page (the bake-off).
- The first Move cost $0.22 and the second $0.37.
- **Limits in force:** the gateway refuses a site's calls once it has spent $5 in a day
  (`AGENTRIX_SITE_DAILY_CAP_USD`). The `budgets` table is empty, so that cap is the only limit.
  There is no fleet-wide cap: every sample site and every site a judge assesses gets its own $5 a
  day. §5 asks for one.

### Images and voice (W11, on Vultr Serverless Inference)

The prices are from Vultr's catalogue, 8 Nov. Every call goes through the gateway under purposes
`image`, `voice.listen`, `voice.live` and `voice.speak`. Its rows carry `units` and `unit`
(`image`, `audio_second`) and are priced from Vultr's own `usage.cost` where Vultr reports it.
Budgets and the daily cap apply as for every other purpose.

| Model | Used for | Price |
|---|---|---|
| `z-image-turbo` | Pictures (the default) | $0.02 a megapixel: $0.0118 for a 768×768 product shot, $0.0206 for a 1344×768 hero |
| `qwen-image-2.1` | Edits only | Free while Vultr evaluates it. Its Qwen Research License goes to James before any live site uses it |
| `nemotron-3.5-content-safety` | Screens each picture (about 750 tokens in, 70 out) | about $0.00005 a picture |
| `qwen3.8-27b` | Alt text | about $0.0003 a picture |
| `voxtral-mini-4b-realtime` | Live owner speech | $0.000133 a second, $0.48 an hour |
| `whisper-large-v3-turbo` | Transcription | $0.0000111 a second |
| `qwen3-tts-12hz-1.7b` | Speech out | $0.00028 a second, $0.017 a minute |

**Made once.**
- A CLI draws the sample and home-page pictures once. They are committed with a manifest
  (digest, alt text, label, seed), embedded in the binary, and seeded into the content store by
  digest at startup.
- They are never redrawn on a deploy, a page load or a test. Tests use a fake Vultr or replay.
- In production, an owner's "draw again" is a new picture with a new seed, metered against the
  site's budget.

**The one-off seeding is done: $1.21** on Vultr Inference. That's from W11's local gateway's
own usage rows, live mode, Sun 8 Nov from 00:40 EAT, reported at 08:22 EAT.

| What | Calls | Cost |
|---|---|---|
| Pictures, `z-image-turbo` | 85 drawn | $1.14 |
| Alt text, `qwen3.8-27b` | 72 | $0.056 |
| Screening, `nemotron-3.5-content-safety` | 86 | $0.004 |
| Voice checks (25 s of speech, 1 s of live transcription) | | $0.007 |
| **Total** | | **$1.21** |

- **55 pictures shipped:** Kilele 13, Classy Perfumes 31, Tana Valley Times 11. They're
  embedded and seeded by digest, and never redrawn.
- **The other 30 were waste:** 13 lost to the screening bug (about $0.21, fixed in e73d93b)
  and about 12 redraws where a picture showed things the product doesn't have.

**Running cost from now** is only what someone asks for. All of it is metered under `image` and
`voice.*` with units, inside each site's budget and the daily caps.
- An owner's picture in the console costs about $0.012 at 768×768 or $0.021 for a hero, plus
  about $0.001 of checks.
- Speech costs $0.017 a minute; live transcription $0.008 a minute.
- The Move's fill drafts at most 5 pictures a run, about $0.07.

## 2. Sleep and wake

### 2.1 When each VM can sleep

| Period | control | sandbox | fixture |
|---|---|---|---|
| Now to Sun 8 Nov 20:00 EAT (the first release's windows A to D) | up | up | up |
| Sun 20:00 to Mon 9 Nov 02:00 (lablab closes) | up | up | up |
| **Judging** (from Mon 02:00 until lablab's results are final) | up | up | up |
| After judging, between working sessions | up | asleep | asleep |
| After judging, a pause longer than a week | James decides (§2.3) | asleep | asleep |

**When judging ends: not published.** On Sun 8 Nov, 00:50 EAT, lablab's event page, its live page
and Vultr's community event page gave only the build dates (3 to 8 Nov) and the on-site day in Salt
Lake City (8 Nov). None gave a judging period or a results date. Until lablab marks the event's
results final, or the organisers say otherwise, judging is running, and nothing sleeps.
`SLEEP_NOT_BEFORE` in `deploy/.env` stays empty until then, and `provision.sh sleep --yes`
refuses while it is empty.

**What judging costs:** $6.39 a day with everything up, plus model calls. Two weeks is about
$90; a whole month can't pass the caps, $187.69.

### 2.2 Each VM

**control: keep running.**
- It serves agentrix.website and every site.
- It holds `control.db` and the keys, which nothing else backs up yet (02 §8).
- The sandbox's and the fixture's firewalls name its VPC address (10.42.0.3).
- Owners' domains will point at its public address.

Sleeping it would save $1.71 a day. A rebuild would need a reserved IP ($3 a month, converted
from today's address so no DNS changes) and new firewall files on the other two hosts.
`provision.sh sleep control` refuses.

**sandbox: snapshot and delete when idle, after judging.** It saves $3.67 a day.
- **Why a snapshot, not a rebuild.** The four room images were built on the box and node.env pins
  their IDs. A rebuild from cloud-init makes new IDs, which would need a commit and a deploy. The
  snapshot keeps the images, their IDs, the runner and egress binaries, control's job key, and
  the nftables and ufw rules that name control.
- **What changes on wake.**
  - The public address changes. Nothing uses it: no DNS name, nothing inbound.
  - The VPC address is Vultr's choice and may not be 10.42.0.4. `wake` writes the new
    `RUNNER_ADDR` into `deploy/.state/inventory.env`. `deploy.sh` now installs node's
    `AGENTRIX_RUNNER` from there, so the next deploy points node at it with no commit.
- **Reserved IP:** not worth it. Nothing needs the sandbox's public address.

**fixture: snapshot and delete with the sandbox, after judging.** It saves $0.36 a day.
- **Why a snapshot, not a rebuild.** It keeps WordPress exactly as the gauntlet knows it (its
  posts, the payloads, `home` and `siteurl` on fixture.agentrix.website) and Caddy's
  certificate.
- **What changes on wake.** The public address changes, and `wake` rewrites the fixture's A and
  AAAA records (TTL 300, so up to 5 minutes for resolvers). The VPC address goes into
  `ssh_config`.
- **Reserved IP:** not worth it. $3 a month would only save a scripted DNS update.

**Both:**
- A sleeping sandbox or fixture stops deploys: the smoke test checks the runner, and the gauntlet
  reads the fixture. Wake both before a deploy.
- A sleeping sandbox also stops Assess. After judging, W10's home page and W4's onboarding
  should say so honestly ("Assess is resting; back at …") rather than fail.
- Sleeping both saves $4.03 a day, up to $121.69 a month, less at most $1.95 a month of
  snapshots. Asleep, the platform costs $2.36 a day (control and Object Storage).

**Object Storage, the VPC, the firewall groups and DNS stay.** Storage is the backup, at a flat
$18 a month; the rest cost nothing.

### 2.3 Everything offline: Sun 8 Nov, for local development

James decided at 07:25 EAT on Sun 8 Nov that the cloud setup is done and tested. All three VMs
are snapshotted and taken offline until local development is finished. This overrides 2.1 for
now.

| Snapshot (Vultr console) | Taken (EAT) | Size | A month |
|---|---|---|---|
| `agx-b1-control-11-08-2026` | 07:23 | 5.0 GB | $0.25 |
| `agx-b1-sandbox-11-08-2026` | 07:23 | 9.6 GB | $0.48 |
| `agx-b1-fixture-11-08-26` | 07:19 | 3.8 GB | $0.19 |

- The last deploy before going offline was 33220a0 at 07:10 EAT, and the deploy lock was free.
- **Delete a VM only after its snapshot says Complete.** Deleting it while the snapshot is
  pending can lose the snapshot.
- **Delete the instances only.** Keep the VPC `agx-b1`, the three firewall groups, the SSH key, the
  DNS zone and Object Storage. All are free except storage. Don't run `provision.sh destroy`: it
  also deletes the VPC and the groups that the restore reuses.
- **Offline cost:** Object Storage $0.64 a day plus snapshots about $0.03 a day, about $0.67 a
  day. That saves $5.72 a day against running all three.
- **Down while offline:** agentrix.website and every `*.agentrix.website` host, the console, the
  runner and rooms, the Model Gateway on control, and fixture.agentrix.website. DNS still points
  at the old addresses.
- **What still works:** Vultr Serverless Inference (not a VM) and the bucket.
- **The only copy of the live `control.db`** and control's keys is the control snapshot (and its
  receipts and blobs in the bucket). Don't delete that snapshot.

#### Restore runbook

From the first release checkout. It needs James's yes. W12 runs it, or James runs it with W12
watching.

Until W12's `deploy/` commits are merged, run W12's copy of the script against the first release
checkout's settings and state:

```sh
B1=/Users/jameskaranja/Developer/hackathons/agentrix-webagent-platform
AGENTRIX_DEPLOY_ENV=$B1/deploy/.env AGENTRIX_DEPLOY_STATE=$B1/deploy/.state \
  $B1/.claude/worktrees/w12-cost-090e42/deploy/provision.sh wake control sandbox fixture
```

It was tested against a fake Vultr API in three cases: the addresses come back the same,
control's moves, and only the sandbox's moves.

1. **Check nothing else is changing.** `deploy/deploy.sh --status` must say "lock: free"; it will
   say control is unreachable until step 3.
2. **Plan:** `deploy/provision.sh wake control sandbox fixture`. The dry run names each snapshot
   and the hourly cost each VM adds back.
3. **Wake:** `deploy/provision.sh wake control sandbox fixture --yes`. It always restores control
   first, then the sandbox, then the fixture, one at a time, into the kept VPC. Each comes back
   with:
   - its label, plan and firewall group;
   - cloud-init asked to keep the snapshot's SSH host keys (`ssh_deletekeys: false`); control's
     changed anyway on 8 Nov.

   It then:
   - rewrites the DNS for `agentrix.website`, `*.agentrix.website` and the fixture (TTL 300);
   - updates `deploy/.state/inventory.env` and `ssh_config`;
   - checks each new address's host key against the one `~/.ssh/known_hosts` held for the old
     address before trusting it;
   - starts the fixture's WordPress and database (`docker compose start` in `/opt/fixture`);
   - waits up to 5 minutes for control's `/healthz`, the runner's 404 and the fixture's 200, and
     exits non-zero, naming the VM, if one isn't serving;
   - logs the times in `deploy/.state/snapshots.log`.
4. **VPC addresses.** In an empty VPC the first VM attached gets the first address, as when
   `apply` created them: control 10.42.0.3, then the sandbox 10.42.0.4, then the fixture
   10.42.0.5. `wake` checks this:
   - **If control's address moved**, it stops before the others. Their snapshots' nftables and
     ufw accept SSH and the runner only from 10.42.0.3. Fix them from Vultr's web console, or
     rebuild them with `apply` (new room images and IDs, so a `node.env` commit).
   - **If only the sandbox's address moved**, `inventory.env` gets the new `RUNNER_ADDR`, and
     the next `deploy.sh` installs it as `AGENTRIX_RUNNER` (W12's `deploy.sh` change).
5. **Admin SSH.** If James's public address changed, set `ADMIN_CIDR` in `deploy/.env` and run
   `deploy/provision.sh apply --yes`. It reuses the three VMs by label, re-adds the SSH rule and
   rewrites DNS and the inventory again.
6. **Check the stack before any platform deploy:**
   1. `deploy/provision.sh status` shows the three VMs active and cloud-init done.
   2. `deploy/deploy.sh --status` shows "deployed: 33220a0…" and "lock: free".
   3. The coordinator redeploys the commit that's already there, `deploy/deploy.sh 33220a0`. It
      changes nothing and re-runs the smoke test (the runner from control, TLS, the public pages,
      the rooms) and the gauntlet on every live site. It must print 10/10 with a
      `gauntlet.run` receipt.
   4. Only then platform's Deploy A, then the rest.
7. **Tell everyone.** W12 posts the restore and the measured times on the board and messages
   every session.

**What the restore costs:** $0.266 an hour again from the moment the VMs exist. Vultr bills whole
hours.

### 2.4 What a restore must bring back

| Must come back | How |
|---|---|
| Public IPs and DNS | Control keeps its address (it never sleeps). The fixture gets a new one; `wake` rewrites its A and AAAA records. The sandbox has no DNS |
| The sandbox's VPC address that node calls | `wake` records it in `inventory.env`; `deploy.sh` installs it as `AGENTRIX_RUNNER` |
| Room images pinned by ID | On the snapshot's disk, unchanged |
| SSH host keys | `wake` passes `ssh_deletekeys: false` to cloud-init, so the snapshot's keys stay. If the VPC address moved, `wake` asks control for the key at the new address, compares it with the one recorded at snapshot time (`.state/hostkeys/`), and only then adds it to `~/.ssh/known_hosts` |
| Firewall groups | Kept while asleep; `wake` puts each VM back in its own group |
| VPC | Kept while asleep; `wake` attaches it |
| The gauntlet against the fixture | The fixture's disk is the snapshot; the first deploy after waking re-runs the gauntlet as usual |

### 2.5 Restore time

**Measured on Sun 8 Nov.** W0 ran `wake control sandbox fixture --yes` from 17:58 EAT, at James's
word, from the snapshots taken at 07:19 to 07:23. The times are from `deploy/.state/snapshots.log`.

| VM | Snapshot | Disk restored | Serving | Notes |
|---|---|---|---|---|
| control | 5.0 GB | not logged | not logged | Back at 64.177.51.184, VPC address 10.42.0.3. Its SSH host key was new, so wake stopped at the key check; W0 checked the key and accepted it, then woke the other two |
| sandbox | 9.6 GB | 12 min 33 s | 12 min 55 s | VPC address 10.42.0.4, as before, so no deploy was needed for the runner's address |
| fixture | 3.8 GB | 5 min 43 s | 9 min 10 s | Back at 64.177.57.247, VPC address 10.42.0.5. It answered 502: WordPress and its database stayed stopped (see below). W0 started them by hand and it answered 200 |

- **Plan on about 13 minutes per VM, one after another,** so 30 to 40 minutes for all three,
  plus a deploy if the sandbox's address ever moves. The VPC handed out .3, .4 and .5 again,
  in order.
- **The fixture doesn't start its containers on boot.** Its compose project
  (`fixtures/wordpress-infected/compose.yaml`) has no restart policy, so a restored VM boots
  with `agx-fixture-db-1` and `agx-fixture-wordpress-1` stopped.
  - `wake` now runs `docker compose start` in `/opt/fixture` (c9716e5).
  - It reports a VM as serving only when that VM's check passes, and exits non-zero otherwise.
  - The lasting fix is `restart: unless-stopped` on `db` and `wordpress` in the fixture's compose
    file. That file belongs to the first release Gauntlet builder, so it's an ask on the board.
- **Host keys:** `ssh_deletekeys: false` didn't keep control's key. Expect to check a new key in
  Vultr's web console after a restore.

`provision.sh restore-test ROLE --yes` can still time a throwaway copy, outside the VPC and with
nothing inbound, without touching anything live (about $0.17 for the sandbox, $0.02 for the
fixture).

### 2.6 The commands

All in `deploy/provision.sh`, run from the first release checkout. Each is a dry run without `--yes`.

```sh
deploy/provision.sh costs                          # read only, any time
deploy/provision.sh snapshot [ROLE…] [--yes]       # changes nothing
deploy/provision.sh restore-test ROLE [--yes]      # a throwaway copy, timed, then deleted
deploy/provision.sh sleep sandbox fixture [--yes]  # fresh snapshot, then delete
deploy/provision.sh wake sandbox fixture [--yes]   # back from the latest snapshot
```

## 3. Rules for acting

- **Snapshots** cost little and change nothing. W12 takes them outside the first release's windows A to D
  and logs each one on the board.
- **Sleeping, waking, restore tests and destroying** need James's yes on the board first. The ask
  names the exact command, what goes down, the saving per day and the restore time.
- **Nothing sleeps before judging ends.** Judges use the live site, Assess needs the sandbox and
  every deploy needs the fixture. `SLEEP_NOT_BEFORE` enforces it.
- **Never during a deploy.** `snapshot` and `sleep` check the deploy lock
  (`deploy/deploy.sh --status`) and refuse while it is held.
- Only W12 runs these commands, and only from the first release checkout. Merging them goes through the
  control-plane session's review (they are `deploy/` changes).

## 4. Cost rules for every session

1. **Tests call models in replay mode only** (`AGENTRIX_MODELS_MODE=replay`), never live. A
   recording is made once, on purpose, and committed.
2. **No session touches the VMs.** No SSH, no `provision.sh` beyond `plan`, no hand changes. Deploys
   are the coordinator's; snapshots, sleep and wake are W12's.
3. **Generated images and speech are made once,** stored by digest in the content store and
   reused. Check the digest before generating; never regenerate on a deploy, a page load or a
   test (W11, W2, W10).
4. **The Model Gateway's limits stay on for the samples:** per-site budgets and the daily cap.
   Nobody raises or removes a cap to make a demo work; ask W12 and the coordinator.
5. **Each session builds only its own crates,** against the shared cache
   (`CARGO_TARGET_DIR=/Users/jameskaranja/Developer/hackathons/.agentrix-platform-target`), and
   never a full `--workspace` release build.
6. **Anything that adds a paid Vultr resource** (a VM, a bucket, a reserved IP, a new inference
   model) goes to W12 and James first.

## 5. Production cost controls

What 02 §8's cost-controls row needs, beyond what the gateway already does (reserve-then-settle,
per-site and per-purpose budgets, record and replay).

### 5.1 The cost picture in the control plane (W3, from W8's reads)

- **Cost per Move** = its model spend (`usage` by `move_id`) + its room minutes × the sandbox's
  hourly price ÷ 60.
- **Cost per site per month** = its model spend + its share of the platform:
  - the sandbox, in proportion to the site's room minutes;
  - control and Object Storage, split evenly across live sites (samples and fixtures are
    operator cost, not a site's).
  Idle sandbox time is platform overhead, shown on its own line, never charged to a site.
- **Against the plan price in USD:** each site's monthly cost next to what its plan charges, with
  the margin.
- **A sleeping WebAgent costs nothing.** It has no process, no model calls and no room. Its state
  is rows in `control.db` and blobs in the content store, already paid for. A live site's floor is
  its share of control and storage.

### 5.2 Budget alerts

- A site's model spend at 50%, 80% and 100% of its monthly budget, and of the daily cap.
- **A fleet-wide daily cap** in `agentrix-models`, plus one shared cap for the sample sites, which
  bill to the operator. The first release gives each site its own $5 a day, and nothing caps the total.
  W8 built both caps (38b7dd2 on its branch), and the coordinator merges them before any sample
  goes live. The settings and their defaults:
  - `AGENTRIX_FLEET_DAILY_CAP_USD`, **$8** for all sites together per UTC day. That holds a bad
    day under the Vultr credit left on Sun 8 Nov;
  - `AGENTRIX_SAMPLES_DAILY_CAP_USD`, **$3**, shared by every `-sample` host;
  - `AGENTRIX_SITE_DAILY_CAP_USD`, **$5** per site, as before.

  How the caps behave:
  - 0 refuses everything; it never means unlimited.
  - The checks run in this order: the site's budgets, the site's daily limit, the samples, the
    platform. The refusal names the cap that tripped.
  - The answer is 429 `paused_for_today`, with Retry-After set to midnight UTC (03:00 EAT).
- **The platform cap during judging.** If it trips, every model call pauses until 03:00 EAT: a
  judge's Move, Ask this site and the brief. Assess makes no model calls (its detectors are
  code), so it keeps working. The live value follows James's answer on automatic billing:
  - if the card pays automatically, set `AGENTRIX_FLEET_DAILY_CAP_USD=20` in `models.env`. That's
    about 50 Moves on Claude, or several hundred on Vultr;
  - if it doesn't, keep $8 and watch the trips hourly.

  The Move page says "paused for today" honestly when it happens (W4), and so does every other
  surface that calls a model.
- **Trips are `usage` rows** with `outcome = 'over-budget'` and `cost_micro_usd = 0`. The
  `tier_reason` is "refused: platform daily limit ($8.00 spent of $8.00)" or "refused: samples
  daily limit (…)"; a site's own limit says "refused: over budget". W12 alerts on the board from:
  `SELECT site_id, at, tier_reason FROM usage WHERE outcome = 'over-budget' AND tier_reason LIKE 'refused: % daily limit%'`.
- The Vultr bill: an alert when month-to-date charges run more than 20% above the forecast from
  the hourly rate.

### 5.3 The control.db backup (W8's proposal, for the control-plane session)

The backup adds no VM and no service. Every 15 minutes, a timer on control writes an
age-encrypted copy of `control.db` and `keys/` to the existing bucket, under `backups/control/`.
It costs nothing extra while the bucket stays inside the Standard tier's flat $18 a month.
- **Live size on Sun 8 Nov, 07:15 EAT:** `control.db` is 2.7 MB, its WAL 4 MB and `keys/` 16 KB.
- **Per day:** a compressed copy should be well under 2.7 MB, so 96 copies a day is at most about
  250 MB.
- **A month:** at most about 8 GB with a 30-day lifecycle rule on the prefix.
- **Without the rule** the copies grow with the database and are never deleted. W12 recommends
  the rule (James's yes).

Once the backup runs, a snapshot of control is no longer the only copy of `control.db`.

### 5.4 Reconciliation with Vultr

A daily job reads Vultr's `/v2/billing/pending-charges` with a read-only key on control (a new
secret, so a control-plane change). It writes the lines to `infra_cost_lines`, a table W8 owns (in
its 08xx range) and serves as `infra_spend(window)` beside `spend()`:

| Column | Type | Notes |
|---|---|---|
| `line_id` | TEXT, primary key | sha256 of source, product, description and period start |
| `period_start`, `period_end` | TEXT, UTC | Vultr's `start_date` and `end_date` |
| `product` | TEXT | "Vultr Cloud Compute", "Object Storage", snapshots, reserved IPs |
| `description` | TEXT | Vultr's line, with the `[label]` |
| `role` | TEXT, nullable | `control`, `sandbox`, `fixture`, `storage`, from the label |
| `units`, `unit_type` | REAL, TEXT | for example 5 hours |
| `unit_price_micro_usd`, `total_micro_usd` | INTEGER | money in micro-USD, like `usage` |
| `source` | TEXT | `vultr-pending-charges` or `vultr-invoice` |
| `fetched_at` | TEXT, UTC | |

Model spend is reconciled the same way: the gateway's `usage` rows for Vultr against Vultr
Inference's usage report, and for Anthropic against its console. A drift above 2% raises an
alert.
