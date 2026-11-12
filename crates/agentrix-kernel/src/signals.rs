//! Signals: what the public desks tell the back office, as typed records (ADR-011; 05 §4.2).
//!
//! A desk reads untrusted text all day; the back office never does (A-I3). When a desk learns
//! something the back office should act on (visitors asked what the site doesn't say, a visitor
//! started an order, someone needs a person) it emits an `awp_types::Signal`: typed fields, and
//! free text only as `Bounded`, marked untrusted. Before a signal is stored, every `Bounded`
//! field is redacted (email addresses and phone numbers) and clipped, so a visitor's personal
//! details never reach a back-office prompt.
//!
//! Each stored signal names the WebAgents whose wake set lists its kind (the Editor wakes on
//! `content_gap`, Sales on `lead_captured`), read from the site's roster. Nothing here starts
//! a model call: the back office takes signals off its agenda when it next wakes.

use agentrix_types::SiteId;
use awp_types::{Bounded, Signal, SignalKind};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::spec::WakeKind;
use crate::store::Rosters;

/// The longest topic a content gap keeps.
pub const TOPIC_MAX: usize = 120;
/// The longest sample question a content gap keeps.
pub const SAMPLE_MAX: usize = 200;
/// The most sample questions a content gap keeps.
pub const SAMPLES_MAX: usize = 3;

/// Replaces email addresses and phone numbers in visitor text.
pub fn redact(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for word in text.split(' ') {
        let core = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '+');
        let digits = core.chars().filter(char::is_ascii_digit).count();
        if core.contains('@') && core.contains('.') {
            out.push(word.replace(core, "[email]"));
        } else if digits >= 7
            && core
                .chars()
                .all(|c| c.is_ascii_digit() || "+-().".contains(c))
        {
            out.push(word.replace(core, "[number]"));
        } else {
            out.push(word.to_string());
        }
    }
    // A number written with spaces ("0712 345 678") is caught as a run of short digit groups.
    let joined = out.join(" ");
    redact_spaced_numbers(&joined)
}

fn redact_spaced_numbers(text: &str) -> String {
    let words: Vec<&str> = text.split(' ').collect();
    let is_group = |w: &str| {
        !w.is_empty()
            && w.chars().all(|c| c.is_ascii_digit() || "+-().".contains(c))
            && w.chars().any(|c| c.is_ascii_digit())
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if is_group(words[i]) {
            let mut j = i;
            let mut digits = 0;
            while j < words.len() && is_group(words[j]) {
                digits += words[j].chars().filter(char::is_ascii_digit).count();
                j += 1;
            }
            if digits >= 7 && j - i > 1 {
                out.push("[number]".to_string());
            } else {
                out.extend(words[i..j].iter().map(|w| w.to_string()));
            }
            i = j;
        } else {
            out.push(words[i].to_string());
            i += 1;
        }
    }
    out.join(" ")
}

/// Visitor text as a signal may carry it: redacted, then clipped to `max` characters.
pub fn bounded(text: &str, max: usize) -> Bounded {
    Bounded::truncated(&redact(text.trim()), max)
}

fn screen(b: &Bounded, max: usize) -> Bounded {
    bounded(b.as_str(), max)
}

/// The same kind with every free-text field redacted and clipped.
fn screened(kind: SignalKind) -> SignalKind {
    match kind {
        SignalKind::ContentGap {
            topic,
            question_count,
            samples,
        } => SignalKind::ContentGap {
            topic: screen(&topic, TOPIC_MAX),
            question_count,
            samples: samples
                .iter()
                .take(SAMPLES_MAX)
                .map(|s| screen(s, SAMPLE_MAX))
                .collect(),
        },
        SignalKind::ReviewSeen {
            platform,
            rating,
            excerpt,
        } => SignalKind::ReviewSeen {
            platform,
            rating,
            excerpt: screen(&excerpt, SAMPLE_MAX),
        },
        SignalKind::LeadCaptured {
            lead_ref,
            source,
            consent,
            answers,
        } => SignalKind::LeadCaptured {
            lead_ref,
            source,
            consent,
            answers: answers
                .into_iter()
                .map(|(k, v)| (k, screen(&v, SAMPLE_MAX)))
                .collect(),
        },
        other => other,
    }
}

/// One stored signal, as the console and the back office read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stored {
    /// The desk that emitted it.
    pub desk: String,
    /// The signal.
    pub signal: Signal,
    /// The WebAgents it wakes.
    pub wakes: Vec<String>,
    /// When a back-office WebAgent took it off the agenda.
    pub seen_at: Option<DateTime<Utc>>,
}

fn time(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl Rosters {
    /// A desk tells the back office something. `desk` is `concierge`, `agent-desk` or
    /// `desk-{slug}`. Returns the signal's id.
    pub async fn emit(&self, site: &str, desk: &str, kind: SignalKind) -> anyhow::Result<Uuid> {
        anyhow::ensure!(
            !desk.contains(':') && !desk.is_empty(),
            "a desk id like desk-cakes"
        );
        let kind = screened(kind);
        let name = kind.name();
        let wakes: Vec<String> = match self.roster(site).await? {
            Some(r) => r
                .on_duty()
                .filter(|m| {
                    m.spec
                        .wakes
                        .iter()
                        .any(|w| w.kind == WakeKind::Signal && w.key == name)
                })
                .map(|m| m.spec.id.clone())
                .collect(),
            None => Vec::new(),
        };
        let signal = Signal {
            id: Uuid::new_v4(),
            kind,
            emitted_by: format!("urn:adk:agent:{}/{desk}", SiteId::new(site).as_str()),
            emitted_at: Utc::now(),
        };
        sqlx::query(
            "INSERT INTO webagent_signals (signal_id, site_id, desk, kind, body, wakes, emitted_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(signal.id.to_string())
        .bind(site)
        .bind(desk)
        .bind(name)
        .bind(serde_json::to_string(&signal)?)
        .bind(serde_json::to_string(&wakes)?)
        .bind(time(signal.emitted_at))
        .execute(self.pool())
        .await?;
        Ok(signal.id)
    }

    /// The site's signals, newest first: the back office's agenda (`unseen_only`) or the
    /// console's list.
    pub async fn signals(
        &self,
        site: &str,
        unseen_only: bool,
        limit: u32,
    ) -> anyhow::Result<Vec<Stored>> {
        let sql = if unseen_only {
            "SELECT desk, body, wakes, seen_at FROM webagent_signals WHERE site_id = ? AND seen_at IS NULL \
             ORDER BY emitted_at DESC LIMIT ?"
        } else {
            "SELECT desk, body, wakes, seen_at FROM webagent_signals WHERE site_id = ? \
             ORDER BY emitted_at DESC LIMIT ?"
        };
        let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(sql)
            .bind(site)
            .bind(i64::from(limit))
            .fetch_all(self.pool())
            .await?;
        rows.into_iter()
            .map(|(desk, body, wakes, seen)| {
                Ok(Stored {
                    desk,
                    signal: serde_json::from_str(&body)?,
                    wakes: serde_json::from_str(&wakes)?,
                    seen_at: seen
                        .map(|s| DateTime::parse_from_rfc3339(&s).map(|t| t.with_timezone(&Utc)))
                        .transpose()?,
                })
            })
            .collect()
    }

    /// A back-office WebAgent took a signal off its agenda.
    pub async fn mark_seen(&self, site: &str, signal: Uuid) -> anyhow::Result<()> {
        sqlx::query("UPDATE webagent_signals SET seen_at = ? WHERE site_id = ? AND signal_id = ? AND seen_at IS NULL")
            .bind(time(Utc::now()))
            .bind(site)
            .bind(signal.to_string())
            .execute(self.pool())
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use awp_types::Outcome;

    #[test]
    fn personal_details_never_reach_the_back_office() {
        assert_eq!(
            redact("email me at amina@example.com please"),
            "email me at [email] please"
        );
        assert_eq!(redact("call +254712345678 today"), "call [number] today");
        assert_eq!(redact("call 0712 345 678 today"), "call [number] today");
        assert_eq!(
            redact("a cake for 12 people on 14 Nov"),
            "a cake for 12 people on 14 Nov"
        );
        let b = bounded("Do you deliver to amina@example.com in Kilimani?", 30);
        assert!(b.is_untrusted());
        assert!(!b.as_str().contains('@'));
        assert!(b.as_str().chars().count() <= 30);
    }

    #[tokio::test]
    async fn a_content_gap_wakes_the_editor_on_duty() {
        let ctx = agentrix_host::testing::context().await.unwrap();
        crate::migrate(&ctx.pool).await.unwrap();
        let rosters = Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
        rosters.seed_sample("kilele", "kilele").await.unwrap();
        let gap = SignalKind::ContentGap {
            topic: bounded("gluten-free cakes", TOPIC_MAX),
            question_count: 1,
            samples: vec![
                bounded(
                    "Do you make gluten-free cakes? Text me on 0712 345 678",
                    SAMPLE_MAX,
                ),
                bounded("two", SAMPLE_MAX),
                bounded("three", SAMPLE_MAX),
                bounded("four", SAMPLE_MAX),
            ],
        };
        let id = rosters.emit("kilele", "desk-cakes", gap).await.unwrap();
        rosters
            .emit(
                "kilele",
                "desk-orders",
                SignalKind::VisitorIntent {
                    capability: "request_custom_order".into(),
                    outcome: Outcome::Held,
                },
            )
            .await
            .unwrap();
        let all = rosters.signals("kilele", false, 10).await.unwrap();
        assert_eq!(all.len(), 2);
        let gap = all.iter().find(|s| s.signal.id == id).unwrap();
        assert_eq!(gap.wakes, vec!["editor".to_string()]);
        assert_eq!(gap.signal.emitted_by, "urn:adk:agent:kilele/desk-cakes");
        let SignalKind::ContentGap { samples, .. } = &gap.signal.kind else {
            panic!()
        };
        assert_eq!(samples.len(), SAMPLES_MAX);
        assert!(!samples[0].as_str().contains("345"), "{}", samples[0]);
        rosters.mark_seen("kilele", id).await.unwrap();
        assert_eq!(rosters.signals("kilele", true, 10).await.unwrap().len(), 1);
    }
}
