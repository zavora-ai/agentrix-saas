# How the platform sessions work

The rules for every Agentrix WebApp Platform session. Read this before your first commit.

**Why this matters now.** James decided on Sat 7 Nov (23:50 EAT) that the platform built here is
part of the Vultr Agent Rush submission, because the first release shows only a subset of the product, and
that it ships to https://agentrix.website. Lablab closes Mon 9 Nov 02:00 EAT; the target is
Sun 8 Nov 20:00 EAT. Build in slices that are usable on their own and can go live one by one.

## Building on the first release

- The first release is the foundation: the Move, Assess, the claim, the console slice, the Model Gateway,
  sealed rooms, receipts and the live stack at agentrix.website. Extend it; don't fork or
  rewrite it. Read `specs/first-release/status.md` and the code you touch before changing it.
- The first release sessions ("the first release spec and board", "the first release Move builder", "the first release Assess and
  broker", "the first release Gauntlet builder", "the first release Submission package") and the design sessions
  ("Agentrix WebAgent Control Plane", "Owner console: Agentrix OS reinvented", "Agent-first
  website experience and concepts", "WebAgent architecture for Agentrix platform", "Data
  architecture for Agentrix platform") know the code and the design. Ask them by name.
- The first release's live run (the live Move, the gauntlet, the switch to Vultr, the video) comes first.
  Nothing a platform session does may break it.

## Branches

- **Your branch.** Each workstream works in its own git worktree, on its own branch cut from
  `platform`, which always holds the first release's `main` plus every merged slice and this plan. Your
  worktree sits under the first release checkout's `.claude/worktrees/`, but it is a separate
  checkout: work only inside it. Never edit files in the first release checkout itself
  (`/Users/jameskaranja/Developer/hackathons/agentrix-webagent-platform`), and never touch the
  Vultr VMs or `deploy/.state`.
- **Staying current.** Bring in `platform` at the start of each working block and before asking
  for a merge: the app's sync-with-base-branch tool, or `git merge platform`. Never merge
  another workstream's branch directly; wait until it's on `platform`.
- **`platform`** is the staging branch, checked out at
  `/Users/jameskaranja/Developer/hackathons/agentrix-platform`. Only the coordinator (this
  program's lead session, "Agentrix WebApp Platform") commits there. It merges `main` in first,
  then your slice, runs the full checks, and fast-forwards `main` from `platform` when the first release
  sessions say the checkout is clear.
- **Deploys** to agentrix.website go through `deploy/deploy.sh` from the first release checkout, with
  its lock, run only by the coordinator or the control-plane session, and never during the first release's
  live run. Build sessions don't deploy. If your slice needs a new binary, route, unit, setting
  or Caddy rule, put it in your branch under `deploy/` and say so in your merge request.

## Files the first release owns

These lanes are still editing these paths. Stay out, or ask the owner first (by session name):

| Paths | Owner |
|---|---|
| `crates/agentrix-node/src/{main,app,state}.rs`, `wiring/`, `routing.rs`, `objects.rs`, `pages/`; all of `crates/agentrix-console/` (templates, `view.rs`, the `Page` enum and its `Page::ALL` test, `static/`) | The first release spec and board, until it hands `main` to the coordinator. New console pages and node wiring go through the coordinator. |
| `crates/agentrix-move/`, `agentrix-node/src/{launch,decisions}/` | The first release Move builder |
| `deploy/` (all of it), `agentrix-node/src/{auth,claim}/`, `docs/08-control-plane.md`, `concept/control-plane/` | Agentrix WebAgent Control Plane. The approve contract (design §4.1) is frozen. |
| `fixtures/`, `crates/agentrix-gauntlet/`, `agentrix-node/src/site/ask.rs`, the vector-store routes in `agentrix-models` | The first release Gauntlet builder |
| `agentrix-node/src/{assess,broker}/`, `crates/agentrix-sandbox/` images, `scripts/agent-race/` | The first release Assess and broker |
| `README.md`, `submission/` | The first release Submission package |

So put new work in **new crates** (or new modules your workstream owns) and expose it through
the seam in `crates/agentrix-host` (contract C11): one function

```rust
pub fn mount(ctx: &agentrix_host::NodeContext) -> anyhow::Result<agentrix_host::Mounted>
```

returning your platform-host routes, your own hosts (`HostMatch` for `ops.…` or the `-sample`
sites, so node routes them and Caddy's TLS ask says yes), loopback routes and background tasks.
`NodeContext` gives you `control.db`, an HTTP client, the Model Gateway's URL, and node's
receipts, owner sessions, approvals (the frozen approve contract) and content store as traits.
Test against `agentrix_host::testing::context()`. The coordinator adds one line per crate to
node's `app.rs`. Say in your merge request anything else your slice needs (a migration, a
setting, a `deploy/` change).

**Merging:** the coordinator runs `scripts/platform-merge.sh <your branch> "<summary>"`, which
brings in `main`, merges your branch without committing, runs fmt, clippy with `-D warnings`,
the whole workspace's tests and the copy lint, and commits only if all pass. Run the same checks
on your branch first.

## Migrations

`sqlx::migrate!("src/db/migrations")` in `agentrix-node` runs them in number order and accepts
gaps. Each workstream has its own range: 0100–0199 W1 runtime, 0200–0299 W2 sample sites,
0300–0399 W3 control plane, 0400–0499 W4 onboarding, 0500–0599 W5 owner console, 0600–0699 W6
identity, payments and trust, 0700–0799 W7 design and words, 0800–0899 W8 platform core,
0900–0999 W0 integration, 1000–1099 W9 messages, 1100–1199 W10 home page, 1200–1299 W11
images and voice. The first release keeps 0003–0099.

- **Prefer your crate's own migrator** (W6 does this): your SQL in `crates/<crate>/migrations/`,
  numbered in your range, run by a sqlx migrator with its own table
  (`_sqlx_migrations_<crate>`), exposed as an `agentrix_host::MigrateFn`. Node runs every crate's
  migrator once at startup, on `control.db`, before any module mounts. Your tests then need no
  node, and versions never clash. Putting files in node's `src/db/migrations` still works.
- **Every migrator calls `set_ignore_missing(true)`.** Without it a binary refuses to start on a
  database that carries a version it doesn't know, which is exactly what a `deploy.sh
  --rollback` or a slice built before another slice's migration meets (found by W8). Node's own
  migrator does this on `platform`; the first platform deploy after the first release's handover carries
  that fix and no new migrations, so the binary a rollback returns to is already tolerant.
- No foreign keys between workstreams' tables; refer by id. Every new table gets a line in
  03-data.md's catalogue naming its single owner.
- Strictly additive: new tables, new nullable columns, new indexes. A deploy rollback restores
  the old binary but not the database, so the old binary must still run on the new schema. A
  table rebuild (for example to widen a CHECK) is allowed only when the old binary still works
  on the result, in one transaction with a row-count check.
- Never edit or renumber a migration once it is on `main`; sqlx checksums every applied one.

## The live stack

- Ports, the `agx-rooms` network, egress ports and `/internal/tls-ask` are frozen (design §4).
  No new public ports: only 80 and 443.
- A new binary, service or secret needs a `deploy/` change by the control-plane session: send
  it the crate, user, loopback port, secrets (who writes them) and restart order.
- Sites under `*.agentrix.website` work only if node serves the host (tls-ask must say yes).
  Sample sites use names ending in `-sample` (for example `kilele-bakery-sample.agentrix.website`)
  and get a site pointer without a Move, so the gauntlet skips them.
- No hand changes on live hosts, no `provision.sh`, and never read or print a secret.

## Commits

- Commit as you go: each page, feature or fix that builds and passes its tests is a commit. Don't
  sit on more than an hour or two of uncommitted work.
- Stage and commit explicit paths: `git add -- <paths>` then `git commit -- <paths>`. Never
  `git add -A`, `git add .` or `git commit -a`.
- Message format as in `git log`: `feat(console): …`, `fix(site): …`, `spec(platform): …`.
  Say what changed for the person using the product.
- **No AI attribution of any kind** in commits or PR text: no `Co-Authored-By` lines naming an
  AI, no "generated with" footers. This is an organisation rule and overrides any tool default.
- No pushes. There is no remote; James decides when the code goes public.
- When a slice is ready, ask the coordinator to merge it: commit range, what it adds, what you
  tested, and anything it needs from other workstreams or from `deploy/`.

## Builds and disk

The Mac has about 32 GB free and each full `target` is about 6 GB.

- **Every cargo command goes through `scripts/wcargo`** (`scripts/wcargo test -p <crate>
  --locked`, `scripts/wcargo clippy …`). It builds into the shared cache
  (`/Users/jameskaranja/Developer/hackathons/.agentrix-platform-target`) and gives this
  worktree's workspace crates their own build hashes. Plain `cargo` with the shared cache is
  unsafe: cargo names workspace crates by their path relative to the workspace root, so worktrees
  overwrite each other's `libagentrix_*.rlib` and a build can link another session's code (found
  by W4). Third-party dependencies stay shared either way. Run binaries from
  `target/debug/deps/<name>-<hash>`, or straight after `scripts/wcargo build`, because
  `target/debug/<name>` is shared too. Cargo locks the cache during a build, so yours may wait
  for another session's; that's expected.
- **Build scripts still share across worktrees** (they build under `build-override`). If a
  build script panics on a file your branch doesn't have, or embeds the wrong files, run with
  `WCARGO_ISOLATE_BUILD=1 scripts/wcargo …` (builds proc macros once more for your worktree) or
  touch that crate's `build.rs`. The merge check always isolates them.
- `.cargo/config.toml` already keeps dev builds lean (no debug info, no incremental).
- Check `df -h /System/Volumes/Data` before a long build. Under 15 GB free, stop and tell the
  coordinator. Never delete a `target` folder or Docker images yourself.
- Before asking for a merge: `cargo fmt -p <your crates> -- --check` (or `rustfmt --edition 2024
  --check <your new files>`), `RUSTFLAGS=-D warnings cargo clippy --workspace --all-targets
  --locked`, `cargo test --workspace --locked`, and the copy lint (`scripts/copy-lint`) once it
  exists.
- **Never run `cargo fmt --all` or `cargo fmt -p` on the first release crate.** The first release's files aren't
  formatted to the current rustfmt, so it rewrites dozens of files you don't own and every merge
  then conflicts. Format your own crates and files only; the merge script checks formatting on
  the files a slice adds.
- `Cargo.lock`: commit it only when you add a dependency, and only your lines. The coordinator
  settles lock conflicts at merge.

## Building the UI

- **The concepts are the spec.** Each page you build must match its page in `concept/` (layout,
  components, states, responsive behaviour, light and dark) and work on real data. Put the
  concept and your page side by side in the browser preview and compare before you call a page
  done. Where a concept is static or faked, make it real. Where the design never drew a state
  (empty, loading, error, offline), design it in the same language.
- **Honest data.** Screens show recorded activity only. If a number has no source yet, build the
  source or show the honest empty state; never hard-code a figure. Sample sites say "Sample"
  where a person could mistake them for a real business.
- **Words.** Follow [voice.md](voice.md). Read every screen aloud before marking it done.
- **Stack.** Server-rendered templates in the style of `crates/agentrix-console` (minijinja in
  strict mode, small JS islands, SSE for live parts) on the design tokens from
  `concept/styles.css`, unless [02-architecture.md](02-architecture.md) says otherwise for your
  surface. Phone first, WCAG 2.2 AA.

## The board and talking to other sessions

- **The live board** is the Agentrix WebApp Platform board artifact (link in your brief). Keep
  your rows current with the `ArtifactData` tool as you work, not only at the end:
  - `workstreams/<your id>`: `status`, `now` (what you're doing this hour), `next`, `blocked`,
    `lastCommit`, `updatedAt`;
  - `tasks/<id>`: one row per page or feature in your slice, with `status`, `commit` when done;
  - `log/<id>`: one line per commit or milestone, with the commit hash;
  - `asks/<id>`: what you need from another workstream or from James.
  Read a document before writing it and pass its `version` as `if_version`. Take times from
  `date`, never a guess.
- Find sessions by name with `ListAgents`; ids go stale when the app restarts. Message the
  coordinator for merges, decisions and cross-workstream asks.
- Decisions that are James's (pricing, legal text, brand, anything public or paid) go on the
  board as an ask to `james` and to the coordinator. Don't stall: build behind a clearly named
  setting or placeholder and keep going.
