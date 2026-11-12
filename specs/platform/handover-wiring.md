# Wiring that waits for the first release's handover

Changes to files the first release sessions still own (`how-we-work.md`, "Files the first release owns"). W0 applies
them on `platform` once the first release spec hands over `main`, in this order, each with the full check
(`scripts/platform-merge.sh --check`) and a local node run, and with the owner's agreement where
the owner is back. Kept current by W0.

**§1 to §3 are applied** on branch `w0/w15-node-wiring` (W0, Sun 8 Nov 2026, 19:57 EAT), one
commit per item or group, each with tests; the coordinator merges and deploys. Workspace fmt,
clippy (`-D warnings`) and tests pass on that branch. The "Applied" column names the commit.

## 0. Before anything else on main

| # | Change | From | Files | Why first |
|---|---|---|---|---|
| 0.1 | The content-safety screen reads the verdict after the model's reasoning (`e73d93b`) | W11 | `crates/agentrix-models/src/gateway.rs` `parse_verdict`, one test | On Vultr the screen otherwise blocks everything or lets an injection through. Needed before window B (the switch to Vultr). James decides (board ask d10); it may go to `main` ahead of the handover |
| 0.2 | Node's migrator ignores versions it doesn't know (`575aae9`) | W8, W0 | `crates/agentrix-node/src/db/mod.rs` | The first platform deploy carries this and no new migrations, so a rollback binary still starts on a newer database |

## 0b. Deploy order (W0, with the control-plane session or spec)

1. **Deploy A:** branch `platform-deploy-a` (36cff41): `main` as of 976b0e7 plus only the
   tolerant migrator in `db/mod.rs` and its test (no new migrations, no new routes). `main` can't
   fast-forward to `575aae9` because the first release kept committing, so this branch is rebuilt on the
   latest `main` if it moves again. The gauntlet must stay 10/10. After this, the binary a
   rollback returns to starts on a database with newer versions.
2. **Deploy B:** `main` fast-forwarded to the checked `platform` head. New tables arrive (0100,
   0300, 0500, 0600, 0800 to 0802, 1000), all additive except 0801's checked `usage` rebuild. The
   home page is at `/home`, the operator console on `ops.<host>`, the owner console at `/os`.
   Smoke: `/healthz`, `/status`, `/home`, `/os` signed in, `ops.<host>` sign-in page, a sample host
   once W2 has merged.
3. **Then** the §1 to §3 wiring below in one or two deploys, each re-running the gauntlet.

## 1. Front door and onboarding

| # | Change | From | Files | Applied |
|---|---|---|---|---|
| 1.1 | Assess renders W4's pages; `GET /assess` beside `POST` | W4 | `specs/platform/patches/w4-assess-wiring.patch` (assess/, claim/, app.rs) | `d25b510` on `platform` (ported by hand); test pinning W4's Start at `GET /assess` and on a refused address: `351360a` |
| 1.2 | The home page becomes `/`: drop `.route("/", get(start_page))` from `assess::router`, call `agentrix_home::mount_with_front_door` instead of `mount` | W10 | `crates/agentrix-node/src/assess/`, `wiring/host.rs` | `d25b510` on `platform` |
| 1.3 | Pictures for bare pages at go-live: right after `go_live` moves the pointer (beside W3's `Billing::went_live`, same moment), spawn in the background `agentrix_media::fill::fill(&agentrix_media::routes::MediaState::new(&ctx), site, "", &pages, 5)` with `pages` from the new release (`fill::Page { path, title, summary, has_picture }`). At most 5 drafts a site per go-live (about $0.10, inside the site's budget and W12's daily caps); go-live never waits on it and a page view never starts it. W4's "you're live" card then shows "Pictures for pages that had none wait for you in Pictures" when `agentrix_media::library::waiting(&pool, site, Some("move"))` is above 0, linking `library::pictures_url(site)` | W11, W4 | `crates/agentrix-node/src/launch/` (go_live) | `96fbc43`: `launch::after_live::AfterLive`, spawned once the first probe passes, with the release's pages (title and summary from each Markdown twin); `NodeAfterLive` runs billing, then the fill. The card's line is `MovePage.pictures_href`, set by `pages/` from `ConsoleReads::pictures_waiting`. Node now depends on `agentrix-billing` |

## 2. Living marks that move

| # | Change | From | Files | Applied |
|---|---|---|---|---|
| 2.1 | `presence::global().on_move(site, &event)` in `MoveEvents::emit` | W1 | `crates/agentrix-node/src/launch/ports.rs` | `642088f` |
| 2.2 | `on_ask(site, "concierge", event, data)` in Ask's send closure and refusal loop | W1 | `crates/agentrix-node/src/site/mod.rs` | `642088f`; the resting replies added later (2a.3, 2b.1, 3.2) call it too |
| 2.3 | `on_decision(...)` when a held order's card is inserted, approved or declined | W1 | `crates/agentrix-node/src/decisions/mod.rs` | `642088f`; approve, decline and expiry call it for every card kind, so a go-live or restore card lets its raiser rest too |
| 2.4 | `on_brief(site, true/false)` around a brief refresh that writes | W1 | `crates/agentrix-node/src/brief/mod.rs` | `642088f` |

## 2a. Dials that bite (W1)

In node: `let rosters = agentrix_kernel::Rosters::new(pool, blobs)`.

| # | Change | Files | Applied |
|---|---|---|---|
| 2a.1 | Held order, before raising the card: `rosters.check(site_id, &Action::new("agent-desk", "held-orders", Effect::Financial))`; on Refuse the door says the shop isn't taking orders through agents right now | `crates/agentrix-node/src/decisions/mod.rs` | `2ac798d`, in `door/mod.rs` before `HeldCalls::raise` (the door words the refusal: "This shop isn't taking orders through agents right now. Nothing was sent.") |
| 2a.2 | Before writing a new brief: `Action::new("coordinator", "morning-brief", Effect::Draft)`; on Refuse (Watch) write nothing and call no model | `crates/agentrix-node/src/brief/mod.rs` | `2ac798d` |
| 2a.3 | Ask: `Action::new("concierge", "presentations", Effect::Read)`; on Refuse (off duty) send the resting frame | `crates/agentrix-node/src/site/mod.rs` | `2ac798d` |

The checks live in `crates/agentrix-node/src/guards.rs` (`Guards::for_node`, handed to the site
state, the brief, decisions and the Move routes). A dial check that fails to read is logged and
the path runs as before.

## 2b. Freeze that stops what it says

| # | Change | From | Files | Applied |
|---|---|---|---|---|
| 2b.1 | `agentrix_shell::freeze_gate(&ctx).is_frozen(site)` checked in the brief timer, Ask, the agent door and the Move (skip or refuse while frozen); until then the console hides the Freeze pill | W5 | `brief/`, `site/`, `door/`, `launch/` | `4abb00a`: the brief writes nothing; Ask rests; the door's `tools/call` is refused (discovery and held-call status still answer); a Move doesn't start, resume or take accepted pages, and a go-live approval is refused before it is recorded (in `decisions/`, so the card keeps waiting). Undo stays available. A Move already running in its sealed rooms finishes them, since they change nothing visitors see; stopping it mid-step needs a pause port in `agentrix-move`. The console offers Freeze once node runs with `AGENTRIX_FREEZE_ENFORCED=1` (4.10) |
| 2b.2 | `/console` points at the new shell (`/os`) | W5 | `pages/` | `0698aa7` on `platform` |

## 3. Sites and the edge

| # | Change | From | Files | Applied |
|---|---|---|---|---|
| 3.1 | The visit beacon (`agentrix_store::visits::beacon_routes`) in the site router, and `visits::BEACON_SCRIPT` in releases | W8 | `app.rs` site router; the release builder (`agentrix-move`, with the Move builder) | `02c98c3`: the beacon finds the site by its live release (`wiring::ServedSites`, own domain or platform address). The crate's API is `visits::script_tag(seq)`, carried by the Move's pages with the samples' footer line and the visitor's "Don't count my visits" switch (`agentrix-console/templates/site/page.html`). Only releases built from now on count; a live release is unchanged until its next Move. `agentrix-move` now depends on `agentrix-store` |
| 3.2 | Platform holds enforced per request (`agentrix_operator::holds::active`) | W3 | `crates/agentrix-node/src/site/` | `8bc8771`: a middleware on the site router serves only "This site isn't available right now." under a `notice` hold; under `read-only` the pages serve and Ask, passed questions and the door's tools pause ("Answers and forms are paused right now. You can still read the site."). A hold that can't be read is logged and the site serves as usual |

## 4. Deploy changes (the control-plane session)

| # | Change | From |
|---|---|---|
| 4.1 | `/etc/agentrix/operators.txt` (operator roles) and `AGENTRIX_OPERATORS_FILE` | W3 |
| 4.2 | Model caps in `deploy/env/models.env`: `AGENTRIX_FLEET_DAILY_CAP_USD` (default 8), `AGENTRIX_SAMPLES_DAILY_CAP_USD` (default 3) | W8, W12 |
| 4.3 | `provision.sh` costs, snapshot, sleep, wake and restore-test (dry runs) | W12 |
| 4.4 | PayPal sandbox credentials as a node credential | W6 |
| 4.5 | A backup for `control.db` | W8, W12 |
| 4.6 | Email on: `RESEND_API_KEY` (send-only, from James) in node's environment file, optional `RESEND_WEBHOOK_SECRET` (mounts `POST /messages/email/webhook`), `AGENTRIX_EMAIL_FROM` defaults to `hello@agentrix.website`. First real send only on James's word | W9 |
| 4.7 | Telegram: `AGENTRIX_TELEGRAM_FILE` naming a 0600 JSON file (`{"bots":[{"site":…,"token":…}]}`) for the Agentrix bot, @agentrix_websites_bot. One process polls a bot: only the live node, never a local one at the same time | W13 |
| 4.8 | The home page: `AGENTRIX_HOME_WHATSAPP=+254721490933`, `AGENTRIX_HOME_WHATSAPP_HOURS="6am to 11pm EAT"` (James, 8 Nov), and `AGENTRIX_HOME_FILM_DIR` holding hero, film, classy, tana and salon `.mp4` (about 39 MB, rendered by `crates/agentrix-home/film/render.py`) | W10 |
| 4.9 | `AGENTRIX_MODELS_URL` set for node (the gateway on loopback); the gateway rebuilt with W8's `x-agentrix-webagent` and `x-agentrix-job` usage columns | W8, W1 |
| 4.10 | `AGENTRIX_FREEZE_ENFORCED=1` in node's environment once 2b.1 is deployed, so the console and gestures offer Freeze | W5, W0 |

## 4a. Fixes in the first release files (found on the platform walk, Sun 8 Nov)

| # | Fix | Found by | Files |
|---|---|---|---|
| 4a.1 | Passkey registration attaches the new passkey to whoever is signed in, whatever email was typed. James hit it: his passkey landed on a test account. Refuse when the typed email isn't the signed-in account's, and have the first release's `/auth/register` page say who is signed in, as W4's pages now do (81b7a97) | W0, W4 | `crates/agentrix-node/src/auth/routes.rs` (passkey register start), the `/auth/register` template |
| 4a.2 | The claim's error message ends in a doubled full stop because it joins `{e}.` onto an error that already ends in one. W4 tidies it on its page; fix it at the source | W4 | `crates/agentrix-node/src/claim/mod.rs` |

## 5. Reviews owed by sessions that were stopped

| What | Reviewer |
|---|---|
| W3's operator sign-in through the public approvals API with a second sign-in context | Agentrix WebAgent Control Plane |
| `OwnerAccounts` (one re-export in `auth/mod.rs`) for W6's email and recovery codes | Agentrix WebAgent Control Plane |
| Receipt kinds and `Subject::Account` added to `agentrix-types` | The first release spec, Data architecture |
| `collapsible_if` allowed workspace-wide | The first release spec |
