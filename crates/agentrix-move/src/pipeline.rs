//! The Mover: runs a Move step by step, persisting each step before the next (N9).
//!
//! Each step writes a `running` row, does its work, stores what it produced in the CAS, issues
//! a `move.step` receipt, writes a `done` row with the receipt and its resume data, and only
//! then moves `moves.state` on. A crash at any point resumes at the first step without a
//! `done` row; a finished step is never run twice. The Mover stops at the go-live card: it
//! never goes live by itself (R7).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use agentrix_types::ui::{
    CardEvent, DoneEvent, DoneOutcome, ErrorEvent, PageEvent, PageStatus, ParityEvent, ParityState,
    QuarantinedEvent, StepEvent, StepName, StepState,
};
use agentrix_types::{
    CardKind, Digest, JobKind, MoveId, QuarantineEntry, ReceiptBody, ReceiptKind, SiteId, Subject,
    UiEvent,
};
use anyhow::Context as _;
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use futures::{FutureExt as _, StreamExt as _};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::candidate::{self, Candidate};
use crate::clean::{self, CleanContext, CleanSite, LeftBehind};
use crate::golive::{GoLiveFacts, charter_digest, charter_v1, go_live_card};
use crate::inventory::{self, MoveInputs, OldSite};
use crate::parity::{self, PageParity, ParityPolicy, ParityReport};
use crate::ports::{Blobs, CallContext, Events, Gateway, Receipts, RoomJob, Rooms, VisionRequest};
use crate::rebuild::{self, BuiltBody, MAX_ATTEMPTS};
use crate::state::MoveState;
use crate::store::{MoveRecord, MoveStore, QuarantineState};
use crate::theme::{self, PageView, SiteView, Tokens};

/// The rebuilt pages and the look they were built with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuiltSite {
    /// One body per clean page.
    pub bodies: Vec<BuiltBody>,
    /// The site's look.
    pub tokens: Tokens,
    /// What happened to the old theme's build hook, in plain words, if there was one.
    #[serde(default)]
    pub legacy_theme: Option<String>,
}

/// Moves running in this process.
static RUNNING: std::sync::OnceLock<std::sync::Mutex<BTreeSet<String>>> =
    std::sync::OnceLock::new();

/// Marks a Move as running until dropped.
struct RunGuard(String);

impl RunGuard {
    fn take(move_id: &MoveId) -> Option<Self> {
        let set = RUNNING.get_or_init(Default::default);
        let mut set = set
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set.insert(move_id.to_string())
            .then(|| Self(move_id.to_string()))
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if let Some(set) = RUNNING.get() {
            set.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&self.0);
        }
    }
}

/// Whether a Move is running in this process.
pub fn is_running(move_id: &MoveId) -> bool {
    RUNNING.get().is_some_and(|set| {
        set.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(move_id.as_str())
    })
}

/// The runner's limit on one job's inputs (agentrix-runner `MAX_INPUTS`, raised to 512 so
/// one room carries a site's every asset); batching stays as the guard.
pub const MAX_ROOM_INPUTS: usize = 512;

/// Pages built at once (the gateway meters and budgets each call).
pub const BUILD_CONCURRENCY: usize = 6;

/// What the Prove step leaves for go-live.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proved {
    /// The parity report.
    pub parity: ParityReport,
    /// The candidate, once parity allows one.
    pub candidate: Option<Candidate>,
}

/// What a finished step reports.
struct StepDone {
    summary: String,
    resume: Value,
    inputs: Vec<Digest>,
    outputs: Vec<Digest>,
    detail: Value,
    /// The step ends waiting for the owner rather than done.
    waiting: bool,
    /// The state the Move takes; `None` keeps the default for the step.
    state: Option<MoveState>,
}

/// The Move's engine and its ports.
#[derive(Clone)]
pub struct Mover {
    /// The move lane's tables.
    pub store: MoveStore,
    /// The CAS.
    pub blobs: Arc<dyn Blobs>,
    /// The receipts store.
    pub receipts: Arc<dyn Receipts>,
    /// The Move's live stream.
    pub events: Arc<dyn Events>,
    /// Sealed rooms.
    pub rooms: Arc<dyn Rooms>,
    /// The Model Gateway.
    pub gateway: Arc<dyn Gateway>,
    /// Parity bars.
    pub policy: ParityPolicy,
}

fn plain(e: &anyhow::Error) -> String {
    let text = e.to_string();
    let first = text.lines().next().unwrap_or("Something went wrong").trim();
    first.chars().take(200).collect()
}

impl Mover {
    async fn emit(&self, site: &SiteId, move_id: &MoveId, event: UiEvent) {
        if let Err(e) = self.events.emit(site, move_id, event).await {
            tracing::warn!(%move_id, error = %e, "could not emit a Move event");
        }
    }

    async fn put_json<T: Serialize>(&self, site: &SiteId, value: &T) -> anyhow::Result<Digest> {
        self.blobs
            .put(
                site,
                "move-state",
                "application/json",
                &serde_json::to_vec(value)?,
            )
            .await
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        site: &SiteId,
        digest: &Digest,
    ) -> anyhow::Result<T> {
        let bytes = self
            .blobs
            .get(site, digest)
            .await?
            .with_context(|| format!("the Move's saved state {digest} is missing"))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn resumed<T: DeserializeOwned>(
        &self,
        mv: &MoveRecord,
        step: StepName,
        key: &str,
    ) -> anyhow::Result<T> {
        let resume = self
            .store
            .resume_of(&mv.id, step)
            .await?
            .with_context(|| format!("the {key} of an earlier step is missing"))?;
        let digest: Digest = resume
            .get(key)
            .and_then(Value::as_str)
            .with_context(|| format!("the {key} of an earlier step is missing"))?
            .parse()?;
        self.get_json(&mv.site, &digest).await
    }

    fn call(&self, mv: &MoveRecord) -> CallContext {
        CallContext {
            site: mv.site.clone(),
            move_id: mv.id.clone(),
            attempt: 1,
        }
    }

    /// Starts a Move for a claimed site. Assess and Claim are recorded as done with their
    /// receipts; the inputs are stored for the Replay step.
    pub async fn start(
        &self,
        site: &SiteId,
        inputs: &MoveInputs,
        assessment_receipt: Option<Digest>,
        claim_receipt: Option<Digest>,
    ) -> anyhow::Result<MoveId> {
        let now = Utc::now();
        let move_id = self
            .store
            .create_move(site, MoveState::Claimed, now)
            .await?;
        let inputs_digest = self.put_json(site, inputs).await?;
        for (step, summary, receipt, resume) in [
            (
                StepName::Assess,
                "Your site was assessed",
                assessment_receipt,
                Value::Null,
            ),
            (
                StepName::Claim,
                "You proved you own the site",
                claim_receipt,
                json!({ "inputs": inputs_digest }),
            ),
        ] {
            let resume = (!resume.is_null()).then_some(resume);
            self.store
                .record_step(
                    &move_id,
                    site,
                    step,
                    StepState::Done,
                    summary,
                    receipt.as_ref(),
                    resume.as_ref(),
                    now,
                    Some(now),
                )
                .await?;
            let mut event = StepEvent::new(step, StepState::Done, 0, summary, now);
            event.receipt = receipt;
            self.emit(site, &move_id, UiEvent::Step(event)).await;
        }
        Ok(move_id)
    }

    /// Runs the Move from wherever it stands until it waits for the owner, ends, or a step
    /// fails. Safe to call again after a crash or a failure: finished steps are not re-run.
    pub async fn run(&self, move_id: &MoveId) -> anyhow::Result<MoveState> {
        // One run per Move in this process: a second call (a restart's resume, the owner
        // pressing Start again) returns the Move's state and leaves the running one alone.
        let Some(_running) = RunGuard::take(move_id) else {
            let mv = self
                .store
                .move_record(move_id)
                .await?
                .with_context(|| format!("no Move {move_id}"))?;
            return Ok(mv.state);
        };
        loop {
            let mv = self
                .store
                .move_record(move_id)
                .await?
                .with_context(|| format!("no Move {move_id}"))?;
            let Some(step) = mv.state.next_step() else {
                return Ok(mv.state);
            };
            if step == StepName::Prove
                && let Some(last) = self
                    .store
                    .steps(move_id)
                    .await?
                    .into_iter()
                    .rev()
                    .find(|s| s.step == StepName::Prove)
                && last.state == StepState::Waiting
            {
                // Flagged pages wait for the owner (R6.4).
                return Ok(mv.state);
            }
            self.run_step(&mv, step).await?;
        }
    }

    async fn run_step(&self, mv: &MoveRecord, step: StepName) -> anyhow::Result<()> {
        let started = Utc::now();
        self.store
            .record_step(
                &mv.id,
                &mv.site,
                step,
                StepState::Running,
                "",
                None,
                None,
                started,
                None,
            )
            .await?;
        self.emit(
            &mv.site,
            &mv.id,
            UiEvent::Step(StepEvent::new(
                step,
                StepState::Running,
                0,
                "Working",
                started,
            )),
        )
        .await;
        let result = match step {
            StepName::Replay => self.replay(mv).await,
            StepName::Clean => self.clean(mv).await,
            StepName::Rebuild => self.rebuild(mv).await,
            StepName::Prove => self.prove(mv).await,
            StepName::GoLive => self.raise_go_live(mv).await,
            StepName::Assess | StepName::Claim => {
                Err(anyhow::anyhow!("that step happens before the Move"))
            }
        };
        let ended = Utc::now();
        let elapsed = u64::try_from((ended - started).num_milliseconds()).unwrap_or(0);
        match result {
            Ok(done) => {
                let mut body = ReceiptBody::new(
                    ReceiptKind::MoveStep,
                    mv.site.clone(),
                    Subject::Move(mv.id.clone()),
                    started,
                    ended,
                );
                body.inputs = done.inputs.clone();
                body.outputs = done.outputs.clone();
                let mut detail = match done.detail {
                    Value::Object(map) => map,
                    _ => serde_json::Map::new(),
                };
                detail.insert("step".into(), json!(crate::state::step_str(step)));
                detail.insert("summary".into(), json!(done.summary));
                body.detail = Some(Value::Object(detail));
                let receipt = self
                    .receipts
                    .issue(body.with_title(done.summary.clone()))
                    .await?;
                let state = if done.waiting {
                    StepState::Waiting
                } else {
                    StepState::Done
                };
                self.store
                    .record_step(
                        &mv.id,
                        &mv.site,
                        step,
                        state,
                        &done.summary,
                        Some(&receipt),
                        Some(&done.resume),
                        started,
                        Some(ended),
                    )
                    .await?;
                let next = done.state.unwrap_or_else(|| MoveState::after(step));
                self.store.set_state(&mv.id, next, ended).await?;
                let mut event = StepEvent::new(step, state, elapsed, &done.summary, ended);
                event.receipt = Some(receipt);
                self.emit(&mv.site, &mv.id, UiEvent::Step(event)).await;
                Ok(())
            }
            Err(e) => {
                let message = plain(&e);
                tracing::warn!(move_id = %mv.id, ?step, error = %format!("{e:#}"), "Move step failed");
                self.store
                    .record_step(
                        &mv.id,
                        &mv.site,
                        step,
                        StepState::Failed,
                        &message,
                        None,
                        None,
                        started,
                        Some(ended),
                    )
                    .await?;
                self.emit(
                    &mv.site,
                    &mv.id,
                    UiEvent::Step(StepEvent::new(
                        step,
                        StepState::Failed,
                        elapsed,
                        &message,
                        ended,
                    )),
                )
                .await;
                self.emit(
                    &mv.site,
                    &mv.id,
                    UiEvent::Error(ErrorEvent {
                        step: Some(step),
                        message: message.clone(),
                        retrying: false,
                        at: ended,
                    }),
                )
                .await;
                Err(e)
            }
        }
    }

    // ---------- Replay (R4) ----------

    async fn replay(&self, mv: &MoveRecord) -> anyhow::Result<StepDone> {
        let inputs: MoveInputs = self.resumed(mv, StepName::Claim, "inputs").await?;
        let mut old = inventory::read_fallback(self.blobs.as_ref(), &mv.site, &inputs).await?;
        let mut outputs = Vec::new();
        if self.rooms.supports(JobKind::Replay) && (inputs.dump.is_some() || inputs.wxr.is_some()) {
            // room-replay's contract (S2-05): bare hex digests and the public origin; it
            // records each page as a browser and as a crawler sees it.
            let hex = |d: &Option<Digest>| d.as_ref().map(Digest::hex);
            let mut args = json!({ "public_origin": inputs.origin, "max_pages": 200 });
            for (key, digest) in [
                ("dump", &inputs.dump),
                ("wxr", &inputs.wxr),
                ("wp_content", &inputs.wp_content),
            ] {
                if let Some(h) = hex(digest) {
                    args[key] = json!(h);
                }
            }
            if inputs.dump.is_some() {
                args.as_object_mut().map(|o| o.remove("wxr"));
            }
            let job = RoomJob {
                kind: JobKind::Replay,
                inputs: [inputs.dump, inputs.wxr, inputs.wp_content]
                    .into_iter()
                    .flatten()
                    .collect(),
                args,
            };
            match self.rooms.run(&mv.site, &mv.id, job).await {
                Ok(run) => {
                    outputs.push(run.receipt);
                    let recording = match run.artifact("recording.json") {
                        Some(d) if run.succeeded() => self
                            .get_json::<Vec<inventory::ReplayedPage>>(&mv.site, d)
                            .await
                            .ok(),
                        _ => None,
                    };
                    match recording {
                        Some(pages) => {
                            let read = inventory::merge_replay(&mut old, &pages, |file| {
                                run.artifact(file).copied()
                            });
                            old.replayed = true;
                            old.notes = vec![format!(
                                "Read in a sealed replay of your old site, with no route out: {read} pages, {} served differently to search crawlers.",
                                old.cloaked.len()
                            )];
                        }
                        None => old.notes.push(
                            "The sealed replay did not finish, so your export was used instead."
                                .into(),
                        ),
                    }
                }
                Err(e) => old.notes.push(format!(
                    "The sealed replay could not start ({}); your export was used instead.",
                    plain(&e)
                )),
            }
        }
        let total = u32::try_from(old.inventory.items.len()).unwrap_or(u32::MAX);
        for (n, item) in old.inventory.items.iter().enumerate() {
            self.emit(
                &mv.site,
                &mv.id,
                UiEvent::Page(PageEvent {
                    n: u32::try_from(n + 1).unwrap_or(u32::MAX),
                    of: total,
                    url: item.path.clone(),
                    status: PageStatus::Read,
                    reason: None,
                    at: Utc::now(),
                }),
            )
            .await;
        }
        let digest = self.put_json(&mv.site, &old).await?;
        outputs.insert(0, digest);
        Ok(StepDone {
            summary: old.summary(),
            resume: json!({ "old": digest }),
            inputs: [inputs.wxr, inputs.wp_content, inputs.dump, inputs.crawl]
                .into_iter()
                .flatten()
                .collect(),
            outputs,
            detail: json!({
                "replayed": old.replayed,
                "pages": old.inventory.items.len(),
                "files": old.media.len(),
                "notes": old.notes,
            }),
            waiting: false,
            state: None,
        })
    }

    // ---------- Clean (R5) ----------

    async fn clean(&self, mv: &MoveRecord) -> anyhow::Result<StepDone> {
        let old: OldSite = self.resumed(mv, StepName::Replay, "old").await?;
        let inventory = old.hydrate(self.blobs.as_ref(), &mv.site).await?;
        let restored = self
            .store
            .quarantine(&mv.site)
            .await?
            .into_iter()
            .filter(|q| q.state == QuarantineState::Restored)
            .filter_map(|q| kind_of_title(&q.title).map(|k| (k, q.location)))
            .collect();
        let ctx = CleanContext {
            site: &mv.site,
            call: self.call(mv),
            blobs: self.blobs.as_ref(),
            gateway: self.gateway.as_ref(),
            restored,
            now: Utc::now(),
        };
        let site = clean::clean(&ctx, &old, &inventory).await?;
        self.store.insert_findings(&mv.id, &site.findings).await?;
        self.store
            .insert_quarantine(&mv.site, &mv.id, &site.quarantined, Utc::now())
            .await?;
        for q in &site.quarantined {
            self.emit(
                &mv.site,
                &mv.id,
                UiEvent::Quarantined(QuarantinedEvent {
                    item_id: q.id.clone(),
                    title: q.title.clone(),
                    reason: q.reason.clone(),
                    location: q.location.clone(),
                    at: Utc::now(),
                }),
            )
            .await;
        }
        let digest = self.put_json(&mv.site, &site).await?;
        let mut outputs = vec![digest];
        outputs.extend(
            site.quarantined
                .iter()
                .flat_map(|q| q.evidence.first().copied()),
        );
        Ok(StepDone {
            summary: site.summary(),
            resume: json!({ "clean": digest }),
            inputs: vec![],
            outputs,
            detail: json!({
                "pages": site.pages.len(),
                "files": site.media.len(),
                "quarantined": site.quarantined.iter().map(|q| QuarantineEntry {
                    item_id: q.id.clone(),
                    title: q.title.clone(),
                    reason: q.reason.clone(),
                    location: q.location.clone(),
                    detector: Some(q.kind.detector().to_owned()),
                }).collect::<Vec<_>>(),
                "notes": site.notes,
            }),
            waiting: false,
            state: None,
        })
    }

    // ---------- Rebuild (R6.1, R6.3) ----------

    async fn rebuild(&self, mv: &MoveRecord) -> anyhow::Result<StepDone> {
        let old: OldSite = self.resumed(mv, StepName::Replay, "old").await?;
        let site: CleanSite = self.resumed(mv, StepName::Clean, "clean").await?;
        let mut outputs = Vec::new();
        // The look the owner chose before the Move wins over the old theme's; the old theme's
        // build step then never runs.
        let inputs: MoveInputs = self.resumed(mv, StepName::Claim, "inputs").await?;
        let (tokens, legacy_theme) = match &inputs.look {
            Some(look) => (
                Tokens::chosen(look),
                Some("Your site was rebuilt in the new look you chose, so the old theme's styles weren't used.".to_owned()),
            ),
            None => self.theme_tokens(mv, &old, &mut outputs).await?,
        };
        let site_paths: BTreeSet<String> = site.pages.iter().map(|p| p.path.clone()).collect();
        let total = u32::try_from(site.pages.len()).unwrap_or(u32::MAX);
        // Pages build concurrently (N1: a 50-page Move reaches its card in minutes); events
        // are emitted as each page finishes.
        let call = self.call(mv);
        let mut done = 0u32;
        let mut built_pages: Vec<(usize, BuiltBody)> = Vec::with_capacity(site.pages.len());
        let jobs: Vec<BoxFuture<'_, _>> = site
            .pages
            .iter()
            .enumerate()
            .map(|(i, page)| {
                let (call, site_paths, name) = (call.clone(), &site_paths, &site.site_name);
                async move {
                    let mut retries = Vec::new();
                    let body = rebuild::build_page(
                        self.gateway.as_ref(),
                        &call,
                        name,
                        page,
                        site_paths,
                        1,
                        None,
                        &mut |attempt, problem| retries.push((attempt, problem.to_owned())),
                    )
                    .await;
                    (i, page, body, retries)
                }
                .boxed()
            })
            .collect();
        let mut pending = futures::stream::iter(jobs).buffer_unordered(BUILD_CONCURRENCY);
        while let Some((i, page, body, retries)) = pending.next().await {
            for (attempt, problem) in retries {
                self.emit(
                    &mv.site,
                    &mv.id,
                    UiEvent::Error(ErrorEvent {
                        step: Some(StepName::Rebuild),
                        message: clip(&format!(
                            "{}: try {attempt} of {MAX_ATTEMPTS} failed ({problem}); trying again",
                            page.path
                        )),
                        retrying: true,
                        at: Utc::now(),
                    }),
                )
                .await;
            }
            done += 1;
            self.emit(
                &mv.site,
                &mv.id,
                UiEvent::Page(PageEvent {
                    n: done,
                    of: total,
                    url: page.path.clone(),
                    status: if body.flagged.is_some() {
                        PageStatus::Failed
                    } else {
                        PageStatus::Read
                    },
                    reason: body.flagged.as_ref().map(|f| clip(f)),
                    at: Utc::now(),
                }),
            )
            .await;
            built_pages.push((i, body));
        }
        drop(pending);
        built_pages.sort_by_key(|(i, _)| *i);
        let bodies: Vec<BuiltBody> = built_pages.into_iter().map(|(_, b)| b).collect();
        let retried = bodies.iter().filter(|b| b.attempts > 1).count();
        let flagged = bodies.iter().filter(|b| b.flagged.is_some()).count();
        let built = BuiltSite {
            bodies,
            tokens,
            legacy_theme: legacy_theme.clone(),
        };
        let digest = self.put_json(&mv.site, &built).await?;
        outputs.insert(0, digest);
        let mut summary = format!("Rebuilt {} pages", built.bodies.len());
        if retried > 0 {
            summary.push_str(&format!("; {retried} needed another try"));
        }
        if flagged > 0 {
            summary.push_str(&format!("; {flagged} flagged for you"));
        }
        Ok(StepDone {
            summary,
            resume: json!({ "built": digest }),
            inputs: vec![],
            outputs,
            detail: json!({
                "pages": built.bodies.len(),
                "retried": retried,
                "flagged": flagged,
                "builders": built.bodies.iter().map(|b| json!({"path": b.path, "attempts": b.attempts, "builder": b.builder})).collect::<Vec<_>>(),
                "legacy_theme": legacy_theme,
            }),
            waiting: false,
            state: None,
        })
    }

    /// The site's look: the old theme's colours, read from its stylesheet; when the old
    /// theme has a build hook, it runs only inside a `build` room with no network, and a room
    /// killed by its caps is retried once without the hook (gauntlet attack 9).
    async fn theme_tokens(
        &self,
        mv: &MoveRecord,
        old: &OldSite,
        receipts: &mut Vec<Digest>,
    ) -> anyhow::Result<(Tokens, Option<String>)> {
        let theme = old.theme_hooks.first().cloned().or_else(|| {
            old.code.iter().find_map(|f| {
                f.path
                    .strip_prefix("themes/")?
                    .strip_suffix("/style.css")
                    .filter(|t| !t.contains('/'))
                    .map(str::to_owned)
            })
        });
        let mut css = String::new();
        if let Some(theme) = &theme
            && let Some(file) = old
                .code
                .iter()
                .find(|f| f.path == format!("themes/{theme}/style.css"))
            && let Some(bytes) = self.blobs.get(&mv.site, &file.digest).await?
        {
            css = String::from_utf8_lossy(&bytes).into_owned();
        }
        let mut words = None;
        if let (Some(theme), Some(wp)) = (old.theme_hooks.first(), old.wp_content)
            && self.rooms.supports(JobKind::Build)
        {
            let job = RoomJob {
                kind: JobKind::Build,
                inputs: vec![wp],
                args: json!({ "step": "legacy-theme", "theme": theme }),
            };
            match self.rooms.run(&mv.site, &mv.id, job).await {
                Ok(run) => {
                    receipts.push(run.receipt);
                    if let Some(reason) = run.result.killed {
                        let line = format!(
                            "The old theme's build step was stopped by the sealed room's limits ({}); the room was destroyed and the build was retried without it.",
                            serde_json::to_value(reason)?.as_str().unwrap_or("caps")
                        );
                        self.emit(
                            &mv.site,
                            &mv.id,
                            UiEvent::Error(ErrorEvent {
                                step: Some(StepName::Rebuild),
                                message: clip(&line),
                                retrying: true,
                                at: Utc::now(),
                            }),
                        )
                        .await;
                        let retry = RoomJob {
                            kind: JobKind::Build,
                            inputs: vec![wp],
                            args: json!({ "step": "site", "theme": theme }),
                        };
                        match self.rooms.run(&mv.site, &mv.id, retry).await {
                            Ok(again) => receipts.push(again.receipt),
                            Err(e) => {
                                tracing::warn!(error = %e, "the clean build retry did not run")
                            }
                        }
                        words = Some(line);
                    } else if run.succeeded() {
                        for artifact in run.artifacts.iter().filter(|a| a.name.ends_with(".css")) {
                            if let Some(bytes) = self.blobs.get(&mv.site, &artifact.digest).await? {
                                css.push_str(&String::from_utf8_lossy(&bytes));
                            }
                        }
                        words = Some("The old theme's styles were built in a sealed room.".into());
                    } else {
                        words = Some("The old theme's build step failed in its sealed room; its plain styles were used.".into());
                    }
                }
                Err(e) => {
                    words = Some(format!(
                        "The old theme's build step could not run ({}).",
                        plain(&e)
                    ))
                }
            }
        }
        Ok((
            if css.is_empty() {
                Tokens::default()
            } else {
                Tokens::from_css(&css)
            },
            words,
        ))
    }

    // ---------- Prove (R6.2, R6.4) ----------

    /// The release's files: each page in the site theme and its Markdown twin, the theme's
    /// stylesheet, and the clean uploads. Pages carry W8's visit counter, naming release `seq`
    /// once it is known (the parity renders before it don't count anything: their room has no
    /// network).
    async fn site_files(
        &self,
        mv: &MoveRecord,
        inputs: &MoveInputs,
        site: &CleanSite,
        built: &BuiltSite,
        seq: Option<u64>,
    ) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
        let contact = site
            .pages
            .iter()
            .find(|p| p.path.contains("contact"))
            .map(|p| p.path.clone());
        let view = SiteView {
            name: site.site_name.clone(),
            domain: inputs.domain.clone(),
            lang: site.language.clone(),
            tokens: built.tokens.clone(),
            nav: site.nav.clone(),
            contact_href: contact,
            fixture: inputs.fixture,
            counting: true,
            counter: Some(agentrix_store::visits::script_tag(
                seq.and_then(|s| i64::try_from(s).ok()),
            )),
        };
        let mut files = BTreeMap::new();
        for page in &site.pages {
            let Some(body) = built.bodies.iter().find(|b| b.path == page.path) else {
                continue;
            };
            let md_href = format!("/{}", theme::twin_file(&page.path));
            let html = theme::render_page(
                &view,
                &PageView {
                    title: &page.title,
                    description: page.description.as_deref(),
                    path: &page.path,
                    md_href,
                    body_html: &body.body_html,
                    updated: None,
                },
            )?;
            files.insert(theme::page_file(&page.path), html.into_bytes());
            files.insert(
                theme::twin_file(&page.path),
                format!("# {}\n\n{}", page.title, page.markdown).into_bytes(),
            );
        }
        files.insert("assets/site.css".into(), theme::SITE_CSS.to_vec());
        for media in &site.media {
            if let Some(bytes) = self.blobs.get(&mv.site, &media.digest).await? {
                files.insert(media.path.trim_start_matches('/').to_owned(), bytes);
            }
        }
        Ok(files)
    }

    #[allow(clippy::too_many_arguments)]
    async fn render_and_score(
        &self,
        mv: &MoveRecord,
        old: &OldSite,
        site: &CleanSite,
        built: &BuiltSite,
        files: &BTreeMap<String, Vec<u8>>,
        check: &BTreeSet<String>,
        report: &mut ParityReport,
    ) -> anyhow::Result<()> {
        let mut shots: BTreeMap<String, Digest> = BTreeMap::new();
        if self.rooms.supports(JobKind::Render) {
            // A browser needs the pages, the stylesheet and the images, not the Markdown
            // twins. Pages go in batches so each room stays within the runner's input limit.
            let mut shared = BTreeMap::new();
            let mut page_digests = BTreeMap::new();
            for (path, bytes) in files {
                if path.ends_with(".md") {
                    continue;
                }
                let kind = adk_site::release::content_type(path);
                let digest = self
                    .blobs
                    .put(&mv.site, "candidate-file", kind, bytes)
                    .await?;
                if path.ends_with(".html") {
                    page_digests.insert(path.clone(), digest);
                } else {
                    shared.insert(path.clone(), digest);
                }
            }
            let cap = self.policy.room_inputs.max(2);
            if shared.len() + 1 > cap {
                // Too many images to carry: keep the stylesheet so pages still render styled.
                shared.retain(|path, _| path.ends_with(".css"));
            }
            let per_room = cap.saturating_sub(shared.len()).max(1);
            let checking: Vec<&crate::clean::CleanPage> = site
                .pages
                .iter()
                .filter(|p| check.contains(&p.path))
                .collect();
            let mut failures = Vec::new();
            let mut rooms = 0;
            for batch in checking.chunks(per_room) {
                rooms += 1;
                let mut files_arg = shared.clone();
                let pages: Vec<Value> = batch
                    .iter()
                    .filter_map(|p| {
                        let file = theme::page_file(&p.path);
                        let digest = page_digests.get(&file)?;
                        files_arg.insert(file.clone(), *digest);
                        Some(json!({ "path": p.path, "file": file, "shot": parity::shot_name(&file) }))
                    })
                    .collect();
                let job = RoomJob {
                    kind: JobKind::Render,
                    inputs: files_arg
                        .values()
                        .copied()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect(),
                    args: json!({ "files": files_arg, "pages": pages, "width": 390, "height": 844 }),
                };
                // A render room that fails is said plainly and its pages fall back to their
                // text, rather than stopping the Move on a sandbox fault.
                match self.rooms.run(&mv.site, &mv.id, job).await {
                    Ok(run) => {
                        report.render_receipts.push(run.receipt);
                        if !run.succeeded() {
                            failures
                                .push("a sealed render room stopped before it finished".to_owned());
                            continue;
                        }
                        for page in batch {
                            if let Some(d) =
                                run.artifact(&parity::shot_name(&theme::page_file(&page.path)))
                            {
                                shots.insert(page.path.clone(), *d);
                            }
                        }
                    }
                    Err(e) => failures.push(plain(&e)),
                }
            }
            report.rendered_in_room = failures.is_empty();
            if let Some(why) = failures.first() {
                tracing::warn!(move_id = %mv.id, %why, failed = failures.len(), rooms, "render room failed; parity on text alone for its pages");
                report.render_note = Some(format!(
                    "{} of {rooms} render rooms didn't finish ({why}), so their pages' parity was scored on text alone.",
                    failures.len()
                ));
                self.emit(
                    &mv.site,
                    &mv.id,
                    UiEvent::Error(ErrorEvent {
                        step: Some(StepName::Prove),
                        message: clip(report.render_note.as_deref().unwrap_or_default()),
                        retrying: false,
                        at: Utc::now(),
                    }),
                )
                .await;
            }
        }
        let scoring = site
            .pages
            .iter()
            .filter(|p| check.contains(&p.path))
            .filter_map(|page| {
                let body = built.bodies.iter().find(|b| b.path == page.path)?;
                Some(
                    self.score_page(mv, old, page, body, shots.get(&page.path).copied())
                        .boxed(),
                )
            })
            .collect::<Vec<BoxFuture<'_, _>>>();
        let scored: Vec<anyhow::Result<PageParity>> = futures::stream::iter(scoring)
            .buffer_unordered(BUILD_CONCURRENCY)
            .collect()
            .await;
        for entry in scored {
            let entry = entry?;
            self.emit(
                &mv.site,
                &mv.id,
                UiEvent::Parity(ParityEvent {
                    page: Some(entry.path.clone()),
                    score: entry.score,
                    verdict: clip(&entry.verdict),
                    state: entry.state,
                    attempt: entry.attempt,
                    at: Utc::now(),
                }),
            )
            .await;
            report.pages.retain(|p| p.path != entry.path);
            report.pages.push(entry);
        }
        report.pages.sort_by(|a, b| a.path.cmp(&b.path));
        report.recompute();
        Ok(())
    }

    /// One page's parity: text against the clean source, and the vision verdict when the
    /// render room made a screenshot.
    async fn score_page(
        &self,
        mv: &MoveRecord,
        old: &OldSite,
        page: &crate::clean::CleanPage,
        body: &BuiltBody,
        shot: Option<Digest>,
    ) -> anyhow::Result<PageParity> {
        let text = parity::text_parity(&page.source(), &body.body_html);
        let mut vision = None;
        let mut verdict = format!("{:.0}% of the clean text is on the page", text * 100.0);
        if let Some(shot) = shot
            && let Some(new_png) = self.blobs.get(&mv.site, &shot).await?
        {
            let old_png = match old.screenshots.get(&page.path) {
                Some(d) => self.blobs.get(&mv.site, d).await?,
                None => None,
            };
            let request = VisionRequest {
                path: page.path.clone(),
                new_png,
                old_png,
                expected_text: parity::words_of_markdown(&page.source()),
            };
            let ctx = CallContext {
                attempt: body.attempts,
                ..self.call(mv)
            };
            match self.gateway.vision_parity(&ctx, &request).await {
                Ok(v) => {
                    vision = Some(v.score.clamp(0.0, 1.0));
                    verdict = v.verdict;
                }
                Err(e) => tracing::warn!(error = %e, "vision parity unavailable; scoring on text"),
            }
        }
        let score = parity::page_score(text, vision);
        let state = if score >= self.policy.page_pass && body.flagged.is_none() {
            ParityState::Pass
        } else if body.attempts < MAX_ATTEMPTS {
            ParityState::Retry
        } else {
            ParityState::Flagged
        };
        Ok(PageParity {
            path: page.path.clone(),
            score,
            text,
            vision,
            verdict,
            state,
            attempt: body.attempts,
            screenshot: shot,
            accepted: false,
        })
    }

    async fn prove(&self, mv: &MoveRecord) -> anyhow::Result<StepDone> {
        let inputs: MoveInputs = self.resumed(mv, StepName::Claim, "inputs").await?;
        let old: OldSite = self.resumed(mv, StepName::Replay, "old").await?;
        let site: CleanSite = self.resumed(mv, StepName::Clean, "clean").await?;
        let mut built: BuiltSite = self.resumed(mv, StepName::Rebuild, "built").await?;
        let site_paths: BTreeSet<String> = site.pages.iter().map(|p| p.path.clone()).collect();
        let mut report = ParityReport {
            pages: vec![],
            site: 0.0,
            rendered_in_room: false,
            render_receipts: vec![],
            render_note: None,
        };
        let mut check = site_paths.clone();
        loop {
            let files = self.site_files(mv, &inputs, &site, &built, None).await?;
            self.render_and_score(mv, &old, &site, &built, &files, &check, &mut report)
                .await?;
            let retry: Vec<PageParity> = report
                .pages
                .iter()
                .filter(|p| p.state == ParityState::Retry)
                .cloned()
                .collect();
            if retry.is_empty() {
                break;
            }
            check.clear();
            for shortfall in retry {
                let Some(page) = site.pages.iter().find(|p| p.path == shortfall.path) else {
                    continue;
                };
                let attempts = built
                    .bodies
                    .iter()
                    .find(|b| b.path == page.path)
                    .map_or(1, |b| b.attempts);
                let mut notes = Vec::new();
                let body = rebuild::build_page(
                    self.gateway.as_ref(),
                    &self.call(mv),
                    &site.site_name,
                    page,
                    &site_paths,
                    attempts + 1,
                    Some(parity::feedback(&shortfall)),
                    &mut |attempt, problem| notes.push((attempt, problem.to_owned())),
                )
                .await;
                built.bodies.retain(|b| b.path != body.path);
                built.bodies.push(body);
                check.insert(page.path.clone());
            }
        }
        self.emit(
            &mv.site,
            &mv.id,
            UiEvent::Parity(ParityEvent {
                page: None,
                score: report.site,
                verdict: format!(
                    "Site parity {:.1}% over {} pages",
                    report.site * 100.0,
                    report.pages.len()
                ),
                state: if report.candidate_ready(&self.policy) {
                    ParityState::Pass
                } else {
                    ParityState::Flagged
                },
                attempt: 1,
                at: Utc::now(),
            }),
        )
        .await;
        self.finish_prove(mv, &inputs, &site, built, report).await
    }

    async fn finish_prove(
        &self,
        mv: &MoveRecord,
        inputs: &MoveInputs,
        site: &CleanSite,
        built: BuiltSite,
        report: ParityReport,
    ) -> anyhow::Result<StepDone> {
        let built_digest = self.put_json(&mv.site, &built).await?;
        let mut outputs = vec![built_digest];
        outputs.extend(report.render_receipts.iter().copied());
        let candidate = if report.candidate_ready(&self.policy) {
            let signed = self.store.signed_releases(&mv.site).await?;
            let seq = signed.iter().map(|r| r.seq).max().unwrap_or(0) + 1;
            let previous = self.store.pointer(&mv.site).await?.map(|(_, d)| d);
            let files = self.site_files(mv, inputs, site, &built, Some(seq)).await?;
            let staged = candidate::stage(
                self.blobs.as_ref(),
                &mv.site,
                seq,
                previous,
                files,
                site.redirects.clone(),
                site.pages.len(),
                Utc::now(),
            )
            .await?;
            self.store
                .put_candidate(&mv.site, staged.seq, &staged.manifest_digest, Utc::now())
                .await?;
            outputs.push(staged.manifest_digest);
            Some(staged)
        } else {
            None
        };
        let flags: Vec<String> = report.open_flags().iter().map(|p| p.path.clone()).collect();
        let proved = Proved {
            parity: report,
            candidate,
        };
        let digest = self.put_json(&mv.site, &proved).await?;
        outputs.insert(0, digest);
        let ready = proved.candidate.is_some();
        let rendered = if proved.parity.rendered_in_room {
            ""
        } else {
            " (text only, not rendered)"
        };
        let summary = match &proved.candidate {
            Some(c) => format!(
                "Parity {:.1}%{rendered}; release {} is ready for you",
                proved.parity.site * 100.0,
                c.seq
            ),
            None if !flags.is_empty() => format!(
                "Parity {:.1}%; {} pages need your look",
                proved.parity.site * 100.0,
                flags.len()
            ),
            None => format!(
                "Parity {:.1}% is below 95%; the Move needs a person",
                proved.parity.site * 100.0
            ),
        };
        Ok(StepDone {
            summary,
            resume: json!({ "proved": digest, "built": built_digest }),
            inputs: vec![],
            outputs,
            detail: json!({
                "parity": proved.parity.site,
                "rendered_in_room": proved.parity.rendered_in_room,
                "pages": proved.parity.pages.iter().map(|p| json!({
                    "path": p.path, "score": p.score, "text": p.text, "vision": p.vision,
                    "state": p.state, "attempt": p.attempt, "verdict": p.verdict,
                })).collect::<Vec<_>>(),
                "flagged": flags,
                "candidate": proved.candidate,
            }),
            waiting: !ready,
            state: Some(if ready {
                MoveState::Proved
            } else {
                MoveState::Rebuilt
            }),
        })
    }

    /// The owner accepts flagged pages as they are (R6.4). When that clears every flag and the
    /// site's parity is at the bar, the candidate is staged and the Move continues to go-live.
    pub async fn accept_pages(
        &self,
        move_id: &MoveId,
        paths: &[String],
    ) -> anyhow::Result<MoveState> {
        let mv = self
            .store
            .move_record(move_id)
            .await?
            .with_context(|| format!("no Move {move_id}"))?;
        let prove_waits = self
            .store
            .steps(move_id)
            .await?
            .into_iter()
            .rev()
            .find(|s| s.step == StepName::Prove)
            .is_some_and(|s| s.state == StepState::Waiting);
        anyhow::ensure!(
            mv.state != MoveState::Rebuilt || prove_waits,
            "The pages are still being checked. Accept a page when the Move asks you to."
        );
        anyhow::ensure!(
            mv.state == MoveState::Rebuilt,
            "This Move has no pages waiting for you."
        );
        let inputs: MoveInputs = self.resumed(&mv, StepName::Claim, "inputs").await?;
        let site: CleanSite = self.resumed(&mv, StepName::Clean, "clean").await?;
        let mut proved: Proved = self.resumed(&mv, StepName::Prove, "proved").await?;
        let built: BuiltSite = self.resumed(&mv, StepName::Prove, "built").await?;
        for page in &mut proved.parity.pages {
            if paths.contains(&page.path) && page.state == ParityState::Flagged {
                page.accepted = true;
            }
        }
        let started = Utc::now();
        let done = self
            .finish_prove(&mv, &inputs, &site, built, proved.parity)
            .await?;
        let mut body = ReceiptBody::new(
            ReceiptKind::MoveStep,
            mv.site.clone(),
            Subject::Move(mv.id.clone()),
            started,
            Utc::now(),
        );
        body.outputs = done.outputs.clone();
        body.detail = Some(json!({ "step": "prove", "accepted": paths, "summary": done.summary }));
        let receipt = self
            .receipts
            .issue(body.with_title(format!("You accepted {} pages as they are", paths.len())))
            .await?;
        let state = if done.waiting {
            StepState::Waiting
        } else {
            StepState::Done
        };
        self.store
            .record_step(
                &mv.id,
                &mv.site,
                StepName::Prove,
                state,
                &done.summary,
                Some(&receipt),
                Some(&done.resume),
                started,
                Some(Utc::now()),
            )
            .await?;
        let next = done.state.unwrap_or(MoveState::Proved);
        self.store.set_state(&mv.id, next, Utc::now()).await?;
        self.run(move_id).await
    }

    // ---------- Go-live card (R7.1) ----------

    async fn raise_go_live(&self, mv: &MoveRecord) -> anyhow::Result<StepDone> {
        let inputs: MoveInputs = self.resumed(mv, StepName::Claim, "inputs").await?;
        let site: CleanSite = self.resumed(mv, StepName::Clean, "clean").await?;
        let proved: Proved = self.resumed(mv, StepName::Prove, "proved").await?;
        let candidate = proved
            .candidate
            .clone()
            .context("no candidate release to raise a card for")?;
        let charter = charter_v1();
        self.store
            .put_charter(&mv.site, 1, &charter, Utc::now())
            .await?;
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        for q in &site.quarantined {
            *kinds.entry(q.title.clone()).or_default() += 1;
        }
        let mut kinds: Vec<(String, usize)> = kinds.into_iter().collect();
        kinds.sort_by_key(|k| std::cmp::Reverse(k.1));
        let facts = GoLiveFacts {
            move_id: &mv.id,
            domain: &inputs.domain,
            seq: candidate.seq,
            manifest: &candidate.manifest_digest,
            pages: candidate.pages,
            parity: proved.parity.site,
            left_behind: site.quarantined.len(),
            left_behind_kinds: kinds.into_iter().map(|(k, _)| k).take(4).collect(),
            site: &mv.site,
            charter: &charter,
        };
        let mut card = go_live_card(&facts, Utc::now())?;
        let state = format!(
            "Publish a {}-page website rebuilt from the owner's own content at {:.1}% parity; {} infected items were left out; reversible for 30 days.",
            candidate.pages,
            proved.parity.site * 100.0,
            site.quarantined.len()
        );
        match self.gateway.decide(&self.call(mv), &state).await {
            Ok(risk) => card.risk = risk,
            Err(e) => tracing::warn!(error = %e, "no risk score for the go-live card"),
        }
        self.store.insert_card(&card).await?;
        let href = format!("/console/{}/decisions/{}", mv.site, card.decision_id);
        self.emit(
            &mv.site,
            &mv.id,
            UiEvent::Card(CardEvent {
                decision_id: card.decision_id.clone(),
                kind: CardKind::GoLive,
                title: card.title.clone(),
                href: href.clone(),
                at: Utc::now(),
            }),
        )
        .await;
        self.emit(
            &mv.site,
            &mv.id,
            UiEvent::Done(DoneEvent {
                outcome: DoneOutcome::CandidateReady,
                href: Some(href),
                at: Utc::now(),
            }),
        )
        .await;
        Ok(StepDone {
            summary: "Waiting for you: your go-live card is ready".into(),
            resume: json!({ "decision": card.decision_id, "release": candidate.manifest_digest, "seq": candidate.seq }),
            inputs: vec![candidate.manifest_digest],
            outputs: vec![],
            detail: json!({
                "decision": card.decision_id,
                "fingerprint": card.fingerprint,
                "release": candidate.manifest_digest,
                "charter": charter_digest(&charter)?,
            }),
            waiting: true,
            state: Some(MoveState::AwaitingOwner),
        })
    }

    /// Resumes every Move a crash interrupted (N9). Call once at startup.
    pub async fn resume_all(&self) -> Vec<(MoveId, anyhow::Result<MoveState>)> {
        let mut out = Vec::new();
        match self.store.unfinished_moves().await {
            Ok(ids) => {
                for id in ids {
                    let result = self.run(&id).await;
                    out.push((id, result));
                }
            }
            Err(e) => tracing::warn!(error = %e, "could not list unfinished Moves"),
        }
        out
    }
}

fn clip(text: &str) -> String {
    let mut out: String = text.chars().take(139).collect();
    if text.chars().count() > 139 {
        out.push('…');
    }
    out
}

/// The quarantine kind a stored title names.
pub fn kind_of_title(title: &str) -> Option<LeftBehind> {
    use LeftBehind::*;
    [
        HiddenLinks,
        SpamLinks,
        SpamPage,
        Cloaking,
        Executable,
        MimeMismatch,
        ObfuscatedScript,
        AgentInstructions,
        CallsOut,
        PrivateAddress,
        Backdoor,
    ]
    .into_iter()
    .find(|k| k.title() == title)
}

/// The time a Move started, for the receipts.
pub fn started_at(mv: &MoveRecord) -> DateTime<Utc> {
    mv.created_at
}
