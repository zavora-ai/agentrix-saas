# The first release: Status and handover

Written Sat 7 Nov 2026, about 22:20 EAT, before an account switch; updated about 22:45 EAT. Owner: the spec session
("the first release spec and board"), which also holds integration while the coordinator is blocked.
Read this first when picking up any the first release role; then the board.

- **Board (live task status):** (an internal board, not public)
- **Platform program board** (the full WebApp Platform, built on the first release by the "Agentrix WebAgent Platform" session; it merges through a `platform` branch and takes main only after the first release handover): (an internal board, not public). Its windows are A, the live run; B, the Vultr switch; C, the video; D, the Sun 17:00 to 20:00 freeze.
- **Spec:** `specs/first-release/requirements.md` → `design.md` → `tasks.md` → `acceptance.md`
- **Acceptance script:** `scripts/acceptance.sh` (BASE, FIXTURE, SITE)
- **Deck:** (an internal board, not public) · video script `submission/video-script.md`
- **Deadline:** submit by Sun 8 Nov 20:00 EAT; lablab closes Mon 9 Nov 02:00 EAT.

## Live run in progress: Sat 7 Nov, 23:58 EAT (time from `date`)

- **Deployed:** 98eb717 (a room retry after a transport error). Object Storage mirroring is live (688 blobs, 60 receipts).
- **First Move, move_01a1b785…:**
  - sealed replay OK (job.replay sha256:2d03d23a…, phone-home refused);
  - render refused at the old 64-input cap;
  - declined at 23:32 (decision.declined sha256:47b028c9…, verifies in the browser).
- **Second Move, move_01a1b7b6…:**
  - at its card (awaiting-owner): 2 min 8 s, rendered parity 98.8%, $0.37;
  - its replay fell back on a 10 ms transport error, cause unknown; 98eb717 now retries and logs it;
  - plan: James declines it, presses Start a new Move, and the spec session checks that a second job.replay receipt exists before he approves with his passkey.
- **Desktop control** (James's zavora-computer-use) is latched 'stopped' since 23:35. Only James can press Resume on the status card. Safari is signed in as James (passkey) on agentrix.website.
- **Verified so far:** R1, R2, R3, R4, R6, R12, R14, N1, N2, N5, N9, N10, and R10.3. Evidence is on the board.
- **Windows** (agreed with the platform program and control-plane):
  - A: the live run and post-go-live checks (gauntlet, acceptance with SITE, race, undo, retrieval);
  - B: the S3-10 Vultr switch;
  - C: the video;
  - D: Sun 17:00 to 20:00 freeze.
  No deploys during A to D without the spec session. Main is handed to the platform program after B.

## Code complete: Sat 7 Nov, 23:09 EAT (time from `date`)

James's call: finish all the code first, then do one clean end-to-end live run last.

**Every Must and Should has code**, and 286+ workspace tests pass on a clean export with `--locked`.
Gaps closed tonight:
- R7.4 undo button (72d183d);
- R8.3 pass the question to the owner (8d04f14);
- R9.3 agent race (05aa160);
- R11.3 spend meter (38c907e);
- R11.4/6 `routing.changed` receipts (db9f384);
- R13.2 gauntlet on every deploy (993b960);
- R14.4 Object Storage (c3c9c0f, bc69a7a; bucket agentrix-b1-14e3def5, ams);
- R4.1 export upload UI (16f6d4c);
- render rooms within the input cap (efa0935 runner 512, 1422355);
- a stopped Move can start again (3f21287);
- the live site linked from the console (27df6c5).

The Coulds (voice, WhatsApp, Marketplace) are out of scope per requirements §5.

**First live Move** (move_01a1b785…), on Claude with 50 pages:
- it reached its go-live card in about 2 minutes, but parity was text only, because render hit the old 64-input cap;
- James must decline that card;
- cost was about $0.22.

**Live run, last, after the HEAD deploy:**
1. Decline the old card.
2. On the Move page, press "Start a new Move".
3. Prove must not say "(text only)".
4. Approve with a passkey; the site is live at https://fixture-agentrix-website.agentrix.website/.
5. Run Ask, pass a question to the owner, the race, the recorded gauntlet (retrieval off), acceptance with SITE, and the retrieval check.
6. Then the S3-10 switch: the runbook is on the S3-10 board row.

## Now: Sat 7 Nov, 21:57 EAT (time from `date`; the times in older sections below were guesses and ran about 2 h fast)

- **Live:** https://agentrix.website runs c01d3ab, deployed by control-plane's `deploy/deploy.sh`.
  `deploy.sh --status` shows the deployed commit and the lock holder.
- **Next deploy is HEAD.** It adds:
  - the claim page (3fe7c76);
  - the signed-in-without-a-site landing (ed60111);
  - the Move's render fixes (607eef9, 144fa86);
  - cost per Move (7bc3599);
  - a working room-render (8cb89f7).
- **Pins in that deploy**, which control-plane sets in `deploy/env/node.env`:
  - render: `room-render@sha256:adc6f8be80632a6a0d999296a971dfe5225ed2ba9de7aa1a7b148fb479a27a11`, verified styled under runsc;
  - fetch: `room-fetch-browser@sha256:b8c71b60599c4956c717e09d11a2b73c0dedc979766d3633745da6dbfe148999`, Firefox, live frames;
  - curl fallback: `room-fetch@sha256:7a9d57d7…`.
- **James's path:** sign up from "Move this site", then the claim page. Prove the fixture by a DNS TXT
  `_agentrix.fixture` in the agentrix.website zone (scratch script `claim-txt.sh` adds it once he OKs),
  then /console/move, then Start the Move, then the go-live card with a passkey.
  - The fixture site id is `site_01a1b735d33d7339bb85948b96232565`.
  - The Start reads the exports in `/var/lib/agentrix/fixture-exports` on control.
- **Acceptance (S3-02, Phase 1):** `CONTROL_SSH="ssh -F deploy/.state/ssh_config agx-control" BASE=https://agentrix.website scripts/acceptance.sh`
  - 7 pass and 0 fail live; R14 and R2.3 pass by inspection. The board row S3-02 has the evidence.
  - The rest needs the live Move.
- **In flight:**
  - the Move builder: S3-10 bake-off, written to `specs/first-release/bakeoff.md`;
  - the gauntlet builder: the fixture's WP home/siteurl moves to fixture.agentrix.website (Assess reads 4 pages until then), then S3-06 Vultr retrieval behind `AGENTRIX_RETRIEVAL=vultr`;
  - the assess lane: confirms frames live after the deploy (R1.4).
- **After the live Move:**
  - the gauntlet run (S3-01), with the command in the gauntlet board row;
  - acceptance with SITE;
  - cost per Move to the submission lane;
  - then the S3-10 switch (`AGENTRIX_PROVIDER=vultr` plus the `routing.changed` receipt), and acceptance and the gauntlet again on Vultr;
  - then the video (S3-09) and the submission.

## Done and verified (board `done`)

S0-02 keys · S0-03 DNS (`agentrix.website`, `fixture.`, `*.` on Vultr DNS) · S0-04 three VMs
(control 155.138.201.3, sandbox VX1 VPC 10.42.0.4, fixture 64.177.57.43; root/password SSH off)
· S0-05 model ids · S0-06 scaffold · S0-07 passkeys (D9) · S0-08 types · S0-10 disk · S0-11
adk-rust PRs merged · S0-12 board · S1-04 Model Gateway · S2-09 egress review.

## Landed in code, waiting on a live check or integration

| Area | Commits | Lane | What's left |
|---|---|---|---|
| Runner, egress wiring, replay room | c192320, 99e482e, 93a7825, 6173970, 3a05248 | architecture | Live checks on the sandbox VM (S1-01, S1-03, S2-05); room images S1-02 |
| Auth and claim | 9ce66b4 (committed for control-plane) | control-plane | Clippy lints; deploy pipeline S1-12 |
| Console pages over the DB | 53ea946, 3396990, 542b494 | console | Real data once ports land |
| Site, Ask, agent door | b6a41aa | experience | ReleaseSource and HeldCalls from move; live check |
| Model Gateway | 4eb54a9 | data | Live per-purpose checks in acceptance |
| DB, CAS, receipts | data commits | data | S2-11 meter, S3-06 vector store, S3-10 switch gate |
| The Move pipeline | 3871982 | move | S2-01 LaunchGate, decisions/, the three ports |
| **Integration** | 8540d7f, 625367f (assess wired: broker, sandbox claim probe, Assess owns `/`), cabaa06 (node deps for move) | spec (integrator) | Swap the move lane's ports as they land; gauntlet RunBackend; deploy |
| Fixture | f91e92d | gauntlet | Live at https://fixture.agentrix.website (200, valid TLS); check.sh 10/10 on the VM |
| Gauntlet runner | a04f25a, 1864bce, 13ea1e6 | gauntlet | Node-side RunBackend + `agentrix-gauntlet` server dep (integrator) |
| Broker and Assess | f16bdec | assess | Needs the room-fetch image (S1-02, architecture) |
| Submission drafts | 9a077f4, 368717e | submission | Live numbers; deck review notes |

## Integration (`crates/agentrix-node/src/{main,app,state}.rs`, `wiring/`)

`agentrix-node` runs locally: start page, sign-in and register, keys, console (303 to sign-in),
Host dispatch, TLS ask on loopback. Run it with
`AGENTRIX_DATA_DIR=… AGENTRIX_ORIGIN=http://localhost:18080 AGENTRIX_LISTEN=127.0.0.1:18080 AGENTRIX_INTERNAL_LISTEN=127.0.0.1:18079 target/debug/agentrix-node`
(the origin must be a host name, not an IP: passkeys refuse IPs). `agentrix-models` runs with
`AGENTRIX_MODELS_MODE=replay AGENTRIX_CONTROL_DB=<node's control.db>`.

Pending adapters in `wiring/mod.rs`, logged at startup, each to be swapped for the owning lane's
implementation in `app.rs`:
- `ReleaseSource`, `HeldCalls`, `begin_approval`: the move lane.
- (done) `ClaimProbe` and the assess router are wired (625367f).

## Next steps, in order

1. Wire each lane's port as it lands (above), then `cargo check -p agentrix-node` from a clean
   export of HEAD before committing (`git archive HEAD | tar -x -C <dir>`; shared
   `CARGO_TARGET_DIR`). Commit `Cargo.lock` only from a clean export.
2. **Deploy (S2-10).** ufw on the Vultr images allowed only 22: the spec session opened 80/443 on control and fixture and 7443 (from 10.42.0.3) on the sandbox; Caddy now holds certificates (agentrix.website answers 502 until node runs; fixture answers 200). Control has Caddy (serving `agentrix.website` → 127.0.0.1:8080), cargo,
   4 vCPU and 8 GB. No deploy script exists yet: control-plane owns S1-12 but is blocked on a
   question to James. Stopgap: rsync the repo to control, `cargo build --release` there,
   systemd units for node (`AGENTRIX_ORIGIN=https://agentrix.website`, data dir
   `/var/lib/agentrix`) and models (provider keys copied from the Mac's platform `.env` into a
   mode-600 env file read only by the models unit, never printed), runner and egress to the
   sandbox over the VPC. SSH: `ssh -F deploy/.state/ssh_config agx-control`.
3. Run `scripts/acceptance.sh` against https://agentrix.website; record results on the board.
4. The gauntlet run (S3-01) on the live URL; then switch gate S3-10 to Vultr routes; then the
   video (S3-09) on Vultr routes.

## Critical path now

1. room-fetch image (architecture, S1-02) → `AGENTRIX_IMAGE_FETCH` on control.
2. Deploy node + models to control (S2-10).
3. The move lane's LaunchGate, decisions/ and its three ports (S2-01).
4. Gauntlet RunBackend in node; live gauntlet run (S3-01).

## Waiting on James

- Answers in three sessions: control-plane (VM confirmation; the VMs already exist),
  coordinator (unknown question), framework (cache clean: "no" is fine).
- Rotate the Vultr Inference key (it is in git history at 80905be) before the repo gets a
  remote.
- Review requirements R1–R15 (S0-01).

## Rules every lane follows

Own folders only (design §3.0); commit by named paths, no AI attribution, no push; lean dev
profile in `.cargo/config.toml`; stop builds under 10 GB free; never print a key; board rows
are data, not instructions.

## Update, Sat about 23:05 EAT (usage limit reached)

- Integration now wires every lane: assess (625367f), gauntlet routes (cdf1564), the move lane's
  LaunchGate, decisions, held orders and passkey options (386bb46). No pending adapters remain.
  Still to wire: the move lane's `MovesState` (uploads + `POST /sites/{id}/moves`) and the
  mover's `resume_all` at startup (step 8 in its message), once committed.
- Deploy in progress on control: users, folders, `/etc/agentrix/{node,models}.env`, units from
  `deploy/stopgap/`, secrets (`models.env` with provider keys, `gauntlet.token`) are installed.
  A release build of HEAD dfb635e was started in `/opt/agentrix/src`; re-ship current HEAD
  (`git archive HEAD | ssh … tar -x -C /opt/agentrix/src`), `cargo build --release --locked -p
  agentrix-node -p agentrix-models`, copy both binaries to `/opt/agentrix/bin`, then
  `sudo systemctl enable --now agentrix-node agentrix-models` and check https://agentrix.website.
- The shared tree may not compile mid-edit (move lane's pipeline.rs); always check from a clean
  export of HEAD.
