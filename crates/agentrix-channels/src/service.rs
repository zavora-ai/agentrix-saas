//! The channels service: the bots, the sites they speak for, and what owners and customers can
//! do through them. Every owner notification goes through W9's catalogue and outbox; every
//! decision goes through W5's decision contract; nothing here approves anything.

use std::sync::{Arc, RwLock};

use agentrix_decisions::{CardState, Decisions, RiskLevel};
use agentrix_host::NodeContext;
use agentrix_messages::{Messages, Outgoing, Recipient};
use agentrix_shell::store::ShellStore;
use agentrix_types::{ReceiptBody, ReceiptKind, SiteId, Subject};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::config::{BotConfig, Config};
use crate::store::{Link, Store};
use crate::telegram::Client;

/// The channel name in the tables.
pub const TELEGRAM: &str = "telegram";

/// The samples' offset from UTC (`utc_offset_min` in each bundle's manifest): Nairobi.
const SAMPLE_OFFSET_MIN: i32 = 180;

/// A bot node runs, with what `getMe` said about it.
pub struct Bot {
    /// Its configuration.
    pub cfg: BotConfig,
    /// The Bot API.
    pub client: Client,
    me: RwLock<Option<(String, String)>>,
}

impl Bot {
    /// The bot's @username, once `getMe` answered.
    pub fn username(&self) -> Option<String> {
        self.me
            .read()
            .ok()
            .and_then(|m| m.as_ref().map(|(u, _)| u.clone()))
    }

    /// The bot's display name, once `getMe` answered.
    pub fn display_name(&self) -> Option<String> {
        self.me
            .read()
            .ok()
            .and_then(|m| m.as_ref().map(|(_, n)| n.clone()))
    }

    /// Records what `getMe` said.
    pub fn set_me(&self, username: String, name: String) {
        if let Ok(mut m) = self.me.write() {
            *m = Some((username, name));
        }
    }

    /// `https://t.me/<bot>?start=<code>`, once the username is known.
    pub fn deep_link(&self, code: &str) -> Option<String> {
        self.username()
            .map(|u| format!("https://t.me/{u}?start={code}"))
    }
}

/// A site as channels show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteInfo {
    /// Its id.
    pub id: String,
    /// "Kilele Bakery".
    pub name: String,
    /// The host it's served on.
    pub host: String,
    /// A `-sample` site: say "Sample".
    pub sample: bool,
    /// Minutes from UTC, for times.
    pub offset_min: i32,
    /// Its owner, if claimed.
    pub owner: Option<String>,
}

/// A waiting decision as a chat shows it: W5's v2 cards and the first release's cards alike.
#[derive(Debug, Clone, PartialEq)]
pub struct CardView {
    /// `dec2_…` or the first release's id.
    pub id: String,
    /// What will happen.
    pub title: String,
    /// "The Cashier".
    pub agent: String,
    /// `low`, `medium`, `high`.
    pub risk: String,
    /// It moves money.
    pub money: bool,
    /// What W5's rule table lets a chat do with it.
    pub powers: crate::rules::ChatPowers,
    /// A v2 card (chat can decline it through W5's contract).
    pub v2: bool,
    /// When it was raised.
    pub created_at: DateTime<Utc>,
}

/// Node settings the channels read once at start.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    /// Node's loopback listener (`AGENTRIX_LISTEN`, `127.0.0.1:18050`), where customer questions
    /// reach the site's Ask. `None`: the platform origin.
    pub loopback: Option<String>,
    /// W5's `AGENTRIX_FREEZE_ENFORCED`: whether a Freeze stops what it says on this node. When it
    /// doesn't, STOP from chat says so and stops nothing.
    pub freeze_enforced: bool,
}

impl Settings {
    /// From node's environment.
    pub fn from_env() -> Self {
        Self {
            loopback: std::env::var("AGENTRIX_LISTEN")
                .ok()
                .map(|a| a.trim().replace("0.0.0.0", "127.0.0.1"))
                .filter(|a| !a.is_empty())
                .map(|a| format!("http://{a}")),
            freeze_enforced: agentrix_shell::freeze_enforced(),
        }
    }
}

struct Inner {
    ctx: NodeContext,
    settings: Settings,
    store: Store,
    messages: Messages,
    /// Every crate's effect runners, from node (approving from the app runs the card's effect).
    runners: std::sync::OnceLock<Vec<Arc<dyn agentrix_decisions::EffectRunner>>>,
    shell: ShellStore,
    bots: Vec<Arc<Bot>>,
}

/// The channels service. Cheap to clone.
#[derive(Clone)]
pub struct Channels {
    inner: Arc<Inner>,
}

/// "cashier" → "The Cashier".
pub fn agent_words(webagent: &str) -> String {
    let w = webagent.trim();
    if w.is_empty() {
        return "A WebAgent".to_owned();
    }
    // The Chief's ids (`coordinator` is its runtime principal) read as its full name.
    if w.eq_ignore_ascii_case("chief") || w.eq_ignore_ascii_case("coordinator") {
        return "The Chief WebAgent".to_owned();
    }
    let mut chars = w.chars();
    let first: String = chars
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    format!("The {first}{}", chars.as_str().replace(['-', '_'], " "))
}

/// "kilele-bakery-sample" → "Kilele Bakery".
fn name_from_id(site: &str) -> String {
    site.trim_end_matches("-sample")
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Channels {
    /// The service over node's context, with the bots in `config` (none: Telegram is off).
    pub fn new(ctx: &NodeContext, config: Option<&Config>) -> Self {
        let bots = config
            .map(|c| {
                c.bots
                    .iter()
                    .map(|b| {
                        Arc::new(Bot {
                            cfg: b.clone(),
                            client: match &b.api {
                                Some(api) => {
                                    Client::with_base(ctx.http.clone(), b.token.clone(), api)
                                }
                                None => Client::new(ctx.http.clone(), b.token.clone()),
                            },
                            me: RwLock::new(None),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self::with_bots(
            ctx,
            bots,
            agentrix_messages::from_context(ctx),
            Settings::from_env(),
        )
    }

    /// The service with explicit bots, messages and settings (tests use a fake Bot API).
    pub fn with_bots(
        ctx: &NodeContext,
        bots: Vec<Arc<Bot>>,
        messages: Messages,
        settings: Settings,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                ctx: ctx.clone(),
                settings,
                store: Store::new(ctx.pool.clone()),
                messages,
                runners: std::sync::OnceLock::new(),
                // W2's records hold the samples' currency and posted rate, for the AI budget.
                shell: ShellStore::new(ctx.pool.clone())
                    .with_records_at(&ctx.data_dir.join("agentrix-samples").join("records.db")),
                bots,
            }),
        }
    }

    /// A bot built over `client` (tests).
    pub fn bot(cfg: BotConfig, client: Client) -> Arc<Bot> {
        Arc::new(Bot {
            cfg,
            client,
            me: RwLock::new(None),
        })
    }

    /// The settings read at start.
    pub fn settings(&self) -> &Settings {
        &self.inner.settings
    }

    /// Node's context.
    pub fn ctx(&self) -> &NodeContext {
        &self.inner.ctx
    }

    /// The tables.
    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    /// W9's messages.
    pub fn messages(&self) -> &Messages {
        &self.inner.messages
    }

    /// W5's decision cards.
    pub fn decisions(&self) -> Decisions {
        Decisions::new(&self.inner.ctx)
            .with_runners(self.inner.runners.get().cloned().unwrap_or_default())
    }

    /// Hands the service node's effect runners, once. False if it already had them.
    pub fn set_runners(&self, runners: Vec<Arc<dyn agentrix_decisions::EffectRunner>>) -> bool {
        self.inner.runners.set(runners).is_ok()
    }

    /// W5's console records (quiet hours, Freeze).
    pub fn shell(&self) -> &ShellStore {
        &self.inner.shell
    }

    /// Every bot.
    pub fn bots(&self) -> &[Arc<Bot>] {
        &self.inner.bots
    }

    /// The bot for `site`.
    pub fn bot_for(&self, site: &str) -> Option<&Arc<Bot>> {
        self.inner.bots.iter().find(|b| b.cfg.site == site)
    }

    /// The platform's origin without a trailing slash.
    pub fn origin(&self) -> String {
        self.inner
            .ctx
            .origin
            .as_str()
            .trim_end_matches('/')
            .to_owned()
    }

    /// `{origin}/d/{card}`: the short link chats and email carry.
    pub fn card_url(&self, card: &str) -> String {
        agentrix_decisions::surface::short_link(&self.origin(), card)
    }

    /// The console for a site.
    pub fn console_url(&self, site: &str) -> String {
        format!("{}/console/{site}", self.origin())
    }

    /// Settings → Channels for a site.
    pub fn channels_url(&self, site: &str) -> String {
        format!("{}/console/{site}/apps/channels", self.origin())
    }

    /// The site's own origin as browsers reach it: the platform's scheme and port on the site's
    /// host (`http://kilele-bakery-sample.localhost:18050` locally).
    pub fn site_origin(&self, host: &str) -> String {
        let o = &self.inner.ctx.origin;
        match o.port() {
            Some(p) => format!("{}://{host}:{p}", o.scheme()),
            None => format!("{}://{host}", o.scheme()),
        }
    }

    /// The site, from `sites` when it has a row, else from its id (samples).
    pub async fn site(&self, site: &str) -> SiteInfo {
        let row: Option<(String, String, i64, Option<String>)> = sqlx::query_as(
            "SELECT COALESCE(name, ''), COALESCE(domain, ''), COALESCE(utc_offset_minutes, 0), owner_account_id FROM sites WHERE site_id = ?",
        )
        .bind(site)
        .fetch_optional(&self.inner.ctx.pool)
        .await
        .unwrap_or(None);
        let configured = self.bot_for(site).and_then(|b| b.cfg.host.clone());
        let sample_host = site
            .ends_with("-sample")
            .then(|| format!("{site}.{}", self.inner.ctx.platform_host));
        let had_row = row.is_some();
        let (name, domain, offset, owner) = row.unwrap_or_default();
        let host = configured
            .or(sample_host)
            .unwrap_or_else(|| domain.to_ascii_lowercase());
        // A sample without a `sites` row keeps its bundle's time zone: all three are in Nairobi.
        let offset = if !had_row && site.ends_with("-sample") {
            SAMPLE_OFFSET_MIN
        } else {
            i32::try_from(offset).unwrap_or(0)
        };
        SiteInfo {
            id: site.to_owned(),
            name: if name.trim().is_empty() {
                name_from_id(site)
            } else {
                name
            },
            sample: host
                .split('.')
                .next()
                .is_some_and(|l| l.ends_with("-sample")),
            host,
            offset_min: offset,
            owner,
        }
    }

    /// The `site` object every W9 message takes.
    pub fn site_json(&self, s: &SiteInfo) -> Value {
        json!({ "name": s.name, "host": s.host, "sample": s.sample, "url": self.site_origin(&s.host) })
    }

    /// The owner's first name, for greetings.
    pub async fn owner_first_name(&self, account: &str) -> Option<String> {
        self.inner
            .messages
            .directory()
            .owner(account)
            .await
            .ok()
            .flatten()
            .and_then(|c| c.name.split_whitespace().next().map(str::to_owned))
    }

    /// Whether `account` owns `site` (the first release's `sites`, or W1's roster owner).
    pub async fn owns(&self, site: &str, account: &str) -> bool {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM sites WHERE site_id = ? AND owner_account_id = ?")
                .bind(site)
                .bind(account)
                .fetch_optional(&self.inner.ctx.pool)
                .await
                .unwrap_or(None);
        row.is_some()
    }

    /// Sends a catalogue message to a linked chat through W9 (recorded in the outbox first).
    /// Returns the outbox id.
    pub async fn notify(
        &self,
        link: &Link,
        message: &str,
        extra: Value,
        reason: &str,
        dedupe_key: Option<String>,
    ) -> anyhow::Result<Option<String>> {
        let site = self.site(&link.site_id).await;
        let mut context = json!({
            "site": self.site_json(&site),
            "console_url": self.console_url(&link.site_id),
            "preferences_url": self.channels_url(&link.site_id),
        });
        if let (Some(obj), Some(more)) = (context.as_object_mut(), extra.as_object()) {
            for (k, v) in more {
                obj.insert(k.clone(), v.clone());
            }
        }
        let to = Recipient {
            name: self.owner_first_name(&link.account_id).await,
            phone: Some(format!("telegram:{}", link.chat_id)),
            account_id: Some(link.account_id.clone()),
            ..Recipient::default()
        };
        let sent = self
            .inner
            .messages
            .send(Outgoing {
                message: message.to_owned(),
                site_id: Some(link.site_id.clone()),
                to,
                context,
                reason: reason.to_owned(),
                dedupe_key,
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(sent.entries.first().map(|e| e.id.clone()))
    }

    /// The site's waiting decisions, newest first: W5's v2 cards, then the first release's.
    pub async fn waiting_cards(&self, site: &str) -> Vec<CardView> {
        let mut out = Vec::new();
        match self.decisions().list(site).await {
            Ok(rows) => {
                for r in rows.into_iter().filter(|r| r.state == CardState::Waiting) {
                    out.push(CardView {
                        id: r.card.id.clone(),
                        title: r.card.title.clone(),
                        agent: agent_words(&r.card.raised_by.webagent),
                        risk: match r.card.risk.level {
                            RiskLevel::Low => "low",
                            RiskLevel::Medium => "medium",
                            RiskLevel::High => "high",
                        }
                        .to_owned(),
                        money: r.card.effect.amount.is_some(),
                        powers: crate::rules::ChatPowers::of(&r.card),
                        v2: true,
                        created_at: r.card.created_at,
                    });
                }
            }
            Err(e) => tracing::debug!(error = %format!("{e:#}"), "no v2 cards"),
        }
        let v1: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT decision_id, card, created_at FROM decisions WHERE site_id = ? AND state = 'waiting' ORDER BY created_at DESC",
        )
        .bind(site)
        .fetch_all(&self.inner.ctx.pool)
        .await
        .unwrap_or_default();
        for (id, card, at) in v1 {
            let Ok(card) = serde_json::from_str::<Value>(&card) else {
                continue;
            };
            let expiry = card["effect"]["expiry"]
                .as_str()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok());
            if expiry.is_some_and(|e| e.with_timezone(&Utc) < Utc::now()) {
                continue;
            }
            let money = !card["effect"]["amount"].is_null();
            out.push(CardView {
                id,
                title: card["title"].as_str().unwrap_or("A decision").to_owned(),
                agent: agent_words(card["effect"]["webagent"].as_str().unwrap_or("")),
                risk: if money { "high" } else { "medium" }.to_owned(),
                money,
                powers: crate::rules::ChatPowers::default(),
                v2: false,
                created_at: DateTime::parse_from_rfc3339(&at)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_default(),
            });
        }
        out.sort_by_key(|c| std::cmp::Reverse(c.created_at));
        out
    }

    /// The W9 `card` object for `owner.decision_waiting`.
    pub fn card_json(&self, card: &CardView, offset_min: i32) -> Value {
        json!({
            "since": clock(card.created_at, offset_min),
            "title": card.title,
            "agent": card.agent,
            "risk": card.risk,
            "money": card.money,
            "url": self.card_url(&card.id),
        })
    }

    /// Declines a card from chat through W5's contract. Returns the receipt, or the owner's words
    /// for why it wasn't declined.
    pub async fn decline(
        &self,
        site: &str,
        account: &str,
        card: &str,
        heard: &str,
    ) -> Result<String, String> {
        let waiting = self.waiting_cards(site).await;
        if let Some(c) = waiting.iter().find(|c| c.id == card)
            && !c.v2
        {
            return Err(format!(
                "I can't decline this one from chat. Open it in the console: {}",
                self.card_url(card)
            ));
        }
        self.decisions()
            .decide_spoken(site, account, card, false, "chat", heard)
            .await
            .map_err(|r| r.words)
    }

    /// The latest brief for `site`, if the Chief has written one.
    pub async fn brief(&self, site: &str) -> Option<agentrix_types::records::Brief> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT body FROM briefs WHERE site_id = ? ORDER BY written_at DESC LIMIT 1",
        )
        .bind(site)
        .fetch_optional(&self.inner.ctx.pool)
        .await
        .ok()
        .flatten();
        row.and_then(|(b,)| serde_json::from_str(&b).ok())
    }

    /// The W9 `brief` object for `owner.morning_brief`: the Chief's latest brief, the waiting
    /// count and this month's AI spend against the monthly AI budget, in the site's currency
    /// ([`Channels::spend_json`]).
    pub async fn brief_json(&self, site: &str) -> Value {
        let brief = self.brief(site).await;
        let waiting = self.waiting_cards(site).await.len();
        let headline = brief
            .as_ref()
            .and_then(|b| b.headline.clone())
            .filter(|h| !h.trim().is_empty())
            .unwrap_or_else(|| {
                if brief.is_some() {
                    "Here's the latest brief from your Chief WebAgent.".to_owned()
                } else {
                    "No brief yet. The Chief WebAgent writes one from what your WebAgents record."
                        .to_owned()
                }
            });
        let lines: Vec<String> = brief
            .as_ref()
            .map(|b| {
                b.lines
                    .iter()
                    .map(|l| l.text.trim().to_owned())
                    .filter(|t| !t.is_empty())
                    .take(6)
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "headline": headline,
            "lines": lines,
            "waiting": waiting,
            "spend": self.spend_json(site).await,
        })
    }

    /// This month's AI spend against the site's monthly AI budget, in its currency: the figure
    /// the console's meter shows (`agentrix_store::budget`). `used` and `budget` are numbers in
    /// `currency` (`budget` is null when no monthly budget is set); `words` is how the owner
    /// reads it ("KSh 38 of KSh 3,870 this month"). The gateway's daily limit is an internal
    /// guard and never appears. When spend can't be read, every figure is null.
    pub async fn spend_json(&self, site: &str) -> Value {
        let info = self.site(site).await;
        let read = agentrix_store::budget::this_month(
            &self.inner.ctx.pool,
            self.inner.shell.records(),
            site,
            &info.host,
            Utc::now(),
        )
        .await;
        match read {
            Ok(m) => json!({
                "used": m.used(),
                "budget": m.cap(),
                "currency": m.budget.currency.code,
                "period": "month",
                "words": m.words(),
            }),
            Err(e) => {
                tracing::warn!(site, error = %format!("{e:#}"), "couldn't read this month's AI spend");
                json!({ "used": null, "budget": null, "currency": null, "period": "month", "words": null })
            }
        }
    }

    /// Freezes every WebAgent on the site from chat, through W5's Freeze. `Ok(false)` when Freeze
    /// isn't enforced yet on this node (W5's `AGENTRIX_FREEZE_ENFORCED`), so nothing was stopped
    /// and the chat must say so.
    pub async fn freeze(&self, site: &str, account: &str) -> anyhow::Result<bool> {
        if !self.inner.settings.freeze_enforced {
            return Ok(false);
        }
        self.inner
            .shell
            .set_frozen(site, account, "chat", Utc::now())
            .await?;
        tracing::info!(site, "site frozen by its owner from a chat");
        Ok(true)
    }

    /// Writes `channel.changed` for a link or unlink. No chat id or name goes in the receipt.
    pub async fn receipt_channel(&self, site: &str, account: &str, action: &str, how: &str) {
        let now = Utc::now();
        let body = ReceiptBody::new(
            ReceiptKind::ChannelChanged,
            SiteId::new(site.to_owned()),
            Subject::Site(SiteId::new(site.to_owned())),
            now,
            now,
        )
        .with_title(match action {
            "linked" => "You linked Telegram to this site",
            _ => "You unlinked Telegram from this site",
        });
        let mut body = body;
        if let Some(Value::Object(map)) = body.detail.as_mut() {
            map.insert("action".into(), json!(action));
            map.insert("channel".into(), json!(TELEGRAM));
            map.insert("by".into(), json!(account));
            map.insert("how".into(), json!(how));
        }
        if let Err(e) = self.inner.ctx.receipts.issue(body).await {
            tracing::error!(error = %format!("{e:#}"), "couldn't write the channel receipt");
        }
    }
}

/// "07:01" in the site's time.
pub fn clock(at: DateTime<Utc>, offset_min: i32) -> String {
    let offset = chrono::FixedOffset::east_opt(offset_min * 60)
        .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).expect("zero offset"));
    at.with_timezone(&offset).format("%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_for_agents_and_sites() {
        assert_eq!(agent_words("cashier"), "The Cashier");
        assert_eq!(agent_words(""), "A WebAgent");
        assert_eq!(agent_words("chief"), "The Chief WebAgent");
        assert_eq!(agent_words("coordinator"), "The Chief WebAgent");
        assert_eq!(name_from_id("kilele-bakery-sample"), "Kilele Bakery");
    }
}
