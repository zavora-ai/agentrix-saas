# Submission checklist (S4-03)

Status: kept by the submission lane. Last updated Sat 7 Nov 2026, 23:15 EAT, at code complete (every Must and Should has code; the live end-to-end run is last, at James's call). Source:
[docs/04-vultr-agent-rush.md §6](../docs/04-vultr-agent-rush.md#6-submission-checklist-lablab-form),
plus R15 and the honesty rule (requirements §2, N8). Submit by **Sun 8 Nov, 20:00 EAT**. The
hard close is **Mon 9 Nov, 02:00 EAT**.

Legend: **ready** (done and checked), **drafted** (written, waiting on live values or James's
review), **waiting** (blocked on another task), **blocker** (must be resolved before submitting).

## The lablab form (04 §6)

| # | Item | State | Where / waiting on |
|---|---|---|---|
| 1 | Project title: **Agentrix: Upgrade your website into WebAgents** (replaces 04 §6's older "Move your website. Meet your WebAgents.") | drafted | [lablab-form.md](lablab-form.md). James approves the title |
| 2 | Short description (one line) and long description | drafted | [lablab-form.md](lablab-form.md). The short one is 174 characters. The long one waits on the gauntlet score and the demo's routes |
| 3 | Technology and category tags | drafted | [lablab-form.md](lablab-form.md) |
| 4 | Cover image, 16:9 | ready | [cover.png](cover.png) (1920 × 1080), drawn from [cover.svg](cover.svg). It states no number |
| 5 | Egress proxy security review passed: DNS rebinding, redirects, IPv6 and encoded addresses, CONNECT, header smuggling | waiting | S2-09 done, verified by spec: 28 cases, 27 pass locally. The DNS-rebinding case needs public DNS (nip.io) and runs on the control VM before deploy |
| 6 | Video (about 4 min, with the containment moment, under 5:00) | drafted | Script v2, [video-script.md](video-script.md), about 4:35. Waiting on S3-10 (Vultr routes) and S3-01 (the gauntlet), then the recording, S3-09 |
| 7 | Slide presentation (PDF) | drafted | 12 slides in [the earlier deck]((an internal board, not public)). Fill its bracketed values, then export to PDF (Share › Export) |
| 8 | Public GitHub repository (MIT) | **blocker** | `LICENSE` is MIT. There is no remote yet. **The Vultr Inference key in commit 80905be's `.env.example` is still in history (05b48af): James rotates it at Vultr before the repo gets a remote** |
| 9 | Demo application platform: Vultr; application URL | ready | https://agentrix.website is live, deployed from a committed tree by `deploy/deploy.sh` (S1-12, S2-10) |
| 10 | README: Vultr services, deployment architecture, how the agent does real work | drafted | [README](../README.md) sections from *How we use Vultr* to *Run it yourself* (S3-07). Each `TODO(live)` waits on the live build |
| 11 | Disclosure: what pre-existed (adk-rust, ideas from Agentrix OS and Agentrix Website) vs. what was built this weekend | drafted | README, *What existed before this weekend vs. what we built*. Waiting on the framework lane to confirm the adk-rust PR list, and on a check of reuse headers |

## Proof the entry rests on (R13, R14, R15)

| # | Item | State | Where / waiting on |
|---|---|---|---|
| 12 | The gauntlet: the score on the live URL, computed from receipts, shown on the Trust tab | waiting | The runner is built, and `deploy.sh` re-runs it on every deploy, printing `n/10` and the `gauntlet.run` receipt (R13.2). Waits on a live site: the demo Move's go-live in James's run |
| 13 | The demo runs on Vultr routes: the switch gate passed, and acceptance and the gauntlet re-run on Vultr | waiting | S3-10, the last step of James's run (runbook on the S3-10 board row). The bake-off is done ([`bakeoff.md`](../specs/first-release/bakeoff.md)): build `qwen3.8-27b`, vision `deepseek-v4.1-flash-1017`, already the Vultr defaults. A routing change signs a `routing.changed` receipt; the first, "Models in use, on anthropic", is live. Acceptance on Phase 1 passes live: 7 pass, 0 fail. The README, deck (slide 8), form and video say the demo runs on Vultr; the switch receipt makes it true, or they must change |
| 14 | Fresh deploy from the README on clean VMs (R14.3) | drafted | The README's *Run it yourself* uses `deploy/deploy.sh <commit>` (lock, `git archive`, a `--locked` release build, rollback, smoke test, gauntlet). Not yet repeated on clean VMs. Object Storage is optional: without `objects.json` it is off and the deploy starts (b7f1147 in node; the unit's fallback and `deploy.sh`'s pre-flight exemption are with control-plane, so recheck both before submitting). The bucket is a manual step; `provision.sh` doesn't create it |
| 15 | Object Storage in use (R14.4, a Should) | ready | Live: bucket `agentrix-b1-14e3def5` on `ams1.vultrobjects.com`; node mirrors every content-store blob and signed receipt each minute, and reads a lost file back by digest (c3c9c0f, bc69a7a). In the README, deck slide 8 and the form. Confirm receipts are in the bucket after the `bc69a7a` deploy |
| 16 | Live frames on the Assess screen (R1.4): the sealed Firefox room | waiting | Deploying; the spec lane confirms. Until then the README and the video don't call frames live |
| 17 | Agent race, or its fallback (R9.3) | waiting | The harness is built (`scripts/agent-race/`, Claude Sonnet 5.5 as both customer agents). A first run on the old site took 22 steps and 62 s, cost $0.18, and gave up when the order form didn't confirm. The real race runs after go-live. The video script and deck slide 6 have both versions |
| 18 | The demo Move: rendered parity, go-live on a passkey | waiting | James's live run, after the HEAD deploy (status.md): decline the first Move's card, start a new Move, check Prove does not say "(text only)", approve with a passkey. The first live Move (50 pages, Claude routes) gave Replay 12.4 s, Clean 37.5 s, Rebuild 26.6 s and about $0.22 in model calls; its renders were refused (107 inputs, cap 64, fixed in 1422355), so its parity is never quoted |

## Consistency pass, the last step before submitting

- [ ] Every placeholder is filled from receipts or the usage meter: `TODO(live)` in the
      README and the form, `[…]` in the deck, `[[…]]` in the video script. One way to check:

      grep -n "TODO" README.md submission/*.md

- [ ] The same values everywhere: the gauntlet score, the parity, seconds to live, the cost per
      Move, the build model chosen by the bake-off.
- [ ] The WordPress share is cited as W3Techs and re-checked on the day (39.4% on 7 Nov 2026).
      Docs 01 and 04 were corrected in 1fa65c7.
- [ ] No claim of a microVM: The first release's replay runs under gVisor (S2-05). Docs 04 were corrected
      in 1fa65c7, and the README's Challenge 1 table in the submission lane's follow-up commit,
      at the spec lane's request.
- [ ] The model names match the gateway's routes: build `qwen3.8-27b` (raised to `glm-5.3`),
      vision-parity `deepseek-v4.1-flash-1017`, the bake-off's picks, and the `routing.changed`
      receipt from the switch names them.
- [ ] Cost figures are labelled: $0.22 is the first live Move on Claude (measured); $0.06 is the
      bake-off's estimate for build and vision on Vultr; `[[cost per Move]]` is the demo Move.
- [ ] The competitor counts are re-checked against lablab on submission day. They are from the
      7 Nov 10:10 EAT snapshot.
- [ ] No secret anywhere: `git grep -n -i -E "api[_-]?key\s*=\s*\S{8,}"` finds nothing, and the
      old key is rotated (item 8).
- [ ] James has reviewed the README (S3-07), and the slides and script (S3-08).

## Submitted

- [ ] Form submitted by James (S4-04), at ____ EAT.
