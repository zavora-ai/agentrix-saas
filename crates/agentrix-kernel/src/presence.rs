//! Presence: what each WebAgent is doing now, from real events only (ADR-031, 06 §4.3).
//!
//! A mark changes state when something happened: an Ask turn started or streamed, a Move step
//! ran, a card started waiting on the owner, the brief ran, the voice played. Nothing here runs
//! on a timer, polls, or calls a model; a sleeping WebAgent costs nothing (05 §8.2).
//!
//! - [`Presence`] is the hub: per site, the last frame for each WebAgent and desk, and a
//!   broadcast to every open stream. [`global`] is the node's one hub.
//! - The `on_*` methods turn the first release's events into frames: Ask frames ([`Presence::on_ask`]),
//!   the Move's UI events ([`Presence::on_move`]), decision cards
//!   ([`Presence::on_decision`]), brief runs ([`Presence::on_brief`]) and the voice level
//!   ([`Presence::on_voice`]). Node's wiring calls them where those events already happen.
//! - [`baseline`] reads what is recorded when a stream opens (cards waiting, a Move running),
//!   so a stream never starts from a guess.
//! - agentrix.website's own team is the one exception to "nothing polls": its sources are other
//!   lanes' tables with no events, so [`crate::home::watch`] reads them every few seconds while
//!   a stream is open on that site, and stops when the last one closes.
//! - Members with no frame yet show their resting place: public desks idle (awake, answering
//!   when asked), back-office WebAgents asleep (no lease, no model call between wakes).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use agentrix_marks::{Audience, MarkState, WordsContext, words};
use agentrix_types::ui::StepState;
use agentrix_types::{CardKind, UiEvent};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::broadcast;

use crate::roster::{Roster, Status};
use crate::spec::Partition;

/// One change of state, for one WebAgent or desk of one site.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresenceFrame {
    /// The site.
    pub site: String,
    /// The WebAgent's id (`mover`) or the desk's (`concierge`, `desk-cakes`): the value of
    /// `<wa-mark webagent="…">`.
    pub webagent: String,
    /// The state its mark shows.
    pub state: MarkState,
    /// The running tool's label while working ("Reading this site").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The voice level, 0 to 1, while speaking. From the TTS stream, never simulated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<f32>,
    /// The owner's status line for this state (the console's `role="status"` text).
    pub words: String,
    /// Per site, increasing.
    pub seq: u64,
    /// When, RFC 3339.
    pub at: String,
}

#[derive(Default)]
struct SiteState {
    seq: u64,
    last: BTreeMap<String, PresenceFrame>,
    /// Cards waiting on the owner, by decision id, with the WebAgent that raised each.
    waiting: BTreeMap<String, String>,
    /// Which of those are v2 cards (W5's `decision_cards_v2`), kept by [`Presence::sync_cards`].
    cards: BTreeSet<String>,
    /// Whether the brief is being written now.
    briefing: bool,
    /// What the Chief is doing for the owner now, when it is answering a question.
    chief_label: Option<String>,
    /// Whether [`baseline`] has read this site's records.
    seeded: bool,
    tx: Option<broadcast::Sender<PresenceFrame>>,
}

impl SiteState {
    fn tx(&mut self) -> broadcast::Sender<PresenceFrame> {
        self.tx
            .get_or_insert_with(|| broadcast::channel(256).0)
            .clone()
    }
}

/// The presence hub.
#[derive(Clone, Default)]
pub struct Presence {
    sites: Arc<Mutex<HashMap<String, SiteState>>>,
    /// Sites whose records a watcher is reading now ([`crate::home::watch`]).
    watching: Arc<Mutex<BTreeSet<String>>>,
}

/// The node's one hub, shared by the kernel's routes and node's wiring.
pub fn global() -> &'static Presence {
    static HUB: OnceLock<Presence> = OnceLock::new();
    HUB.get_or_init(Presence::default)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The WebAgent behind a decision card.
pub fn raised_by(kind: CardKind) -> &'static str {
    match kind {
        CardKind::GoLive | CardKind::QuarantineRestore => "mover",
        // The first release holds orders that AI customers place through the agent door.
        CardKind::OrderHeld => "agent-desk",
    }
}

impl Presence {
    /// A hub of its own (tests, or a node that wants one per process).
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a member's state. A frame that changes nothing (same state, label and level) is
    /// dropped, so marks never restart their motion for no reason.
    pub fn publish(
        &self,
        site: &str,
        webagent: &str,
        state: MarkState,
        label: Option<&str>,
        level: Option<f32>,
    ) -> Option<PresenceFrame> {
        let mut sites = self.sites.lock().expect("presence lock");
        let st = sites.entry(site.to_string()).or_default();
        Self::set(st, site, webagent, state, label, level)
    }

    fn set(
        st: &mut SiteState,
        site: &str,
        webagent: &str,
        state: MarkState,
        label: Option<&str>,
        level: Option<f32>,
    ) -> Option<PresenceFrame> {
        if let Some(prev) = st.last.get(webagent) {
            if prev.state == state && prev.label.as_deref() == label && prev.level == level {
                return None;
            }
        }
        st.seq += 1;
        let frame = PresenceFrame {
            site: site.to_string(),
            webagent: webagent.to_string(),
            state,
            label: label.map(str::to_string),
            level,
            words: words(
                state,
                Audience::Owner,
                &WordsContext {
                    label,
                    ..Default::default()
                },
            ),
            seq: st.seq,
            at: now(),
        };
        st.last.insert(webagent.to_string(), frame.clone());
        let _ = st.tx().send(frame.clone());
        Some(frame)
    }

    /// The frames each member last had, and a receiver for every change after them.
    pub fn subscribe(
        &self,
        site: &str,
    ) -> (Vec<PresenceFrame>, broadcast::Receiver<PresenceFrame>) {
        let mut sites = self.sites.lock().expect("presence lock");
        let st = sites.entry(site.to_string()).or_default();
        let rx = st.tx().subscribe();
        (st.last.values().cloned().collect(), rx)
    }

    /// The last frame for one member, if it has had one.
    pub fn current(&self, site: &str, webagent: &str) -> Option<PresenceFrame> {
        let sites = self.sites.lock().expect("presence lock");
        sites.get(site)?.last.get(webagent).cloned()
    }

    /// One frame of an Ask turn on `desk` (the first release's `/ask` SSE: `ready`, `awp.presence`,
    /// `delta`, `reply`, `error`, `done`). The desk is `concierge` for the front desk.
    pub fn on_ask(&self, site: &str, desk: &str, event: &str, data: &serde_json::Value) {
        match event {
            "awp.presence" => {
                let state = data["state"]
                    .as_str()
                    .and_then(MarkState::from_key)
                    .unwrap_or(MarkState::Working);
                self.publish(site, desk, state, data["label"].as_str(), None);
            }
            "done" => {
                let resting = self
                    .current(site, desk)
                    .is_some_and(|f| f.state == MarkState::Resting);
                if !resting {
                    let waiting = self.held_by(site, desk);
                    let state = if waiting {
                        MarkState::Waiting
                    } else {
                        MarkState::Idle
                    };
                    self.publish(site, desk, state, None, None);
                }
            }
            _ => {}
        }
    }

    /// One of the Move's UI events: the Mover works through its steps and waits on the owner
    /// at the go-live card.
    pub fn on_move(&self, site: &str, event: &UiEvent) {
        match event {
            UiEvent::Step(s) => match s.state {
                StepState::Running => {
                    self.publish(site, "mover", MarkState::Working, Some(&s.summary), None);
                }
                StepState::Waiting => {
                    self.publish(site, "mover", MarkState::Waiting, None, None);
                }
                StepState::Failed => {
                    self.publish(site, "mover", MarkState::Paused, Some(&s.summary), None);
                }
                StepState::Pending | StepState::Done => {}
            },
            UiEvent::Card(c) => self.on_decision(site, c.decision_id.as_str(), c.kind, true),
            UiEvent::Done(_) => {
                self.publish(site, "mover", MarkState::Idle, None, None);
            }
            UiEvent::Error(e) if !e.retrying => {
                self.publish(site, "mover", MarkState::Paused, Some(&e.message), None);
            }
            _ => {}
        }
    }

    /// A decision card started waiting on the owner (`waiting`), or was decided or expired.
    /// The WebAgent that raised it shows the amber dot while it waits, and so does the Chief
    /// WebAgent while any delivered card waits (ADR-031, 07 C13).
    pub fn on_decision(&self, site: &str, decision_id: &str, kind: CardKind, waiting: bool) {
        let mut sites = self.sites.lock().expect("presence lock");
        let st = sites.entry(site.to_string()).or_default();
        let who = raised_by(kind).to_string();
        if waiting {
            st.waiting.insert(decision_id.to_string(), who.clone());
            Self::set(st, site, &who, MarkState::Waiting, None, None);
        } else if st.waiting.remove(decision_id).is_some() {
            let still = st.waiting.values().any(|w| *w == who);
            if !still {
                Self::set(st, site, &who, MarkState::Idle, None, None);
            }
        }
        Self::chief(st, site);
    }

    /// A v2 decision card (W5's `decision_cards_v2`) that `webagent` raised started waiting on
    /// the owner (`waiting`), or was decided. As with [`Presence::on_decision`], the raiser and
    /// the Chief show the amber dot while it waits; the raiser goes back to `rest` once none of
    /// its cards wait.
    pub fn on_card(
        &self,
        site: &str,
        card_id: &str,
        webagent: &str,
        waiting: bool,
        rest: MarkState,
    ) {
        let mut sites = self.sites.lock().expect("presence lock");
        let st = sites.entry(site.to_string()).or_default();
        if waiting {
            st.waiting.insert(card_id.to_string(), webagent.to_string());
            st.cards.insert(card_id.to_string());
            Self::set(st, site, webagent, MarkState::Waiting, None, None);
        } else if st.waiting.remove(card_id).is_some() {
            st.cards.remove(card_id);
            if !st.waiting.values().any(|w| w == webagent) {
                Self::set(st, site, webagent, rest, None, None);
            }
        }
        Self::chief(st, site);
    }

    /// Makes the v2 cards waiting on `site` exactly `waiting` (card id, raiser): a new card
    /// lights its raiser, a decided one lets it rest.
    pub fn sync_cards(&self, site: &str, waiting: &[(String, String)], rest: MarkState) {
        let gone: Vec<(String, String)> = {
            let mut sites = self.sites.lock().expect("presence lock");
            let st = sites.entry(site.to_string()).or_default();
            st.cards
                .iter()
                .filter(|id| !waiting.iter().any(|(w, _)| w == *id))
                .map(|id| (id.clone(), st.waiting.get(id).cloned().unwrap_or_default()))
                .collect()
        };
        for (id, who) in gone {
            self.on_card(site, &id, &who, false, rest);
        }
        for (id, who) in waiting {
            self.on_card(site, id, who, true, rest);
        }
    }

    /// Whether any stream is open on `site` now.
    pub fn watched(&self, site: &str) -> bool {
        let sites = self.sites.lock().expect("presence lock");
        sites
            .get(site)
            .and_then(|st| st.tx.as_ref())
            .is_some_and(|tx| tx.receiver_count() > 0)
    }

    /// Takes the one watcher's place for `site`; false when another already has it.
    pub(crate) fn claim_watch(&self, site: &str) -> bool {
        self.watching
            .lock()
            .expect("presence lock")
            .insert(site.to_string())
    }

    /// Gives the watcher's place back.
    pub(crate) fn release_watch(&self, site: &str) {
        self.watching.lock().expect("presence lock").remove(site);
    }

    /// The Chief WebAgent started (`running`) or finished writing the brief.
    pub fn on_brief(&self, site: &str, running: bool) {
        let mut sites = self.sites.lock().expect("presence lock");
        let st = sites.entry(site.to_string()).or_default();
        st.briefing = running;
        Self::chief(st, site);
    }

    /// The Chief WebAgent started (`Some(label)`) or finished (`None`) answering the owner.
    pub fn on_chief(&self, site: &str, label: Option<&str>) {
        let mut sites = self.sites.lock().expect("presence lock");
        let st = sites.entry(site.to_string()).or_default();
        st.chief_label = label.map(str::to_string);
        Self::chief(st, site);
    }

    /// The voice level while a WebAgent speaks (from W11's TTS stream); `None` when it stops.
    pub fn on_voice(&self, site: &str, webagent: &str, level: Option<f32>) {
        match level {
            Some(l) => {
                self.publish(
                    site,
                    webagent,
                    MarkState::Speaking,
                    None,
                    Some(l.clamp(0.0, 1.0)),
                );
            }
            None => {
                self.publish(site, webagent, MarkState::Idle, None, None);
            }
        }
    }

    fn held_by(&self, site: &str, webagent: &str) -> bool {
        let sites = self.sites.lock().expect("presence lock");
        sites
            .get(site)
            .is_some_and(|st| st.waiting.values().any(|w| w == webagent))
    }

    /// The Chief's own state: working while it writes the brief, waiting while a card waits,
    /// asleep otherwise.
    fn chief(st: &mut SiteState, site: &str) {
        let label_now = st.chief_label.clone();
        let (state, label) = if st.briefing {
            (MarkState::Working, Some("Writing your brief"))
        } else if let Some(l) = label_now.as_deref() {
            (MarkState::Working, Some(l))
        } else if !st.waiting.is_empty() {
            (MarkState::Waiting, None)
        } else {
            (MarkState::Asleep, None)
        };
        Self::set(st, site, "coordinator", state, label, None);
    }

    /// Whether [`baseline`] has already read this site.
    fn seeded(&self, site: &str) -> bool {
        let mut sites = self.sites.lock().expect("presence lock");
        let st = sites.entry(site.to_string()).or_default();
        std::mem::replace(&mut st.seeded, true)
    }

    /// Every member's state now: its last frame, or its resting place when it has had none.
    pub fn snapshot(&self, roster: &Roster) -> Vec<PresenceFrame> {
        let (frames, _) = self.subscribe(&roster.site);
        let mut by_id: BTreeMap<String, PresenceFrame> = frames
            .into_iter()
            .map(|f| (f.webagent.clone(), f))
            .collect();
        let mut ids: BTreeSet<(String, bool)> = BTreeSet::new();
        for m in roster.members.iter().filter(|m| m.status == Status::OnDuty) {
            if m.spec.partition == Partition::Public {
                for d in &m.spec.desks {
                    ids.insert((d.id.clone(), true));
                }
            } else {
                ids.insert((m.spec.id.clone(), false));
            }
        }
        let mut out = Vec::new();
        for (id, public) in ids {
            let frame = by_id.remove(&id).unwrap_or_else(|| {
                let state = if public {
                    MarkState::Idle
                } else {
                    MarkState::Asleep
                };
                PresenceFrame {
                    site: roster.site.clone(),
                    webagent: id.clone(),
                    state,
                    label: None,
                    level: None,
                    words: words(state, Audience::Owner, &WordsContext::default()),
                    seq: 0,
                    at: now(),
                }
            });
            out.push(frame);
        }
        out
    }
}

/// Reads what is recorded for a site the first time a stream opens for it: cards waiting on the
/// owner, and a Move in progress with its latest step. The first release's tables, read by id; missing
/// tables read as nothing recorded.
pub async fn baseline(hub: &Presence, pool: &SqlitePool, site: &str) -> anyhow::Result<()> {
    if hub.seeded(site) {
        return Ok(());
    }
    let waiting: Vec<(String, String)> = sqlx::query_as(
        "SELECT decision_id, kind FROM decisions WHERE site_id = ? AND state = 'waiting'",
    )
    .bind(site)
    .fetch_all(pool)
    .await
    .or_else(empty_if_missing)?;
    for (id, kind) in waiting {
        let kind = match kind.as_str() {
            "go-live" => CardKind::GoLive,
            "quarantine-restore" => CardKind::QuarantineRestore,
            _ => CardKind::OrderHeld,
        };
        hub.on_decision(site, &id, kind, true);
    }
    let moving: Vec<(String, String)> = sqlx::query_as(
        "SELECT move_id, state FROM moves WHERE site_id = ? \
         AND state NOT IN ('live', 'rolled-back', 'failed', 'stopped', 'awaiting-owner') \
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(site)
    .fetch_all(pool)
    .await
    .or_else(empty_if_missing)?;
    if let Some((move_id, _)) = moving.first() {
        let step: Vec<(String,)> = sqlx::query_as(
            "SELECT data FROM ui_events WHERE stream = ? AND name = 'step' ORDER BY seq DESC LIMIT 1",
        )
        .bind(format!("move:{move_id}"))
        .fetch_all(pool)
        .await
        .or_else(empty_if_missing)?;
        let label = step
            .first()
            .and_then(|(d,)| serde_json::from_str::<serde_json::Value>(d).ok())
            .and_then(|v| v["summary"].as_str().map(str::to_string));
        hub.publish(site, "mover", MarkState::Working, label.as_deref(), None);
    }
    Ok(())
}

fn empty_if_missing<T>(e: sqlx::Error) -> Result<Vec<T>, sqlx::Error> {
    if e.to_string().contains("no such table") {
        Ok(Vec::new())
    } else {
        Err(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentrix_types::DecisionId;
    use agentrix_types::ui::{CardEvent, StepEvent, StepName};
    use serde_json::json;

    #[test]
    fn an_ask_turn_moves_the_desk_and_settles_back() {
        let hub = Presence::new();
        let (_, mut rx) = hub.subscribe("k");
        hub.on_ask("k", "concierge", "ready", &json!({}));
        hub.on_ask(
            "k",
            "concierge",
            "awp.presence",
            &json!({"state": "working", "label": "Reading this site"}),
        );
        hub.on_ask(
            "k",
            "concierge",
            "awp.presence",
            &json!({"state": "working", "label": "Reading this site"}),
        );
        hub.on_ask(
            "k",
            "concierge",
            "awp.presence",
            &json!({"state": "answering"}),
        );
        hub.on_ask("k", "concierge", "done", &json!({}));
        let got: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let states: Vec<_> = got.iter().map(|f| f.state).collect();
        assert_eq!(
            states,
            vec![MarkState::Working, MarkState::Answering, MarkState::Idle],
            "a repeated frame is dropped"
        );
        assert_eq!(got[0].words, "Reading this site");
        assert!(got.windows(2).all(|w| w[0].seq < w[1].seq));
    }

    #[test]
    fn resting_holds_after_the_turn_ends() {
        let hub = Presence::new();
        hub.on_ask(
            "k",
            "concierge",
            "awp.presence",
            &json!({"state": "resting"}),
        );
        hub.on_ask("k", "concierge", "done", &json!({}));
        assert_eq!(
            hub.current("k", "concierge").unwrap().state,
            MarkState::Resting
        );
    }

    #[test]
    fn cards_light_the_raiser_and_the_chief_until_decided() {
        let hub = Presence::new();
        hub.on_move(
            "k",
            &UiEvent::Step(StepEvent::new(
                StepName::Rebuild,
                StepState::Running,
                10,
                "Rebuilding 12 pages",
                chrono::Utc::now(),
            )),
        );
        assert_eq!(
            hub.current("k", "mover").unwrap().label.as_deref(),
            Some("Rebuilding 12 pages")
        );
        let card = CardEvent {
            decision_id: DecisionId::generate(),
            kind: CardKind::GoLive,
            title: "Go live".into(),
            href: "/d".into(),
            at: chrono::Utc::now(),
        };
        let id = card.decision_id.clone();
        hub.on_move("k", &UiEvent::Card(card));
        assert_eq!(hub.current("k", "mover").unwrap().state, MarkState::Waiting);
        assert_eq!(
            hub.current("k", "coordinator").unwrap().state,
            MarkState::Waiting
        );
        hub.on_brief("k", true);
        assert_eq!(
            hub.current("k", "coordinator").unwrap().state,
            MarkState::Working
        );
        hub.on_brief("k", false);
        assert_eq!(
            hub.current("k", "coordinator").unwrap().state,
            MarkState::Waiting
        );
        hub.on_decision("k", id.as_str(), CardKind::GoLive, false);
        assert_eq!(hub.current("k", "mover").unwrap().state, MarkState::Idle);
        assert_eq!(
            hub.current("k", "coordinator").unwrap().state,
            MarkState::Asleep
        );
    }

    #[test]
    fn a_held_order_keeps_the_desk_waiting_after_its_turn() {
        let hub = Presence::new();
        hub.on_decision("k", "d-1", CardKind::OrderHeld, true);
        hub.on_ask(
            "k",
            "agent-desk",
            "awp.presence",
            &json!({"state": "working"}),
        );
        hub.on_ask("k", "agent-desk", "done", &json!({}));
        assert_eq!(
            hub.current("k", "agent-desk").unwrap().state,
            MarkState::Waiting
        );
    }

    #[test]
    fn the_voice_level_drives_speaking() {
        let hub = Presence::new();
        hub.on_voice("k", "concierge", Some(1.7));
        let f = hub.current("k", "concierge").unwrap();
        assert_eq!((f.state, f.level), (MarkState::Speaking, Some(1.0)));
        hub.on_voice("k", "concierge", None);
        assert_eq!(
            hub.current("k", "concierge").unwrap().state,
            MarkState::Idle
        );
    }

    #[test]
    fn a_snapshot_shows_every_member_on_duty() {
        let hub = Presence::new();
        let roster = crate::roster::sample("k", "kilele").unwrap();
        hub.on_ask(
            "k",
            "desk-cakes",
            "awp.presence",
            &json!({"state": "answering"}),
        );
        let snap = hub.snapshot(&roster);
        let get = |id: &str| snap.iter().find(|f| f.webagent == id).map(|f| f.state);
        assert_eq!(get("desk-cakes"), Some(MarkState::Answering));
        assert_eq!(get("desk-bread"), Some(MarkState::Idle));
        assert_eq!(get("editor"), Some(MarkState::Asleep));
        assert_eq!(get("coordinator"), Some(MarkState::Asleep));
        assert_eq!(get("sales"), None, "Sales is in Kilele's Store");
    }

    #[tokio::test]
    async fn baseline_reads_waiting_cards_and_a_running_move() {
        let ctx = agentrix_host::testing::context().await.unwrap();
        for sql in [
            "CREATE TABLE decisions (decision_id TEXT, site_id TEXT, kind TEXT, state TEXT)",
            "CREATE TABLE moves (move_id TEXT, site_id TEXT, state TEXT, updated_at TEXT)",
            "CREATE TABLE ui_events (stream TEXT, seq INTEGER, name TEXT, data TEXT)",
            "INSERT INTO decisions VALUES ('d-9', 's1', 'order-held', 'waiting')",
            "INSERT INTO moves VALUES ('m-1', 's1', 'cleaned', '2026-11-08T00:00:00Z')",
            r#"INSERT INTO ui_events VALUES ('move:m-1', 3, 'step', '{"summary": "Cleaning 40 pages"}')"#,
        ] {
            sqlx::query(sql).execute(&ctx.pool).await.unwrap();
        }
        let hub = Presence::new();
        baseline(&hub, &ctx.pool, "s1").await.unwrap();
        assert_eq!(
            hub.current("s1", "agent-desk").unwrap().state,
            MarkState::Waiting
        );
        assert_eq!(
            hub.current("s1", "coordinator").unwrap().state,
            MarkState::Waiting
        );
        assert_eq!(
            hub.current("s1", "mover").unwrap().label.as_deref(),
            Some("Cleaning 40 pages")
        );
        // A second stream doesn't read again.
        baseline(&hub, &ctx.pool, "s1").await.unwrap();
        let empty = Presence::new();
        baseline(
            &empty,
            &agentrix_host::testing::context().await.unwrap().pool,
            "s1",
        )
        .await
        .unwrap();
        assert!(empty.current("s1", "mover").is_none());
    }
}
