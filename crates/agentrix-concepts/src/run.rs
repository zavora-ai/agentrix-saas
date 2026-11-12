//! One run: the planner reads the site's facts and writes three directions; three designers
//! then each build `index.html` in their own room, in parallel, with `fs_write`. Each finished
//! file is checked for the fundamentals and sent back to the same designer, in the same room,
//! until it passes (at most [`check::MAX_ROUNDS`] rounds). The owner can then tell any designer
//! what to change ([`Engine::revise`]). Progress goes to a live [`Hub`] the page follows over
//! SSE; results go to `control.db`, the content store and a receipt per page.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use agentrix_host::{Blobs, Receipts};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

use crate::check;
use crate::facts::{SiteFacts, site_facts};
use crate::prompts::{self, Direction};
use crate::runner::{AgentJob, AgentOutput, AgentRunner};
use crate::sanitize::{Policy, sanitize};
use crate::store::{self, ConceptRow, RunRow};
use crate::stream::{StreamEvent, extract_html};

/// The models a run uses.
#[derive(Debug, Clone)]
pub struct Models {
    /// Writes the three directions.
    pub planner: String,
    /// Writes each page.
    pub worker: String,
}

/// A run as the page shows it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Snapshot {
    /// The run.
    pub run_id: String,
    /// `planning`, `designing`, `revising`, `done` or `failed`.
    pub state: String,
    /// Where the pipeline strip is: `facts`, `plan`, `agents`, `check`, `choose` or `move`.
    pub stage: String,
    /// What's happening, in plain words.
    pub words: String,
    /// Seconds since the run started, or its total once finished.
    pub seconds: u64,
    /// The three designers.
    pub agents: Vec<AgentView>,
}

/// One designer as the page shows it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentView {
    /// 1, 2 or 3.
    pub n: i64,
    /// Its design.
    pub concept_id: String,
    /// The direction's name.
    pub name: String,
    /// The direction in a sentence.
    pub sentence: String,
    /// `waiting`, `reading`, `writing`, `checking`, `fixing`, `revising`, `done` or `failed`.
    pub status: String,
    /// What it's doing, in plain words.
    pub words: String,
    /// Characters in its `index.html` so far.
    pub chars: usize,
    /// Seconds it took, once finished.
    pub seconds: u64,
    /// Whether the owner chose it.
    pub chosen: bool,
    /// The end of its `index.html` while it writes (shown as text, never as markup).
    #[serde(default)]
    pub tail: String,
    /// `fs_write` calls that saved.
    #[serde(default)]
    pub writes: i64,
    /// Check-and-fix rounds.
    #[serde(default)]
    pub rounds: i64,
    /// Tools Zavora refused, by name.
    #[serde(default)]
    pub refused: BTreeMap<String, i64>,
    /// What the last check found missing, in plain words.
    #[serde(default)]
    pub missing: Vec<String>,
    /// The finished page's fingerprint, short.
    #[serde(default)]
    pub digest: String,
    /// Whether it is working on a change the owner asked for.
    #[serde(default)]
    pub busy: bool,
}

/// Seconds for a designer's first build call (fixes and changes get [`crate::AGENT_SECS`]).
pub const BUILD_SECS: u64 = 420;

/// How much of the newest writing the page shows above each preview.
const TAIL_CHARS: usize = 280;
/// The largest `index.html` read back from a room.
const MAX_PAGE_BYTES: u64 = 3 * 1024 * 1024;

/// The last `max` characters of `text`.
fn tail_of(text: &str, max: usize) -> String {
    let start = text
        .char_indices()
        .rev()
        .nth(max.saturating_sub(1))
        .map_or(0, |(i, _)| i);
    text[start..].to_owned()
}

/// `sha256:abcdef…` as `abcdef123456`.
fn short(digest: &str) -> String {
    digest
        .strip_prefix("sha256:")
        .unwrap_or(digest)
        .chars()
        .take(12)
        .collect()
}

/// The plain words for a check's missing keys.
fn missing_words(keys: &[String]) -> Vec<String> {
    keys.iter()
        .map(|k| check::words_for(k).unwrap_or(k).to_owned())
        .collect()
}

impl Snapshot {
    /// A finished (or interrupted) run, from its records.
    pub fn from_records(run: &RunRow, concepts: &[ConceptRow]) -> Self {
        let seconds = run
            .duration_ms
            .map_or(0, |d| u64::try_from(d / 1000).unwrap_or(0));
        let words = match run.state.as_str() {
            "done" => format!(
                "All three are ready, {seconds} seconds from start. Pick the one that feels like your business, or tell a designer what to change."
            ),
            "failed" if !run.note.is_empty() => run.note.clone(),
            "failed" => "This run stopped before it finished.".to_owned(),
            _ => "Still working.".to_owned(),
        };
        let mut s = Self {
            run_id: run.run_id.clone(),
            state: run.state.clone(),
            stage: String::new(),
            words,
            seconds,
            agents: concepts
                .iter()
                .map(|c| {
                    let secs = c
                        .duration_ms
                        .map_or(0, |d| u64::try_from(d / 1000).unwrap_or(0));
                    let missing: Vec<String> = serde_json::from_str(&c.missing).unwrap_or_default();
                    AgentView {
                        n: c.n,
                        concept_id: c.concept_id.clone(),
                        name: c.name.clone(),
                        sentence: c.direction.clone(),
                        status: c.status.clone(),
                        words: match c.status.as_str() {
                            "done" => done_words(secs, c.html_chars, &missing),
                            "failed" if !c.note.is_empty() => c.note.clone(),
                            "failed" => "This one didn't finish.".to_owned(),
                            _ => "Stopped.".to_owned(),
                        },
                        chars: usize::try_from(c.html_chars).unwrap_or(0),
                        seconds: secs,
                        chosen: c.chosen == 1,
                        tail: String::new(),
                        writes: c.writes,
                        rounds: c.rounds,
                        refused: serde_json::from_str(&c.refused).unwrap_or_default(),
                        missing: missing_words(&missing),
                        digest: c.html_digest.as_deref().map(short).unwrap_or_default(),
                        busy: false,
                    }
                })
                .collect(),
        };
        s.stage = s.stage_now().into();
        s
    }

    /// Where the pipeline strip is.
    pub fn stage_now(&self) -> &'static str {
        if self.agents.iter().any(|a| a.chosen) {
            return "move";
        }
        match self.state.as_str() {
            "planning" => "plan",
            "designing"
                if self
                    .agents
                    .iter()
                    .any(|a| a.status == "fixing" || a.status == "checking") =>
            {
                "check"
            }
            "designing" => "agents",
            "done" | "revising" => "choose",
            _ => "facts",
        }
    }
}

fn done_words(secs: u64, chars: i64, missing: &[String]) -> String {
    let checked = if missing.is_empty() {
        "Checked: all the basics are there.".to_owned()
    } else {
        format!(
            "Still missing after the fixes: {}.",
            missing_words(missing).join("; ")
        )
    };
    format!("Done in {secs} seconds, {chars} characters. {checked}")
}

/// A run being followed live.
pub struct LiveRun {
    snapshot: Mutex<Snapshot>,
    started: Instant,
    last_sent: Mutex<Instant>,
    tx: broadcast::Sender<String>,
}

impl LiveRun {
    fn from_snapshot(snapshot: Snapshot) -> Arc<Self> {
        let (tx, _) = broadcast::channel(64);
        Arc::new(Self {
            snapshot: Mutex::new(snapshot),
            started: Instant::now(),
            last_sent: Mutex::new(Instant::now()),
            tx,
        })
    }

    /// The run as it stands.
    pub fn snapshot(&self) -> Snapshot {
        let mut s = self.snapshot.lock().map(|s| s.clone()).unwrap_or_default();
        if matches!(s.state.as_str(), "planning" | "designing") {
            s.seconds = self.started.elapsed().as_secs();
        }
        s.stage = s.stage_now().into();
        s
    }

    /// The run as it stands, as JSON.
    pub fn snapshot_json(&self) -> String {
        serde_json::to_string(&self.snapshot()).unwrap_or_else(|_| "{}".into())
    }

    /// Follows the run: each message is the whole snapshot as JSON.
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    fn update(&self, force: bool, f: impl FnOnce(&mut Snapshot)) {
        if let Ok(mut s) = self.snapshot.lock() {
            f(&mut s);
        }
        let send = match self.last_sent.lock() {
            Ok(mut last) if force || last.elapsed().as_millis() >= 400 => {
                *last = Instant::now();
                true
            }
            _ => false,
        };
        if send {
            let _ = self.tx.send(self.snapshot_json());
        }
    }

    fn agent(&self, n: i64, force: bool, f: impl FnOnce(&mut AgentView)) {
        self.update(force, |s| {
            if let Some(a) = s.agents.iter_mut().find(|a| a.n == n) {
                f(a);
            }
        });
    }

    /// Whether designer `n` is working on something now.
    pub fn busy(&self, n: i64) -> bool {
        self.snapshot.lock().is_ok_and(|s| {
            s.agents.iter().any(|a| {
                a.n == n
                    && (a.busy
                        || matches!(
                            a.status.as_str(),
                            "waiting" | "reading" | "writing" | "checking" | "fixing" | "revising"
                        ))
            })
        })
    }
}

/// Runs followed on this node, by run id, for a day.
#[derive(Default)]
pub struct Hub {
    runs: Mutex<HashMap<String, Arc<LiveRun>>>,
}

impl Hub {
    /// The live run with this id, if it's on this node.
    pub fn get(&self, run_id: &str) -> Option<Arc<LiveRun>> {
        self.runs.lock().ok()?.get(run_id).cloned()
    }

    fn insert(&self, run_id: &str, live: Arc<LiveRun>) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.retain(|_, r| r.started.elapsed().as_secs() < 24 * 3600);
            runs.insert(run_id.to_owned(), live);
        }
    }

    fn start(&self, run_id: &str) -> Arc<LiveRun> {
        let live = LiveRun::from_snapshot(Snapshot {
            run_id: run_id.to_owned(),
            state: "planning".into(),
            words: "The planner is reading your site and choosing three directions.".into(),
            ..Snapshot::default()
        });
        self.insert(run_id, live.clone());
        live
    }
}

/// What a run needs.
#[derive(Clone)]
pub struct Engine {
    /// `control.db`.
    pub pool: sqlx::SqlitePool,
    /// The content store, for the site's pages and the finished designs.
    pub blobs: Arc<dyn Blobs>,
    /// Signs each page's receipt.
    pub receipts: Arc<dyn Receipts>,
    /// Runs one agent.
    pub runner: Arc<dyn AgentRunner>,
    /// Which models.
    pub models: Models,
    /// Live runs.
    pub hub: Arc<Hub>,
    /// Where designers' rooms are kept until the owner chooses.
    pub rooms: PathBuf,
}

/// Counts kept while one designer works.
#[derive(Debug, Default)]
struct Tally {
    writes: i64,
    refused: BTreeMap<String, i64>,
    /// Every model call, recorded when it starts: `{provider, model, kind, ms}`.
    calls: Vec<serde_json::Value>,
    key_checked_files: u64,
}

impl Engine {
    /// The room of design `concept_id` (ids are `cpt_` and hex, made here).
    pub fn room(&self, concept_id: &str) -> Option<PathBuf> {
        concept_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
            .then(|| self.rooms.join(concept_id))
    }

    /// The designer's `index.html` as it is now, if its room still exists.
    pub fn page_in_room(&self, concept_id: &str) -> Option<String> {
        read_page(&self.room(concept_id)?)
    }

    /// Deletes every room of `run_id` and records when.
    pub async fn delete_rooms(&self, run_id: &str) -> anyhow::Result<()> {
        for id in store::concept_ids(&self.pool, run_id).await? {
            if let Some(room) = self.room(&id) {
                let _ = tokio::fs::remove_dir_all(room).await;
            }
        }
        store::rooms_deleted(&self.pool, run_id).await
    }

    /// The live view of `run_id`, loading it from its records when this node isn't following it.
    pub async fn live(&self, site_id: &str, run_id: &str) -> Option<Arc<LiveRun>> {
        if let Some(live) = self.hub.get(run_id) {
            return Some(live);
        }
        let (run, concepts) = store::latest_run(&self.pool, site_id).await.ok()??;
        if run.run_id != run_id {
            return None;
        }
        let live = LiveRun::from_snapshot(Snapshot::from_records(&run, &concepts));
        self.hub.insert(run_id, live.clone());
        Some(live)
    }

    /// Records a new run for `site_id` and starts it in the background; returns its id.
    pub async fn start(&self, site_id: &str, account: &str) -> anyhow::Result<String> {
        let run_id = format!("crun_{}", uuid::Uuid::now_v7().simple());
        store::insert_run(
            &self.pool,
            &run_id,
            site_id,
            account,
            &self.models.planner,
            &self.models.worker,
        )
        .await?;
        let live = self.hub.start(&run_id);
        let engine = self.clone();
        let site = site_id.to_owned();
        let id = run_id.clone();
        tokio::spawn(async move {
            if let Err(e) = engine.run(&site, &id, &live).await {
                tracing::warn!(error = %e, run = %id, "concepts: run failed");
                let words = "This run stopped before it finished, so there are no new looks. Try three more.";
                let _ = store::set_run_state(&engine.pool, &id, "failed", words, None).await;
                live.update(true, |s| {
                    s.state = "failed".into();
                    s.words = words.into();
                });
            }
        });
        Ok(run_id)
    }

    async fn run(&self, site_id: &str, run_id: &str, live: &Arc<LiveRun>) -> anyhow::Result<()> {
        let started = Instant::now();
        let facts = site_facts(&self.pool, &self.blobs, site_id).await?;

        // The planner, in a fresh room deleted afterwards.
        let prompt = prompts::planner_prompt(&facts);
        let prompt_chars = i64::try_from(prompt.chars().count()).unwrap_or(i64::MAX);
        // Recorded when it starts, then replaced by what Zavora reports.
        let asked = format!("vultr / {}", self.models.planner);
        store::run_seen(&self.pool, run_id, Some(&asked), prompt_chars).await?;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let planner = self.runner.run(
            AgentJob {
                model: self.models.planner.clone(),
                prompt,
                ..AgentJob::default()
            },
            tx,
        );
        let drain = async { while rx.recv().await.is_some() {} };
        let (answer, ()) = tokio::join!(planner, drain);
        let (answer, seen) = match answer {
            Ok(out) => {
                let seen = seen_of(&out, &self.models.planner);
                (out.text, Some(seen))
            }
            Err(e) => {
                tracing::warn!(error = %e, "concepts: the planner failed; using plain directions");
                (String::new(), None)
            }
        };
        store::run_seen(&self.pool, run_id, seen.as_deref(), prompt_chars).await?;
        let (directions, from_planner) = prompts::parse_directions(&answer, &facts);

        // Three designers, in parallel, each in its own room.
        let mut concept_ids = Vec::new();
        for (i, d) in directions.iter().enumerate() {
            let concept_id = format!("cpt_{}", uuid::Uuid::now_v7().simple());
            let n = i64::try_from(i + 1)?;
            store::insert_concept(
                &self.pool,
                &concept_id,
                run_id,
                site_id,
                n,
                &d.name,
                &d.sentence,
                &self.models.worker,
            )
            .await?;
            concept_ids.push(concept_id);
        }
        store::set_run_state(&self.pool, run_id, "designing", "", None).await?;
        let planned = if from_planner == 3 {
            format!(
                "The planner chose three directions in {} seconds. Each designer is building its page now.",
                started.elapsed().as_secs()
            )
        } else {
            "The planner didn't give three clear directions, so plain ones fill the gap. Each designer is building its page now.".to_owned()
        };
        live.update(true, |s| {
            s.state = "designing".into();
            s.words = planned;
            s.agents = directions
                .iter()
                .zip(&concept_ids)
                .enumerate()
                .map(|(i, (d, id))| AgentView {
                    n: i64::try_from(i + 1).unwrap_or(0),
                    concept_id: id.clone(),
                    name: d.name.clone(),
                    sentence: d.sentence.clone(),
                    status: "waiting".into(),
                    words: "Waiting to start.".into(),
                    ..AgentView::default()
                })
                .collect();
        });
        let policy = Policy::for_host(&facts.domain);
        let designers = directions
            .iter()
            .zip(&concept_ids)
            .enumerate()
            .map(|(i, (d, id))| {
                self.design(
                    site_id,
                    run_id,
                    i64::try_from(i + 1).unwrap_or(0),
                    id,
                    d,
                    &facts,
                    &policy,
                    live,
                )
            });
        let results = futures::future::join_all(designers).await;
        let finished = results.iter().filter(|ok| **ok).count();
        let total = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let secs = started.elapsed().as_secs();
        let (state, words) = match finished {
            3 => (
                "done",
                format!(
                    "All three are ready, {secs} seconds from start. Pick the one that feels like your business, or tell a designer what to change."
                ),
            ),
            0 => (
                "failed",
                "None of the three designers finished a page. Try three more.".to_owned(),
            ),
            n => (
                "done",
                format!(
                    "{n} of 3 are ready, {secs} seconds from start. Pick one, or try three more."
                ),
            ),
        };
        store::set_run_state(
            &self.pool,
            run_id,
            state,
            if state == "failed" { &words } else { "" },
            Some(total),
        )
        .await?;
        live.update(true, |s| {
            s.state = state.into();
            s.words = words;
            s.seconds = secs;
        });
        Ok(())
    }

    /// Runs one agent turn in `room`, following its events into the live view and the tally.
    #[allow(clippy::too_many_arguments)]
    async fn turn(
        &self,
        concept_id: &str,
        n: i64,
        room: &Path,
        prompt: String,
        live: &Arc<LiveRun>,
        tally: &mut Tally,
        writing_status: &str,
    ) -> anyhow::Result<AgentOutput> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        // The first build gets 420 seconds; fixes and the owner's changes get 300.
        let (kind, cap) = match writing_status {
            "writing" => ("build", BUILD_SECS),
            "fixing" => ("fix", crate::AGENT_SECS),
            _ => ("owner", crate::AGENT_SECS),
        };
        // Recorded when it starts: a call stopped by the cap never sends its result.
        tally.calls.push(serde_json::json!({
            "provider": "vultr", "model": self.models.worker, "kind": kind, "ms": null
        }));
        let call_index = tally.calls.len() - 1;
        let _ = store::concept_calls(&self.pool, concept_id, &serde_json::json!(tally.calls)).await;
        let job = AgentJob {
            model: self.models.worker.clone(),
            prompt,
            files: prompts::reference_files(),
            room: Some(room.to_path_buf()),
            write: true,
            timeout: Some(std::time::Duration::from_secs(cap)),
        };
        let run = self.runner.run(job, tx);
        let pool = self.pool.clone();
        let follow = async {
            while let Some(event) = rx.recv().await {
                match event {
                    StreamEvent::Tool { name } => {
                        let words = match name.as_str() {
                            "fs_read" => "Reading the files in its room.".to_owned(),
                            "fs_write" => "Writing index.html.".to_owned(),
                            other => format!("Tried to use {other}."),
                        };
                        live.agent(n, true, |a| {
                            if a.status == "waiting" || a.status == "reading" {
                                a.status = if name == "fs_write" {
                                    writing_status.into()
                                } else {
                                    "reading".into()
                                };
                            }
                            a.words = words;
                        });
                    }
                    StreamEvent::ToolDone { name, success } => {
                        if !success {
                            *tally.refused.entry(name.clone()).or_default() += 1;
                            let _ = store::concept_refused(&pool, concept_id, &tally.refused).await;
                            let refused = tally.refused.clone();
                            live.agent(n, true, |a| {
                                a.refused = refused;
                                a.words = format!("{name} was refused: it isn't allowed here.");
                            });
                        } else if name == "fs_write" {
                            tally.writes += 1;
                            let page = read_page(room).unwrap_or_default();
                            let chars = page.chars().count();
                            let tail = tail_of(&page, TAIL_CHARS);
                            let writes = tally.writes;
                            let _ = store::concept_progress(
                                &pool,
                                concept_id,
                                i64::try_from(chars).unwrap_or(0),
                                writes,
                            )
                            .await;
                            live.agent(n, true, |a| {
                                a.status = writing_status.into();
                                a.chars = chars;
                                a.tail = tail;
                                a.writes = writes;
                                a.words = format!(
                                    "Writing index.html: {chars} characters, {writes} {} so far.",
                                    if writes == 1 { "save" } else { "saves" }
                                );
                            });
                        }
                    }
                    StreamEvent::Agent(_) | StreamEvent::Started => live.agent(n, true, |a| {
                        if a.status == "waiting" {
                            a.status = "reading".into();
                            a.words = "Thinking about the direction.".into();
                        }
                    }),
                    _ => {}
                }
            }
        };
        let (out, ()) = tokio::join!(run, follow);
        if let Ok(out) = &out {
            if let Some(call) = tally.calls.get_mut(call_index) {
                *call = serde_json::json!({
                    "provider": out.provider.clone().unwrap_or_else(|| "vultr".into()),
                    "model": out.model.clone().unwrap_or_else(|| self.models.worker.clone()),
                    "kind": kind,
                    "ms": out.duration_ms,
                });
            }
            tally.key_checked_files = tally.key_checked_files.max(out.key_checked_files);
        }
        let _ = store::concept_calls(&self.pool, concept_id, &serde_json::json!(tally.calls)).await;
        out
    }

    /// One designer: builds, checks, fixes, cleans and stores its page. `true` when it finished.
    #[allow(clippy::too_many_arguments)]
    async fn design(
        &self,
        site_id: &str,
        run_id: &str,
        n: i64,
        concept_id: &str,
        direction: &Direction,
        facts: &SiteFacts,
        policy: &Policy,
        live: &Arc<LiveRun>,
    ) -> bool {
        let started = Instant::now();
        let _ = store::concept_started(&self.pool, concept_id).await;
        live.agent(n, true, |a| {
            a.status = "reading".into();
            a.words = "Starting.".into();
        });
        let Some(room) = self.room(concept_id) else {
            return false;
        };
        let mut tally = Tally::default();
        let prompt = prompts::designer_prompt(facts, direction);
        let _ = store::run_seen(
            &self.pool,
            run_id,
            None,
            i64::try_from(prompt.chars().count()).unwrap_or(i64::MAX),
        )
        .await;
        let first = self
            .turn(concept_id, n, &room, prompt, live, &mut tally, "writing")
            .await;
        let mut page = match &first {
            Ok(out) => read_page(&room)
                .or_else(|| extract_html(&out.text).map(str::to_owned))
                .unwrap_or_default(),
            Err(e) => {
                tracing::warn!(error = %e, n, "concepts: a designer failed");
                read_page(&room).unwrap_or_default()
            }
        };
        let rounds = self
            .check_and_fix(
                concept_id,
                n,
                &room,
                &mut page,
                facts,
                live,
                &mut tally,
                check::MAX_ROUNDS,
            )
            .await;
        let missing: Vec<&'static str> =
            check::missing(&page, facts).iter().map(|m| m.key).collect();
        let ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let secs = started.elapsed().as_secs();
        if extract_html(&page).is_none() {
            let note = "This one didn't finish a page.";
            let _ = store::concept_finished(
                &self.pool,
                concept_id,
                &store::Finished {
                    note,
                    duration_ms: ms,
                    rounds,
                    missing,
                    key_checked_files: i64::try_from(tally.key_checked_files).unwrap_or(0),
                    ..store::Finished::default()
                },
            )
            .await;
            live.agent(n, true, |a| {
                a.status = "failed".into();
                a.tail.clear();
                a.seconds = secs;
                a.words = note.into();
            });
            return false;
        }
        match self
            .store_page(
                site_id, n, direction, &page, policy, &tally, rounds, &missing, 0,
            )
            .await
        {
            Ok((digest, chars, receipt)) => {
                let last = tally.calls.last();
                let field = |k: &str| {
                    last.and_then(|c| c.get(k))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                };
                let (provider, model) = (field("provider"), field("model"));
                let _ = store::concept_finished(
                    &self.pool,
                    concept_id,
                    &store::Finished {
                        page: Some((&digest, i64::try_from(chars).unwrap_or(0))),
                        model: model.as_deref(),
                        provider: provider.as_deref(),
                        duration_ms: ms,
                        rounds,
                        missing: missing.clone(),
                        key_checked_files: i64::try_from(tally.key_checked_files).unwrap_or(0),
                        receipt: receipt.as_deref(),
                        ..store::Finished::default()
                    },
                )
                .await;
                let missing: Vec<String> = missing.iter().map(|m| (*m).to_owned()).collect();
                live.agent(n, true, |a| {
                    a.status = "done".into();
                    a.tail.clear();
                    a.chars = chars;
                    a.seconds = secs;
                    a.rounds = rounds;
                    a.digest = short(&digest);
                    a.words = done_words(secs, i64::try_from(chars).unwrap_or(0), &missing);
                    a.missing = missing_words(&missing);
                });
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "concepts: storing a design");
                let note = "The page was written but couldn't be saved. Try three more.";
                let _ = store::concept_finished(
                    &self.pool,
                    concept_id,
                    &store::Finished {
                        note,
                        duration_ms: ms,
                        ..store::Finished::default()
                    },
                )
                .await;
                live.agent(n, true, |a| {
                    a.status = "failed".into();
                    a.tail.clear();
                    a.words = note.into();
                });
                false
            }
        }
    }

    /// Checks `page` and sends what's missing back to the designer, up to `max_rounds` times.
    /// Returns the rounds used; `page` ends as the room's latest file.
    #[allow(clippy::too_many_arguments)]
    async fn check_and_fix(
        &self,
        concept_id: &str,
        n: i64,
        room: &Path,
        page: &mut String,
        facts: &SiteFacts,
        live: &Arc<LiveRun>,
        tally: &mut Tally,
        max_rounds: i64,
    ) -> i64 {
        let mut rounds = 0;
        loop {
            live.agent(n, true, |a| {
                a.status = "checking".into();
                a.words = "Checking the page for the Agentrix basics.".into();
            });
            let missing = check::missing(page, facts);
            if missing.is_empty() || rounds >= max_rounds || extract_html(page).is_none() {
                return rounds;
            }
            rounds += 1;
            let words: Vec<String> = missing.iter().map(|m| m.missing.to_owned()).collect();
            let list = words.join("; ");
            live.agent(n, true, |a| {
                a.status = "fixing".into();
                a.rounds = rounds;
                a.missing = words.clone();
                a.words = format!(
                    "Fixing what the check found missing (round {rounds} of {max_rounds}): {list}."
                );
            });
            let prompt = prompts::revise_prompt(&check::fix_request(&missing), false);
            if let Err(e) = self
                .turn(concept_id, n, room, prompt, live, tally, "fixing")
                .await
            {
                tracing::warn!(error = %e, n, "concepts: a fix round failed");
            }
            if let Some(p) = read_page(room) {
                *page = p;
            }
        }
    }

    /// Cleans and stores a page, and signs its receipt. Returns the digest, length and receipt.
    #[allow(clippy::too_many_arguments)]
    async fn store_page(
        &self,
        site_id: &str,
        n: i64,
        direction: &Direction,
        page: &str,
        policy: &Policy,
        tally: &Tally,
        rounds: i64,
        missing: &[&'static str],
        guided: i64,
    ) -> anyhow::Result<(String, usize, Option<String>)> {
        let clean = sanitize(extract_html(page).unwrap_or(page), policy);
        let chars = clean.chars().count();
        let digest = self.blobs.put(Bytes::from(clean.clone())).await?;
        let detail = serde_json::json!({
            "agent": n,
            "direction": direction.name,
            "sha256": digest,
            "bytes": clean.len(),
            "removed_by_sanitiser": page.len().saturating_sub(clean.len()),
            "checks_missing": missing,
            "fix_rounds": rounds,
            "guided": guided,
            "calls": tally.calls,
            "refused": tally.refused,
            "writes": tally.writes,
        });
        let receipt = crate::receipts::issue(
            &self.receipts,
            "concept.page",
            site_id,
            Some(&digest),
            detail,
        )
        .await;
        Ok((digest, chars, receipt))
    }

    /// The owner's change to one finished design: the designer reads its own `index.html` and
    /// changes only what was asked, then the page is checked (one fix round), cleaned, stored
    /// and receipted again. Runs in the background; the live view shows "Revising".
    pub async fn revise(
        &self,
        site_id: &str,
        concept: &ConceptRow,
        change_id: &str,
        words: &str,
    ) -> anyhow::Result<()> {
        let live = self
            .live(site_id, &concept.run_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("no live view for {}", concept.run_id))?;
        let room = self
            .room(&concept.concept_id)
            .filter(|r| r.join("index.html").is_file())
            .ok_or_else(|| anyhow::anyhow!("the room for {} is gone", concept.concept_id))?;
        let n = concept.n;
        let quoted: String = words.chars().take(80).collect();
        live.update(true, |s| {
            s.state = "revising".into();
            if let Some(a) = s.agents.iter_mut().find(|a| a.n == n) {
                a.busy = true;
                a.status = "revising".into();
                a.words = format!("Revising for you: \u{201c}{quoted}\u{201d}");
            }
        });
        let engine = self.clone();
        let site = site_id.to_owned();
        let concept = concept.clone();
        let change = change_id.to_owned();
        let words = words.to_owned();
        tokio::spawn(async move {
            let started = Instant::now();
            let facts = site_facts(&engine.pool, &engine.blobs, &site)
                .await
                .unwrap_or_default();
            let mut tally = Tally {
                refused: serde_json::from_str(&concept.refused).unwrap_or_default(),
                writes: concept.writes,
                calls: serde_json::from_str(&concept.calls).unwrap_or_default(),
                ..Tally::default()
            };
            let prompt = prompts::revise_prompt(&words, true);
            let turn = engine
                .turn(
                    &concept.concept_id,
                    n,
                    &room,
                    prompt,
                    &live,
                    &mut tally,
                    "revising",
                )
                .await;
            let mut page = read_page(&room).unwrap_or_default();
            let rounds = engine
                .check_and_fix(
                    &concept.concept_id,
                    n,
                    &room,
                    &mut page,
                    &facts,
                    &live,
                    &mut tally,
                    1,
                )
                .await;
            let missing: Vec<&'static str> = check::missing(&page, &facts)
                .iter()
                .map(|m| m.key)
                .collect();
            let guided = store::changes_by_concept(&engine.pool, &concept.run_id)
                .await
                .unwrap_or_default()
                .iter()
                .find(|(c, _)| *c == concept.concept_id)
                .map_or(0, |(_, k)| *k)
                + 1;
            let direction = Direction {
                name: concept.name.clone(),
                sentence: concept.direction.clone(),
                details: serde_json::Map::new(),
            };
            let secs = started.elapsed().as_secs();
            let stored = if turn.is_ok() && extract_html(&page).is_some() {
                engine
                    .store_page(
                        &site,
                        n,
                        &direction,
                        &page,
                        &Policy::for_host(&facts.domain),
                        &tally,
                        rounds,
                        &missing,
                        guided,
                    )
                    .await
                    .ok()
            } else {
                None
            };
            let (done, note) = match &stored {
                Some((digest, chars, receipt)) => {
                    let _ = store::concept_revised(
                        &engine.pool,
                        &concept.concept_id,
                        (digest, i64::try_from(*chars).unwrap_or(0)),
                        rounds,
                        &missing,
                        receipt.as_deref(),
                    )
                    .await;
                    (true, String::new())
                }
                None => (
                    false,
                    "The change didn't finish, so the page is as it was. Try sending it again."
                        .to_owned(),
                ),
            };
            let _ = store::change_finished(&engine.pool, &change, done, &note).await;
            let missing_owned: Vec<String> = missing.iter().map(|m| (*m).to_owned()).collect();
            live.update(true, |s| {
                s.state = "done".into();
                s.words = if done {
                    format!("Changed for you in {secs} seconds. Pick the one that feels like your business, or ask for another change.")
                } else {
                    note.clone()
                };
                if let Some(a) = s.agents.iter_mut().find(|a| a.n == n) {
                    a.busy = false;
                    a.status = "done".into();
                    a.tail.clear();
                    a.refused = tally.refused.clone();
                    a.writes = tally.writes;
                    a.rounds += rounds;
                    if let Some((digest, chars, _)) = &stored {
                        a.digest = short(digest);
                        a.chars = *chars;
                        a.missing = missing_words(&missing_owned);
                        a.words = format!(
                            "Changed in {secs} seconds, {chars} characters. {}",
                            if missing_owned.is_empty() {
                                "Checked: all the basics are there."
                            } else {
                                "Some basics are still missing."
                            }
                        );
                    } else {
                        a.words = note.clone();
                    }
                }
            });
        });
        Ok(())
    }
}

/// `room/index.html`, if it's there and not too big.
fn read_page(room: &Path) -> Option<String> {
    let file = room.join("index.html");
    let meta = std::fs::metadata(&file).ok()?;
    if !meta.is_file() || meta.len() > MAX_PAGE_BYTES {
        return None;
    }
    let bytes = std::fs::read(file).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// `vultr / glm-5.3`, from what Zavora reported.
fn seen_of(out: &AgentOutput, asked: &str) -> String {
    format!(
        "{} / {}",
        out.provider.as_deref().unwrap_or("vultr"),
        out.model.as_deref().unwrap_or(asked)
    )
}

#[cfg(test)]
mod tail_tests {
    #[test]
    fn the_tail_keeps_the_last_characters() {
        assert_eq!(super::tail_of("abcdef", 3), "def");
        assert_eq!(super::tail_of("ab", 3), "ab");
        assert_eq!(super::tail_of("ünï©ödé", 2), "dé");
        assert_eq!(super::tail_of("", 3), "");
        assert_eq!(super::short("sha256:0123456789abcdef"), "0123456789ab");
    }
}
