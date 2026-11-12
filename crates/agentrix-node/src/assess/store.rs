//! Where assessments, their UI events and their findings live, and the live fan-out to SSE.
//!
//! [`AssessStore`] is the persistence seam: SQLite through the data lane's `db/` in
//! production (`assessments`, `ui_events` with stream `assess:<id>`, `findings`, and
//! `sites::create_or_get`), [`MemoryStore`] in tests. [`Hub`] holds each running
//! assessment's live state and broadcast channel; [`Emitter`] is the one writer of an
//! assessment's events, so their `seq` is gap-free and in order.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use agentrix_types::ui::{FrameEvent, StepEvent, TickEvent};
use agentrix_types::{AssessmentId, Finding, SiteId, UiEnvelope, UiEvent};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::report::ReportDoc;

/// Where an assessment is (the `assessments.state` column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssessmentState {
    /// Reading the site.
    Running,
    /// The report is ready.
    ReportReady,
    /// The site could not be read, or asked not to be (the report says which).
    Unreachable,
    /// The check itself failed (for example, no sealed room).
    Failed,
}

impl AssessmentState {
    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::ReportReady => "report-ready",
            Self::Unreachable => "unreachable",
            Self::Failed => "failed",
        }
    }

    /// Parses the stored form.
    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::Running,
            Self::ReportReady,
            Self::Unreachable,
            Self::Failed,
        ]
        .into_iter()
        .find(|v| v.as_str() == s)
    }
}

/// One assessment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assessment {
    /// Its id.
    pub id: AssessmentId,
    /// The site.
    pub site: SiteId,
    /// The start URL.
    pub url: String,
    /// The site's host.
    pub host: String,
    /// Whether it is Zavora's fixture.
    pub fixture: bool,
    /// Where it is.
    pub state: AssessmentState,
    /// When it started.
    pub created_at: DateTime<Utc>,
    /// When it finished.
    pub finished_at: Option<DateTime<Utc>>,
    /// The report, once finished.
    pub report: Option<ReportDoc>,
}

/// The SSE stream name for an assessment.
pub fn stream_of(id: &AssessmentId) -> String {
    format!("assess:{id}")
}

/// Persistence for assessments.
#[async_trait]
pub trait AssessStore: Send + Sync + 'static {
    /// The site for `domain`, created with no owner if new.
    async fn site_for(&self, domain: &str, origin: &str, fixture: bool) -> anyhow::Result<SiteId>;
    /// Records a new assessment.
    async fn create(&self, a: &Assessment) -> anyhow::Result<()>;
    /// One assessment.
    async fn get(&self, id: &AssessmentId) -> anyhow::Result<Option<Assessment>>;
    /// The newest assessment of `host` started after `since`, if any.
    async fn latest_for(
        &self,
        host: &str,
        since: DateTime<Utc>,
    ) -> anyhow::Result<Option<Assessment>>;
    /// Records how it ended.
    async fn finish(
        &self,
        id: &AssessmentId,
        state: AssessmentState,
        report: &ReportDoc,
        at: DateTime<Utc>,
    ) -> anyhow::Result<()>;
    /// Appends one UI event to `stream`.
    async fn append(
        &self,
        stream: &str,
        site: &SiteId,
        env: &UiEnvelope,
        at: DateTime<Utc>,
    ) -> anyhow::Result<()>;
    /// Events of `stream` with `seq > after`, in order.
    async fn events_after(&self, stream: &str, after: u64) -> anyhow::Result<Vec<UiEnvelope>>;
    /// Stores the assessment's findings.
    async fn save_findings(
        &self,
        assessment: &AssessmentId,
        findings: &[Finding],
    ) -> anyhow::Result<()>;
}

/// Everything in memory.
#[derive(Debug, Clone, Default)]
pub struct MemoryStore {
    inner: Arc<Mutex<Mem>>,
}

#[derive(Debug, Default)]
struct Mem {
    sites: HashMap<String, SiteId>,
    assessments: HashMap<AssessmentId, Assessment>,
    events: HashMap<String, Vec<UiEnvelope>>,
    findings: HashMap<AssessmentId, Vec<Finding>>,
}

impl MemoryStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, Mem> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The findings stored for an assessment.
    pub fn findings(&self, id: &AssessmentId) -> Vec<Finding> {
        self.lock().findings.get(id).cloned().unwrap_or_default()
    }
}

#[async_trait]
impl AssessStore for MemoryStore {
    async fn site_for(
        &self,
        domain: &str,
        _origin: &str,
        _fixture: bool,
    ) -> anyhow::Result<SiteId> {
        Ok(self
            .lock()
            .sites
            .entry(domain.to_ascii_lowercase())
            .or_insert_with(SiteId::generate)
            .clone())
    }

    async fn create(&self, a: &Assessment) -> anyhow::Result<()> {
        self.lock().assessments.insert(a.id.clone(), a.clone());
        Ok(())
    }

    async fn get(&self, id: &AssessmentId) -> anyhow::Result<Option<Assessment>> {
        Ok(self.lock().assessments.get(id).cloned())
    }

    async fn latest_for(
        &self,
        host: &str,
        since: DateTime<Utc>,
    ) -> anyhow::Result<Option<Assessment>> {
        Ok(self
            .lock()
            .assessments
            .values()
            .filter(|a| a.host.eq_ignore_ascii_case(host) && a.created_at > since)
            .max_by_key(|a| a.created_at)
            .cloned())
    }

    async fn finish(
        &self,
        id: &AssessmentId,
        state: AssessmentState,
        report: &ReportDoc,
        at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        if let Some(a) = self.lock().assessments.get_mut(id) {
            a.state = state;
            a.report = Some(report.clone());
            a.finished_at = Some(at);
        }
        Ok(())
    }

    async fn append(
        &self,
        stream: &str,
        _site: &SiteId,
        env: &UiEnvelope,
        _at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        self.lock()
            .events
            .entry(stream.to_owned())
            .or_default()
            .push(env.clone());
        Ok(())
    }

    async fn events_after(&self, stream: &str, after: u64) -> anyhow::Result<Vec<UiEnvelope>> {
        Ok(self
            .lock()
            .events
            .get(stream)
            .map(|v| v.iter().filter(|e| e.seq > after).cloned().collect())
            .unwrap_or_default())
    }

    async fn save_findings(
        &self,
        assessment: &AssessmentId,
        findings: &[Finding],
    ) -> anyhow::Result<()> {
        self.lock()
            .findings
            .insert(assessment.clone(), findings.to_vec());
        Ok(())
    }
}

/// What a running assessment shows right now, for the page and for `snapshot`.
#[derive(Debug, Clone)]
pub struct LiveState {
    /// The last `seq` sent.
    pub seq: u64,
    /// The latest totals.
    pub tick: TickEvent,
    /// The latest frame.
    pub frame: Option<FrameEvent>,
    /// The assess step's latest event.
    pub step: Option<StepEvent>,
    /// Pages done.
    pub pages_done: u32,
    /// Pages planned.
    pub pages_of: u32,
    /// Whether `done` was sent.
    pub done: bool,
}

/// One running assessment's live state and channel.
#[derive(Debug)]
pub struct Live {
    /// New events, for SSE subscribers.
    pub tx: broadcast::Sender<UiEnvelope>,
    state: Mutex<LiveState>,
}

impl Live {
    /// A fresh live state.
    pub fn new(at: DateTime<Utc>) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            tx,
            state: Mutex::new(LiveState {
                seq: 0,
                tick: TickEvent {
                    job: None,
                    hosts: 0,
                    requests: 0,
                    bytes: 0,
                    refusals: 0,
                    at,
                },
                frame: None,
                step: None,
                pages_done: 0,
                pages_of: 0,
                done: false,
            }),
        }
    }

    /// A copy of the current state.
    pub fn state(&self) -> LiveState {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Live assessments by id.
#[derive(Debug, Clone, Default)]
pub struct Hub {
    map: Arc<Mutex<HashMap<AssessmentId, Arc<Live>>>>,
}

impl Hub {
    /// Registers a new live assessment.
    pub fn start(&self, id: &AssessmentId, at: DateTime<Utc>) -> Arc<Live> {
        let live = Arc::new(Live::new(at));
        self.map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone(), live.clone());
        live
    }

    /// A live assessment.
    pub fn get(&self, id: &AssessmentId) -> Option<Arc<Live>> {
        self.map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    /// Forgets a finished assessment (its events stay in the store).
    pub fn forget(&self, id: &AssessmentId) {
        self.map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
    }
}

/// The one writer of an assessment's events.
pub struct Emitter {
    store: Arc<dyn AssessStore>,
    live: Arc<Live>,
    stream: String,
    site: SiteId,
}

impl std::fmt::Debug for Emitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Emitter")
            .field("stream", &self.stream)
            .finish_non_exhaustive()
    }
}

impl Emitter {
    /// An emitter for `stream`.
    pub fn new(store: Arc<dyn AssessStore>, live: Arc<Live>, stream: String, site: SiteId) -> Self {
        Self {
            store,
            live,
            stream,
            site,
        }
    }

    /// The live state it updates.
    pub fn live(&self) -> &Arc<Live> {
        &self.live
    }

    /// Persists and broadcasts one event.
    pub async fn emit(&self, event: UiEvent) {
        let env = {
            let mut st = self
                .live
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            st.seq += 1;
            match &event {
                UiEvent::ReceiptTick(t) => st.tick = t.clone(),
                UiEvent::Frame(f) => st.frame = Some(f.clone()),
                UiEvent::Step(s) => st.step = Some(s.clone()),
                UiEvent::Page(p) => {
                    st.pages_done = st.pages_done.max(p.n);
                    st.pages_of = st.pages_of.max(p.of);
                }
                UiEvent::Done(_) => st.done = true,
                _ => {}
            }
            UiEnvelope { seq: st.seq, event }
        };
        if let Err(e) = self
            .store
            .append(&self.stream, &self.site, &env, Utc::now())
            .await
        {
            tracing::warn!(stream = %self.stream, error = %e, "could not persist a UI event");
        }
        let _ = self.live.tx.send(env);
    }

    /// Records the planned page count without an event.
    pub fn plan_pages(&self, of: u32) {
        let mut st = self
            .live
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        st.pages_of = st.pages_of.max(of);
    }
}
