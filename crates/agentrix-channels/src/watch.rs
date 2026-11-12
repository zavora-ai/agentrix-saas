//! What linked owners hear without asking: a new waiting decision, the morning brief when quiet
//! hours end, a question a desk passed on, and a Freeze. Each goes once per link
//! (`channel_alerts`), through W9's catalogue and outbox.

use chrono::{DateTime, FixedOffset, Utc};
use serde_json::json;
use tokio::sync::OnceCell;

use crate::service::{Channels, SiteInfo, clock};
use crate::store::Link;

/// "2026-11-08": the site's local date.
pub fn local_day(now: DateTime<Utc>, offset_min: i32) -> String {
    let offset = FixedOffset::east_opt(offset_min * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"));
    now.with_timezone(&offset).format("%Y-%m-%d").to_string()
}

/// Whether the morning brief is due: quiet hours ended today (local) and we're past their end.
pub fn brief_due(now: DateTime<Utc>, offset_min: i32, quiet_end: chrono::NaiveTime) -> bool {
    let offset = FixedOffset::east_opt(offset_min * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"));
    now.with_timezone(&offset).time() >= quiet_end
}

/// The watcher's state across ticks.
#[derive(Default)]
pub struct Watcher {
    records: OnceCell<Option<agentrix_records::Records>>,
}

impl Watcher {
    async fn records(&self, channels: &Channels) -> Option<&agentrix_records::Records> {
        self.records
            .get_or_init(|| async {
                let path = channels
                    .ctx()
                    .data_dir
                    .join("agentrix-samples")
                    .join("records.db");
                if !path.exists() {
                    return None;
                }
                agentrix_records::Records::open(&path)
                    .await
                    .map_err(|e| tracing::warn!(error = %format!("{e:#}"), "couldn't open the samples' records"))
                    .ok()
            })
            .await
            .as_ref()
    }

    /// One pass over every bot's site.
    pub async fn tick(&self, channels: &Channels) {
        for bot in channels.bots() {
            let site = bot.cfg.site.clone();
            let links = match channels.store().links_for_site(&site).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(error = %e, "couldn't read linked chats");
                    continue;
                }
            };
            if links.is_empty() {
                continue;
            }
            let info = channels.site(&site).await;
            for link in &links {
                self.cards(channels, &info, link).await;
                self.brief(channels, &info, link).await;
                self.freeze(channels, &info, link).await;
                if info.sample {
                    self.questions(channels, &info, link).await;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn announce(
        &self,
        channels: &Channels,
        link: &Link,
        kind: &str,
        subject: &str,
        message: &str,
        context: serde_json::Value,
        reason: &str,
    ) {
        let now = Utc::now();
        match channels
            .store()
            .mark_alert(&link.id, kind, subject, now)
            .await
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                tracing::warn!(error = %e, "couldn't record an alert");
                return;
            }
        }
        let key = format!("tg:{}:{kind}:{subject}", link.id);
        match channels
            .notify(link, message, context, reason, Some(key))
            .await
        {
            Ok(Some(outbox)) => {
                let _ = channels
                    .store()
                    .alert_outbox(&link.id, kind, subject, &outbox)
                    .await;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), message, "an alert didn't go; it retries next tick");
                let _ = channels.store().unmark_alert(&link.id, kind, subject).await;
            }
        }
    }

    async fn cards(&self, channels: &Channels, info: &SiteInfo, link: &Link) {
        let cards = channels.waiting_cards(&info.id).await;
        let waiting = cards.len();
        for c in cards.iter().rev() {
            if c.created_at < link.linked_at {
                continue;
            }
            self.announce(
                channels,
                link,
                "card",
                &c.id,
                "owner.decision_waiting",
                json!({ "waiting": waiting, "card": channels.card_json(c, info.offset_min) }),
                &format!("{} raised a decision on {}", c.agent, info.name),
            )
            .await;
        }
    }

    async fn brief(&self, channels: &Channels, info: &SiteInfo, link: &Link) {
        let now = Utc::now();
        let quiet = channels
            .shell()
            .quiet_hours(&info.id)
            .await
            .unwrap_or_default();
        if !brief_due(now, info.offset_min, quiet.end) {
            return;
        }
        let day = local_day(now, info.offset_min);
        let brief = channels.brief_json(&info.id).await;
        self.announce(
            channels,
            link,
            "brief",
            &day,
            "owner.morning_brief",
            json!({ "brief": brief }),
            &format!(
                "The morning brief goes out when quiet hours end, at {}",
                quiet.end.format("%H:%M")
            ),
        )
        .await;
    }

    async fn freeze(&self, channels: &Channels, info: &SiteInfo, link: &Link) {
        let Ok(Some(f)) = channels.shell().freeze(&info.id).await else {
            return;
        };
        if f.at < link.linked_at {
            return;
        }
        let by = if f.by == link.account_id {
            channels
                .owner_first_name(&f.by)
                .await
                .unwrap_or_else(|| "You".into())
        } else {
            "Someone on your team".into()
        };
        let from = match f.how.as_str() {
            "chat" => "Telegram (STOP)",
            "voice" => "voice",
            "gesture" => "a hand gesture",
            _ => "the console",
        };
        self.announce(
            channels,
            link,
            "freeze",
            &f.id,
            "owner.frozen",
            json!({ "freeze": { "frozen": true, "by": by, "at": clock(f.at, info.offset_min), "from": from } }),
            &format!("{by} froze every WebAgent on {}", info.name),
        )
        .await;
    }

    async fn questions(&self, channels: &Channels, info: &SiteInfo, link: &Link) {
        let Some(records) = self.records(channels).await else {
            return;
        };
        let Ok(questions) = records.questions(&info.id).await else {
            return;
        };
        let since = link.linked_at.timestamp();
        for q in questions
            .iter()
            .filter(|q| q.state == "open" && q.created_at >= since)
            .rev()
        {
            let asked = DateTime::<Utc>::from_timestamp(q.created_at, 0).unwrap_or_else(Utc::now);
            let offset = FixedOffset::east_opt(info.offset_min * 60)
                .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"));
            self.announce(
                channels,
                link,
                "question",
                &q.question_id,
                "owner.question_passed",
                json!({ "question": {
                    "text": q.question,
                    "page": q.page.clone().unwrap_or_else(|| "the site".into()),
                    "asked_at": asked.with_timezone(&offset).format("%a %-d %b, %H:%M").to_string(),
                    "desk": q.desk,
                    "reply_to": q.contact,
                    "url": format!("{}/samples/{}/questions", channels.origin(), info.id),
                } }),
                &format!(
                    "The {} passed a visitor's question it couldn't answer from the site",
                    q.desk
                ),
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_brief_waits_for_quiet_hours_to_end_in_the_site_s_time() {
        let end = chrono::NaiveTime::from_hms_opt(7, 0, 0).unwrap();
        // 03:30 UTC is 06:30 in Nairobi: not yet.
        let early = DateTime::parse_from_rfc3339("2026-11-08T03:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!brief_due(early, 180, end));
        let later = DateTime::parse_from_rfc3339("2026-11-08T04:05:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(brief_due(later, 180, end));
        assert_eq!(local_day(early, 180), "2026-11-08");
        let late_utc = DateTime::parse_from_rfc3339("2026-11-07T22:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(local_day(late_utc, 180), "2026-11-08");
    }
}
