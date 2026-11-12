# The first release: Acceptance checks

Status: v1, Sat 7 Nov 2026. Owner: spec session (task S3-02). Spec chain: requirements →
design → tasks → **acceptance (this file)**.

Every Must criterion has one check. `auto` checks run from `scripts/acceptance.sh` against a
deployment (`BASE`, `FIXTURE`); `manual` checks are done once on a phone and recorded on the
board with the receipt digest or screenshot that proves them. A criterion passes only on
evidence from the live run: a receipt, a test result, or a recorded step. Run it twice:
**Phase 1** (Claude routes) and **Phase 2** (Vultr routes, after switch gate S3-10).

| Id | Check | How | Pass when | Evidence |
|---|---|---|---|---|
| R1.1 | Assess starts without an account | auto: `POST $BASE/assess url=$FIXTURE`, time to first SSE event | 303 to `/assess/{id}`; first `page` or `receipt-tick` event in < 2 s (N2) | SSE log |
| R1.2 | Crawl only in sealed rooms, allowlisted, robots respected | auto: fetch the `job.fetch` receipts | `policy.allowlist` = fixture hosts only; runtime `runsc`; `destroyed: true` | Receipts |
| R1.3 | Receipt ticks live | auto: SSE | `receipt-tick` totals rise monotonically | SSE log |
| R1.4 (S) | Frames | manual | Frames labelled `Live` or `Sealed screenshot, about 1 a second` | Screenshot |
| R1.5 | Report with evidence | auto: `GET $BASE/assess/{id}` | Lists the planted spam links and footer script, each with evidence | Report HTML |
| R1.6 | Honest unreachable | auto: assess `https://unreachable.invalid` | `done` outcome `unreachable`; no findings invented | SSE log |
| R1.7 | One call to action | manual | "Move this site" leads to sign-in and claim | Step |
| R2.1–2.3 | Passkey first, TOTP fallback, safe sessions | manual on a phone + auto cookie check | Passkey register/sign-in works; password+TOTP works; session cookie `__Host-`, HttpOnly, Secure, SameSite | Steps, headers |
| R3.1–3.3 | Claim by TXT or file, from the sandbox, on the fixture | auto + manual | `claim.verified` receipt whose job ran on the sandbox VM; nothing private read before it | Receipt |
| R4.1–4.3 | Sealed replay, inventory, no route out | auto: Move on the fixture | `job.replay` receipt: network internal; inventory digests; attack 8's phone-home `refused` | Receipts |
| R4.4 | Fallback stated | manual (force replay off) | Move screen states the fallback | Screenshot |
| R5.1–5.3, 5.5 | Clean | auto | Attacks 3–7 each have a `quarantined` finding with plain-words reason and evidence; none appears in the candidate release's files | Receipts, release manifest |
| R6.1–6.5 | Rebuild and parity | auto | Candidate at site parity ≥ 0.95; retries ≤ 3 per page in receipts; every step shown with time | Receipts, Move screen |
| R7.1–7.5 | Go live with a passkey; undo | manual on a phone | Card shows parity, left-behind, what runs without asking, old site kept, undo; passkey approval; live in < 60 s (N4); rollback restores | `decision.approved`, `release.signed`, `release.rollback` receipts |
| R8.1 | Only signed releases served | auto: tamper a file in CAS on control, request it | Edge keeps the last good release; tamper logged | Response + log |
| R8.2–8.3 | Ask with citations; honest "don't know" | auto: `POST https://<site>/ask` two questions | Cited answer (title + span); a "don't know" offering the owner | SSE log |
| R8.4 | No discounts, no instructions | auto: gauntlet attack 10 | `concierge.refusal` receipt; console shows it | Receipt |
| R10.1–10.3 | Console slice | manual on a phone | Today, Decisions, Move, Trust render real data; every receipt opens and verifies | Screenshots |
| R11.1–11.4, 11.6 | Gateway | auto + inspection | Only the gateway's environment holds provider keys; every model call has a purpose and a usage row; 429 path tested; a provider switch is config-only and receipted | Usage rows, `ps e` on control, test |
| R12.1–12.5 | Sealed rooms | auto | Every job receipt: `runsc`, read-only root, caps, allowlist, `destroyed: true`; control VM ran no untrusted content | Receipts |
| R12.6 | Egress review | auto: `cargo test -p agentrix-egress` + the nip.io case on control | All pass | Test output |
| R13.1–13.3 | Gauntlet | auto: `agentrix-gauntlet run` | 10/10 from receipts on the deployed URL, in both phases | `gauntlet.run` receipt |
| R14.1–14.3 | Runs on Vultr, reproducibly | auto: `deploy/provision.sh status`, firewall groups | Three VMs; control 80/443 (+22 from ADMIN_CIDR); sandbox only 7443 from control | Status output |
| R15.1–15.2 | Submission | manual | Checklist complete (`submission/checklist.md`) | Checklist |
| N1 | Move < 10 min (50 pages) | auto | From Move start to go-live card | Step timestamps |
| N5 | Cost per Move shown | auto | Meter shows it; under $1 on Vultr routes for 50 pages | Usage meter |
| N6 | Secrets only in the gateway | inspection | No key in repo, logs, receipts, sandbox VM | grep + inspection |
| N8 | Honesty | manual | No faked frames, timings or scores | Review of the video |
| N9 | Resume after a crash | auto: kill node mid-Move, restart | The Move continues from its last completed step | Receipts |
