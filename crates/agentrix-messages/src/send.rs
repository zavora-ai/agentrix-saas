//! Sending: record every message in the outbox, then hand it to a transport.
//!
//! ```no_run
//! # async fn demo(messages: agentrix_messages::Messages) -> Result<(), agentrix_messages::SendError> {
//! use agentrix_messages::{Outgoing, Recipient};
//! use serde_json::json;
//! let sent = messages
//!     .send(Outgoing {
//!         message: "customer.signin_code".into(),
//!         site_id: Some("site_01".into()),
//!         to: Recipient::email("amina@example.com", Some("Amina")),
//!         context: json!({
//!             "site": {"name": "Kilele Bakery", "host": "kilele-bakery-sample.agentrix.website", "sample": true},
//!             "code": "482913", "minutes": 10
//!         }),
//!         reason: "Amina asked for a sign-in code at kilele-bakery-sample.agentrix.website".into(),
//!         dedupe_key: None,
//!     })
//!     .await?;
//! for entry in &sent.entries {
//!     println!("{} {}: {:?}", entry.channel.as_str(), entry.status.label(), entry.status_reason);
//! }
//! # Ok(()) }
//! ```
//!
//! Channels drivers live behind W6's Effects Gateway: node gives [`Messages`] a [`Transport`]
//! that reaches it. Until a provider is set up, [`Undelivered`] records every message as
//! "Not sent" with the reason, so flows never wait on delivery and the console shows what
//! would have gone out.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value as Json, json};

use crate::catalogue::{self, Audience, Channel, Kind, MessageDef, Priority};
use crate::directory::Directory;
use crate::outbox::{Entry, NewEntry, Outbox, Status};
use crate::render::{self, Chat, Push, RenderError, Rendered};

/// Who a message goes to.
#[derive(Debug, Clone, Default)]
pub struct Recipient {
    /// Their first name, if known (templates greet them with it).
    pub name: Option<String>,
    /// Their email address.
    pub email: Option<String>,
    /// Their WhatsApp number. Set it only when they agreed to WhatsApp messages.
    pub phone: Option<String>,
    /// Their owner account (owners and teammates): push devices and quiet hours hang off it.
    pub account_id: Option<String>,
    /// Push devices that turned notifications on (subscription ids).
    pub push_devices: Vec<String>,
}

impl Recipient {
    /// Someone reached by email.
    pub fn email(address: impl Into<String>, name: Option<&str>) -> Self {
        Self {
            name: name.map(str::to_string),
            email: Some(address.into()),
            ..Self::default()
        }
    }

    /// Adds a WhatsApp number the person agreed to be messaged on.
    pub fn with_phone(mut self, phone: impl Into<String>) -> Self {
        self.phone = Some(phone.into());
        self
    }
}

/// One message to send.
#[derive(Debug, Clone)]
pub struct Outgoing {
    /// The catalogue id, e.g. `customer.order_received`.
    pub message: String,
    /// The site it's about.
    pub site_id: Option<String>,
    /// Who gets it.
    pub to: Recipient,
    /// The template's values (see the example beside each template).
    pub context: Json,
    /// Why it's being sent, in words the owner reads in the console.
    pub reason: String,
    /// Sending the same key again records nothing new (for retries and repeated events).
    pub dedupe_key: Option<String>,
}

/// What [`Messages::send`] did: one outbox row per channel it had somewhere to send to.
#[derive(Debug, Clone, Serialize)]
pub struct Sent {
    /// The rows, newest state.
    pub entries: Vec<Entry>,
}

impl Sent {
    /// Whether any channel's provider accepted it.
    pub fn delivered(&self) -> bool {
        self.entries.iter().any(|e| e.status == Status::Sent)
    }
}

/// Why a message couldn't be recorded.
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    /// The template couldn't be rendered (usually a missing value).
    #[error(transparent)]
    Render(#[from] RenderError),
    /// The outbox couldn't be written.
    #[error("outbox: {0}")]
    Store(#[from] sqlx::Error),
    /// A rule of the catalogue was broken.
    #[error("{0}")]
    Rule(String),
    /// The directory couldn't be read.
    #[error("directory: {0}")]
    Directory(anyhow::Error),
}

/// A message ready for one channel's provider.
#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    /// The outbox row.
    pub outbox_id: String,
    /// Delivery attempts on this row before this one (0 the first time). A provider's
    /// idempotency key carries it, so another try isn't answered from the first one's record.
    pub attempt: i64,
    /// The catalogue id.
    pub message: String,
    /// The site it's about.
    pub site_id: Option<String>,
    /// The channel.
    pub channel: Channel,
    /// The address, number or push subscription.
    pub to: String,
    /// The recipient's name.
    pub to_name: Option<String>,
    /// The From name.
    pub from_name: String,
    /// The subject (email) or one line.
    pub subject: String,
    /// The plain text.
    pub text: String,
    /// The HTML (email).
    pub html: Option<String>,
    /// Extra email headers, e.g. `List-Unsubscribe`.
    pub headers: Vec<(String, String)>,
    /// The push version.
    pub push: Option<Push>,
    /// The chat version.
    pub chat: Option<Chat>,
}

/// Whether a transport can deliver on a channel right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// It can.
    Ready,
    /// It can't, and this is why (shown in the console).
    NotReady(String),
}

/// What a provider said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// Accepted.
    Sent {
        /// The provider's name.
        provider: String,
        /// The provider's id for the message (for bounces).
        provider_ref: String,
    },
    /// Deliberately not delivered.
    NotSent {
        /// Why.
        reason: String,
    },
    /// Refused or unreachable.
    Failed {
        /// Why, in words.
        reason: String,
    },
}

/// Delivers envelopes. Node's adapter reaches W6's Effects Gateway, which holds the provider
/// credentials (email API key, VAPID private key, WhatsApp token).
#[async_trait]
pub trait Transport: Send + Sync {
    /// Whether `channel` can be delivered now.
    fn readiness(&self, channel: Channel) -> Readiness;
    /// Delivers one envelope.
    async fn deliver(&self, envelope: &Envelope) -> Delivery;
}

/// The transport before any provider is set up: nothing leaves the platform.
#[derive(Debug, Clone, Default)]
pub struct Undelivered;

#[async_trait]
impl Transport for Undelivered {
    fn readiness(&self, channel: Channel) -> Readiness {
        Readiness::NotReady(
            match channel {
                Channel::Email => "Email isn't switched on yet",
                Channel::Push => "Phone notifications aren't switched on yet",
                Channel::Chat => "WhatsApp and Telegram aren't connected yet",
            }
            .to_string(),
        )
    }

    async fn deliver(&self, _envelope: &Envelope) -> Delivery {
        Delivery::NotSent {
            reason: "No provider is set up yet".into(),
        }
    }
}

/// Quiet hours: when Night Time holds operational messages for an owner. W5's shell keeps
/// them per site (`ShellStore::quiet_hours`); node adapts that with [`quiet_end`].
#[async_trait]
pub trait QuietHours: Send + Sync {
    /// When quiet hours end, if `now` is inside them for this account (and site, if the
    /// message is about one).
    async fn quiet_until(
        &self,
        account: &str,
        site_id: Option<&str>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>>;
}

/// When quiet hours from `start` to `end` (local times; `start` may be after `end`, as in
/// 22:00 to 06:00) end, if `now` is inside them at `offset_minutes` from UTC.
pub fn quiet_end(
    start: chrono::NaiveTime,
    end: chrono::NaiveTime,
    offset_minutes: i32,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let offset = chrono::FixedOffset::east_opt(offset_minutes * 60)?;
    let local = now.with_timezone(&offset);
    let t = local.time();
    let quiet = if start < end {
        t >= start && t < end
    } else {
        t >= start || t < end
    };
    if !quiet || start == end {
        return None;
    }
    let mut day = local.date_naive();
    if t >= end {
        day = day.succ_opt()?;
    }
    let local_end = day.and_time(end).and_local_timezone(offset).single()?;
    Some(local_end.with_timezone(&Utc))
}

/// No quiet hours for anyone.
#[derive(Debug, Clone, Default)]
pub struct NeverQuiet;

#[async_trait]
impl QuietHours for NeverQuiet {
    async fn quiet_until(
        &self,
        _account: &str,
        _site_id: Option<&str>,
        _now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        None
    }
}

/// The clock, swappable in tests.
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

struct Inner {
    outbox: Outbox,
    prefs: crate::prefs::Preferences,
    transport: Arc<dyn Transport>,
    directory: Arc<dyn Directory>,
    quiet: Arc<dyn QuietHours>,
    clock: Clock,
}

/// The messages service: what other crates call to send.
#[derive(Clone)]
pub struct Messages {
    inner: Arc<Inner>,
}

/// Addresses that can never receive mail (RFC 2606 and RFC 6761): sample data uses them.
pub fn reserved_address(address: &str) -> bool {
    let domain = address
        .rsplit('@')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    matches!(
        domain.as_str(),
        "example.com" | "example.org" | "example.net"
    ) || [".example", ".test", ".invalid", ".localhost"]
        .iter()
        .any(|tld| domain.ends_with(tld))
        || ["example", "test", "invalid", "localhost"].contains(&domain.as_str())
}

fn label_for(def: &MessageDef, to: &Recipient, channel: Channel, destination: &str) -> String {
    let who = match def.audience {
        Audience::Customer => "customer",
        Audience::Subscriber => "subscriber",
        Audience::Owner => "owner",
        Audience::Teammate => "teammate",
        Audience::Prospect => "asked Agentrix",
    };
    match channel {
        Channel::Email => match &to.name {
            Some(name) => format!("{name} ({who}) · {destination}"),
            None => format!("{destination} ({who})"),
        },
        Channel::Push => match &to.name {
            Some(name) => format!("{name}'s device"),
            None => "A device with notifications on".to_string(),
        },
        Channel::Chat => {
            let (app, handle) = match destination.strip_prefix("telegram:") {
                Some(rest) => ("Telegram", rest),
                None => (
                    "WhatsApp",
                    destination.strip_prefix("whatsapp:").unwrap_or(destination),
                ),
            };
            match &to.name {
                Some(name) => format!("{name} on {app} · {handle}"),
                None => format!("{app} · {handle}"),
            }
        }
    }
}

fn kind_str(kind: Kind) -> &'static str {
    match kind {
        Kind::Security => "security",
        Kind::Transactional => "transactional",
        Kind::Operational => "operational",
        Kind::Digest => "digest",
    }
}

fn priority_str(p: Priority) -> &'static str {
    match p {
        Priority::P1 => "p1",
        Priority::P2 => "p2",
        Priority::P3 => "p3",
    }
}

fn audience_str(a: Audience) -> &'static str {
    match a {
        Audience::Customer => "customer",
        Audience::Subscriber => "subscriber",
        Audience::Owner => "owner",
        Audience::Teammate => "teammate",
        Audience::Prospect => "prospect",
    }
}

impl Messages {
    /// A messages service over the outbox, with no provider and no quiet hours.
    pub fn new(outbox: Outbox, directory: Arc<dyn Directory>) -> Self {
        Self {
            inner: Arc::new(Inner {
                prefs: crate::prefs::Preferences::new(outbox.pool().clone()),
                outbox,
                transport: Arc::new(Undelivered),
                directory,
                quiet: Arc::new(NeverQuiet),
                clock: Arc::new(Utc::now),
            }),
        }
    }

    fn rebuild(&self, f: impl FnOnce(&mut Inner)) -> Self {
        let mut inner = Inner {
            prefs: self.inner.prefs.clone(),
            outbox: self.inner.outbox.clone(),
            transport: self.inner.transport.clone(),
            directory: self.inner.directory.clone(),
            quiet: self.inner.quiet.clone(),
            clock: self.inner.clock.clone(),
        };
        f(&mut inner);
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Uses `transport` for delivery.
    pub fn with_transport(&self, transport: Arc<dyn Transport>) -> Self {
        self.rebuild(|i| i.transport = transport)
    }

    /// Uses `quiet` for quiet hours.
    pub fn with_quiet_hours(&self, quiet: Arc<dyn QuietHours>) -> Self {
        self.rebuild(|i| i.quiet = quiet)
    }

    /// Uses `clock` for the time.
    pub fn with_clock(&self, clock: Clock) -> Self {
        self.rebuild(|i| i.clock = clock)
    }

    /// The owner's notification preferences.
    pub fn preferences(&self) -> &crate::prefs::Preferences {
        &self.inner.prefs
    }

    /// The outbox.
    pub fn outbox(&self) -> &Outbox {
        &self.inner.outbox
    }

    /// The directory.
    pub fn directory(&self) -> &Arc<dyn Directory> {
        &self.inner.directory
    }

    /// Whether `channel` can be delivered now, and if not, why.
    pub fn readiness(&self, channel: Channel) -> Readiness {
        self.inner.transport.readiness(channel)
    }

    /// Sends a message to the owner of `site_id`, looking up their email.
    pub async fn send_to_site_owner(
        &self,
        site_id: &str,
        message: &str,
        context: Json,
        reason: &str,
        dedupe_key: Option<String>,
    ) -> Result<Sent, SendError> {
        let site = self
            .inner
            .directory
            .site(site_id)
            .await
            .map_err(SendError::Directory)?
            .ok_or_else(|| SendError::Rule(format!("no site {site_id}")))?;
        let account = site
            .owner_account
            .clone()
            .ok_or_else(|| SendError::Rule(format!("site {site_id} has no owner yet")))?;
        self.send_to_owner(
            &account,
            Some(site_id),
            message,
            context,
            reason,
            dedupe_key,
        )
        .await
    }

    /// Sends a message to an owner account, looking up their email.
    pub async fn send_to_owner(
        &self,
        account: &str,
        site_id: Option<&str>,
        message: &str,
        context: Json,
        reason: &str,
        dedupe_key: Option<String>,
    ) -> Result<Sent, SendError> {
        let contact = self
            .inner
            .directory
            .owner(account)
            .await
            .map_err(SendError::Directory)?
            .ok_or_else(|| SendError::Rule(format!("no owner account {account}")))?;
        let first = contact.name.split_whitespace().next().map(str::to_string);
        let to = Recipient {
            name: first,
            email: Some(contact.email),
            account_id: Some(account.to_string()),
            ..Recipient::default()
        };
        self.send(Outgoing {
            message: message.to_string(),
            site_id: site_id.map(str::to_string),
            to,
            context,
            reason: reason.to_string(),
            dedupe_key,
        })
        .await
    }

    /// Records the message in the outbox on every channel it has a destination for, then
    /// delivers each (or holds it for quiet hours, or marks it not sent with the reason).
    pub async fn send(&self, out: Outgoing) -> Result<Sent, SendError> {
        let def = catalogue::get(&out.message)
            .ok_or_else(|| RenderError::Unknown(out.message.clone()))?;
        let mut context = out.context.clone();
        if let Some(obj) = context.as_object_mut()
            && !obj.contains_key("person")
        {
            obj.insert("person".into(), json!({ "name": out.to.name }));
        }
        let consent = context.get("consent").filter(|c| !c.is_null()).cloned();
        if def.audience == crate::catalogue::Audience::Prospect
            && consent
                .as_ref()
                .and_then(|c| c.get("words"))
                .and_then(Json::as_str)
                .is_none_or(|w| w.trim().is_empty())
        {
            return Err(SendError::Rule(format!(
                "{} goes to someone who asked Agentrix, so it needs the words they agreed to (consent.words)",
                def.id
            )));
        }
        if def.kind == Kind::Digest
            && context
                .get("unsubscribe_url")
                .and_then(Json::as_str)
                .is_none()
        {
            return Err(SendError::Rule(format!(
                "{} is something people signed up for, so it needs an unsubscribe_url",
                def.id
            )));
        }
        // Render both copies first, so a broken template records nothing.
        let live = render::render(def.id, &context)?;
        let stored = render::render_stored(def.id, &context)?;

        let now = (self.inner.clock)();
        let quiet_until = match (&out.to.account_id, def.kind, def.priority) {
            (Some(account), Kind::Operational, Priority::P2 | Priority::P3) => {
                self.inner
                    .quiet
                    .quiet_until(account, out.site_id.as_deref(), now)
                    .await
            }
            _ => None,
        };

        let mut entries = Vec::new();
        for &channel in def.channels {
            let destinations: Vec<String> = match channel {
                Channel::Email => out.to.email.iter().cloned().collect(),
                Channel::Push => out.to.push_devices.clone(),
                Channel::Chat => out.to.phone.iter().cloned().collect(),
            };
            let wanted = match &out.to.account_id {
                Some(account) => self.inner.prefs.wants(account, def, channel).await?,
                None => true,
            };
            for destination in destinations {
                let entry = self
                    .record_and_deliver(
                        def,
                        wanted,
                        &out,
                        &context,
                        &live,
                        &stored,
                        channel,
                        &destination,
                        quiet_until,
                        now,
                    )
                    .await?;
                entries.push(entry);
            }
        }
        Ok(Sent { entries })
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_and_deliver(
        &self,
        def: &MessageDef,
        wanted: bool,
        out: &Outgoing,
        context: &Json,
        live: &Rendered,
        stored: &Rendered,
        channel: Channel,
        destination: &str,
        quiet_until: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<Entry, SendError> {
        let (status, status_reason, hold_until) =
            if channel == Channel::Email && reserved_address(destination) {
                (
                    Status::NotSent,
                    Some(format!(
                        "{} is a reserved address for examples, so nothing can be delivered to it",
                        destination.rsplit('@').next().unwrap_or(destination)
                    )),
                    None,
                )
            } else if !wanted {
                (
                    Status::Suppressed,
                    Some("Turned off in Notifications".to_string()),
                    None,
                )
            } else if let Some(until) = quiet_until {
                (
                    Status::Held,
                    Some("Held for quiet hours".to_string()),
                    Some(until),
                )
            } else {
                match self.inner.transport.readiness(channel) {
                    Readiness::Ready => (Status::Queued, None, None),
                    Readiness::NotReady(reason) => (Status::NotSent, Some(reason), None),
                }
            };

        let (subject, body_text, body_html, payload, from_name) = stored_parts(stored, channel);
        let fresh_status = status;
        let new = NewEntry {
            site_id: out.site_id.clone(),
            account_id: out.to.account_id.clone(),
            message: def.id.to_string(),
            audience: audience_str(def.audience).into(),
            kind: kind_str(def.kind).into(),
            priority: priority_str(def.priority).into(),
            channel,
            recipient: destination.to_string(),
            recipient_label: label_for(def, &out.to, channel, destination),
            reason: out.reason.clone(),
            from_name,
            subject,
            body_text,
            body_html,
            payload,
            dedupe_key: out.dedupe_key.clone(),
            status,
            status_reason,
            hold_until,
            consent: context
                .get("consent")
                .filter(|c| !c.is_null())
                .map(|c| c.to_string()),
        };
        let (entry, fresh) = self.inner.outbox.record(new.clone(), now).await?;
        let go = if fresh {
            entry.status == Status::Queued
        } else {
            // Asked for again (a visitor signing up twice) after a try that never left: email
            // was off then, or the provider refused it. It goes now if it can.
            fresh_status == Status::Queued
                && matches!(entry.status, Status::NotSent | Status::Failed)
                && self.inner.outbox.requeue(&entry.id, &new, now).await?
        };
        if !go {
            return Ok(entry);
        }

        let mut envelope = envelope_from(def, out, context, live, channel, destination, &entry.id);
        envelope.attempt = entry.attempts;
        let delivery = self.inner.transport.deliver(&envelope).await;
        self.apply(&entry.id, delivery).await?;
        Ok(self.inner.outbox.get(&entry.id).await?.unwrap_or(entry))
    }

    async fn apply(&self, id: &str, delivery: Delivery) -> Result<(), sqlx::Error> {
        let now = (self.inner.clock)();
        match delivery {
            Delivery::Sent {
                provider,
                provider_ref,
            } => {
                self.inner
                    .outbox
                    .mark(
                        id,
                        Status::Sent,
                        None,
                        Some(&provider),
                        Some(&provider_ref),
                        true,
                        now,
                    )
                    .await
            }
            Delivery::NotSent { reason } => {
                self.inner
                    .outbox
                    .mark(id, Status::NotSent, Some(&reason), None, None, false, now)
                    .await
            }
            Delivery::Failed { reason } => {
                self.inner
                    .outbox
                    .mark(id, Status::Failed, Some(&reason), None, None, true, now)
                    .await
            }
        }
    }

    /// Delivers held messages whose quiet hours have ended. Held messages are never secret
    /// (security messages aren't held), so the stored copy is what goes out.
    pub async fn release_due(&self) -> Result<usize, sqlx::Error> {
        let now = (self.inner.clock)();
        let due = self.inner.outbox.due(now, 100).await?;
        let n = due.len();
        for entry in due {
            match self.inner.transport.readiness(entry.channel) {
                Readiness::NotReady(reason) => {
                    self.inner
                        .outbox
                        .mark(
                            &entry.id,
                            Status::NotSent,
                            Some(&reason),
                            None,
                            None,
                            false,
                            now,
                        )
                        .await?;
                }
                Readiness::Ready => {
                    let envelope = envelope_from_entry(&entry);
                    let delivery = self.inner.transport.deliver(&envelope).await;
                    self.apply(&entry.id, delivery).await?;
                }
            }
        }
        Ok(n)
    }

    /// Once email is switched on, sends what people asked agentrix.website for while it was
    /// off: the report of their check and the weekly fix's confirm link, from the last week.
    /// The page told them it would come. The newest row per message and address goes, from its
    /// stored copy (neither message holds a secret); older copies stay as they were. Returns
    /// how many went to the provider.
    pub async fn send_waiting(&self) -> Result<usize, sqlx::Error> {
        if self.inner.transport.readiness(Channel::Email) != Readiness::Ready {
            return Ok(0);
        }
        let now = (self.inner.clock)();
        let off = match Undelivered.readiness(Channel::Email) {
            Readiness::NotReady(reason) => reason,
            Readiness::Ready => return Ok(0),
        };
        let waiting = self
            .inner
            .outbox
            .waiting_email(&WAITING, &off, now - chrono::Duration::days(7), 100)
            .await?;
        let mut n = 0;
        for entry in waiting {
            if !self.inner.outbox.claim(&entry.id, now).await? {
                continue;
            }
            let delivery = self
                .inner
                .transport
                .deliver(&envelope_from_entry(&entry))
                .await;
            n += usize::from(matches!(delivery, Delivery::Sent { .. }));
            self.apply(&entry.id, delivery).await?;
        }
        Ok(n)
    }
}

/// What [`Messages::send_waiting`] sends once email is on: things a person asked for, with no
/// secret in them.
const WAITING: [&str; 2] = ["newsletter.confirm", "lead.report"];

type StoredParts = (String, String, Option<String>, Option<Json>, String);

fn stored_parts(stored: &Rendered, channel: Channel) -> StoredParts {
    match channel {
        Channel::Email => {
            let e = stored.email.as_ref().expect("email rendered");
            (
                e.subject.clone(),
                e.text.clone(),
                Some(e.html.clone()),
                Some(json!({ "preheader": e.preheader })),
                e.from_name.clone(),
            )
        }
        Channel::Push => {
            let p = stored.push.as_ref().expect("push rendered");
            (
                p.body.clone(),
                p.body.clone(),
                None,
                Some(json!({ "title": p.title, "url": p.url })),
                p.title.clone(),
            )
        }
        Channel::Chat => {
            let c = stored.chat.as_ref().expect("chat rendered");
            (
                c.text.lines().next().unwrap_or_default().to_string(),
                c.text.clone(),
                None,
                Some(json!({ "replies": c.replies, "link": c.link })),
                "Agentrix".to_string(),
            )
        }
    }
}

fn headers_for(def: &MessageDef, context: &Json) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if def.kind == Kind::Digest
        && let Some(url) = context.get("unsubscribe_url").and_then(Json::as_str)
    {
        headers.push(("List-Unsubscribe".into(), format!("<{url}>")));
        headers.push((
            "List-Unsubscribe-Post".into(),
            "List-Unsubscribe=One-Click".into(),
        ));
    }
    if matches!(def.kind, Kind::Security | Kind::Transactional) {
        headers.push(("Auto-Submitted".into(), "auto-generated".into()));
    }
    headers
}

fn envelope_from(
    def: &MessageDef,
    out: &Outgoing,
    context: &Json,
    live: &Rendered,
    channel: Channel,
    destination: &str,
    outbox_id: &str,
) -> Envelope {
    let mut env = Envelope {
        outbox_id: outbox_id.to_string(),
        attempt: 0,
        message: def.id.to_string(),
        site_id: out.site_id.clone(),
        channel,
        to: destination.to_string(),
        to_name: out.to.name.clone(),
        from_name: String::new(),
        subject: String::new(),
        text: String::new(),
        html: None,
        headers: Vec::new(),
        push: None,
        chat: None,
    };
    match channel {
        Channel::Email => {
            let e = live.email.as_ref().expect("email rendered");
            env.from_name = e.from_name.clone();
            env.subject = e.subject.clone();
            env.text = e.text.clone();
            env.html = Some(e.html.clone());
            env.headers = headers_for(def, context);
        }
        Channel::Push => {
            let p = live.push.clone().expect("push rendered");
            env.from_name = p.title.clone();
            env.subject = p.body.clone();
            env.text = p.body.clone();
            env.push = Some(p);
        }
        Channel::Chat => {
            let c = live.chat.clone().expect("chat rendered");
            env.from_name = "Agentrix".into();
            env.subject = c.text.lines().next().unwrap_or_default().to_string();
            env.text = c.text.clone();
            env.chat = Some(c);
        }
    }
    env
}

fn envelope_from_entry(entry: &Entry) -> Envelope {
    let payload = entry.payload.clone().unwrap_or(Json::Null);
    let push = (entry.channel == Channel::Push).then(|| Push {
        title: payload["title"].as_str().unwrap_or("Agentrix").to_string(),
        body: entry.body_text.clone(),
        url: payload["url"].as_str().unwrap_or("/os").to_string(),
    });
    let chat = (entry.channel == Channel::Chat).then(|| Chat {
        text: entry.body_text.clone(),
        replies: payload["replies"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        link: payload["link"].as_array().and_then(|a| {
            Some((
                a.first()?.as_str()?.to_string(),
                a.get(1)?.as_str()?.to_string(),
            ))
        }),
    });
    Envelope {
        outbox_id: entry.id.clone(),
        attempt: entry.attempts,
        message: entry.message.clone(),
        site_id: entry.site_id.clone(),
        channel: entry.channel,
        to: entry.recipient.clone(),
        to_name: None,
        from_name: entry.from_name.clone(),
        subject: entry.subject.clone(),
        text: entry.body_text.clone(),
        html: entry.body_html.clone(),
        headers: catalogue::get(&entry.message)
            .map(|def| {
                if def.kind == Kind::Transactional || def.kind == Kind::Security {
                    vec![("Auto-Submitted".into(), "auto-generated".into())]
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default(),
        push,
        chat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::{Contact, Fixed, SiteRef};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recording {
        sent: Mutex<Vec<Envelope>>,
    }

    #[async_trait]
    impl Transport for Recording {
        fn readiness(&self, _channel: Channel) -> Readiness {
            Readiness::Ready
        }
        async fn deliver(&self, envelope: &Envelope) -> Delivery {
            self.sent.lock().unwrap().push(envelope.clone());
            Delivery::Sent {
                provider: "test".into(),
                provider_ref: format!("ref-{}", envelope.outbox_id),
            }
        }
    }

    fn directory() -> Arc<dyn Directory> {
        Arc::new(Fixed {
            sites: vec![SiteRef {
                id: "site_k".into(),
                name: "Kilele Bakery".into(),
                domain: "kilele-bakery.example".into(),
                owner_account: Some("acct_w".into()),
                utc_offset_minutes: 180,
            }],
            owners: vec![Contact {
                account: "acct_w".into(),
                email: "wanjiru.kamau@gmail.com".into(),
                name: "Wanjiru Kamau".into(),
            }],
        })
    }

    async fn messages() -> Messages {
        Messages::new(Outbox::new(crate::outbox::tests::pool().await), directory())
    }

    fn signin(to: &str) -> Outgoing {
        let def = catalogue::get("customer.signin_code").unwrap();
        Outgoing {
            message: def.id.into(),
            site_id: Some("site_k".into()),
            to: Recipient::email(to, Some("Amina")),
            context: def.example_context(),
            reason: "Amina asked for a sign-in code".into(),
            dedupe_key: None,
        }
    }

    #[tokio::test]
    async fn without_a_provider_messages_are_recorded_as_not_sent() {
        let m = messages().await;
        let sent = m.send(signin("amina@gmail.com")).await.unwrap();
        assert_eq!(sent.entries.len(), 1);
        let e = &sent.entries[0];
        assert_eq!(e.status, Status::NotSent);
        assert_eq!(
            e.status_reason.as_deref(),
            Some("Email isn't switched on yet")
        );
        // The stored copy never holds the code.
        assert!(!e.body_text.contains("482"));
        assert!(!e.body_html.as_deref().unwrap().contains("482"));
    }

    #[tokio::test]
    async fn the_provider_gets_the_live_copy_and_the_outbox_keeps_the_redacted_one() {
        let transport = Arc::new(Recording::default());
        let m = messages().await.with_transport(transport.clone());
        let sent = m.send(signin("amina@gmail.com")).await.unwrap();
        let e = &sent.entries[0];
        assert_eq!(e.status, Status::Sent);
        assert_eq!(
            e.provider_ref.as_deref(),
            Some(format!("ref-{}", e.id).as_str())
        );
        let delivered = transport.sent.lock().unwrap();
        assert!(delivered[0].text.contains("482 913"));
        assert!(!e.body_text.contains("482 913"));
    }

    #[tokio::test]
    async fn reserved_addresses_are_never_delivered() {
        let transport = Arc::new(Recording::default());
        let m = messages().await.with_transport(transport.clone());
        let sent = m.send(signin("amina@example.com")).await.unwrap();
        assert_eq!(sent.entries[0].status, Status::NotSent);
        assert!(
            sent.entries[0]
                .status_reason
                .as_deref()
                .unwrap()
                .contains("example.com")
        );
        assert!(transport.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_same_event_twice_is_one_message() {
        let transport = Arc::new(Recording::default());
        let m = messages().await.with_transport(transport.clone());
        let mut out = signin("amina@gmail.com");
        out.dedupe_key = Some("signin:1".into());
        let a = m.send(out.clone()).await.unwrap();
        let b = m.send(out).await.unwrap();
        assert_eq!(a.entries[0].id, b.entries[0].id);
        assert_eq!(transport.sent.lock().unwrap().len(), 1);
    }

    fn confirm(to: &str, key: &str) -> Outgoing {
        let def = catalogue::get("newsletter.confirm").unwrap();
        Outgoing {
            message: def.id.into(),
            site_id: None,
            to: Recipient::email(to, None),
            context: def.example_context(),
            reason: "A visitor asked for the weekly fix".into(),
            dedupe_key: Some(format!("newsletter.confirm:{key}")),
        }
    }

    fn at(t: DateTime<Utc>) -> Clock {
        Arc::new(move || t)
    }

    #[tokio::test]
    async fn asking_again_once_email_is_on_sends_what_never_left() {
        let off = messages().await;
        let first = off
            .send(confirm("amina@gmail.com", "newsletter:amina@gmail.com"))
            .await
            .unwrap();
        assert_eq!(first.entries[0].status, Status::NotSent);
        let transport = Arc::new(Recording::default());
        let on = off.with_transport(transport.clone());
        let again = on
            .send(confirm("amina@gmail.com", "newsletter:amina@gmail.com"))
            .await
            .unwrap();
        assert_eq!(again.entries[0].id, first.entries[0].id, "still one row");
        assert_eq!(again.entries[0].status, Status::Sent);
        assert_eq!(again.entries[0].status_reason, None);
        // Once sent, asking again sends nothing more.
        on.send(confirm("amina@gmail.com", "newsletter:amina@gmail.com"))
            .await
            .unwrap();
        assert_eq!(transport.sent.lock().unwrap().len(), 1);
    }

    #[derive(Default)]
    struct RefusesFirst {
        attempts: Mutex<Vec<i64>>,
    }

    #[async_trait]
    impl Transport for RefusesFirst {
        fn readiness(&self, _channel: Channel) -> Readiness {
            Readiness::Ready
        }
        async fn deliver(&self, envelope: &Envelope) -> Delivery {
            let mut attempts = self.attempts.lock().unwrap();
            attempts.push(envelope.attempt);
            if attempts.len() == 1 {
                Delivery::Failed {
                    reason: "refused".into(),
                }
            } else {
                Delivery::Sent {
                    provider: "test".into(),
                    provider_ref: "ref-2".into(),
                }
            }
        }
    }

    #[tokio::test]
    async fn a_refused_email_is_tried_again_when_asked_for_again_as_a_new_attempt() {
        let transport = Arc::new(RefusesFirst::default());
        let m = messages().await.with_transport(transport.clone());
        let out = confirm("amina@gmail.com", "newsletter:amina@gmail.com");
        assert_eq!(
            m.send(out.clone()).await.unwrap().entries[0].status,
            Status::Failed
        );
        let again = m.send(out).await.unwrap();
        assert_eq!(again.entries[0].status, Status::Sent);
        assert_eq!(again.entries[0].attempts, 2);
        // The second try carries its attempt, so the provider's idempotency key differs.
        assert_eq!(*transport.attempts.lock().unwrap(), vec![0, 1]);
    }

    #[tokio::test]
    async fn what_people_asked_for_while_email_was_off_goes_once_when_it_is_on() {
        use chrono::TimeZone;
        let now = Utc.with_ymd_and_hms(2026, 11, 8, 17, 0, 0).unwrap();
        let off = messages().await;
        // Too old: a sign-up from last month stays as it was.
        off.with_clock(at(now - chrono::Duration::days(8)))
            .send(confirm("old@gmail.com", "newsletter:old@gmail.com"))
            .await
            .unwrap();
        // The same address twice (the sign-up, then the keep step's tick): only the newest goes.
        off.with_clock(at(now - chrono::Duration::hours(2)))
            .send(confirm("amina@gmail.com", "newsletter:amina@gmail.com"))
            .await
            .unwrap();
        let newest = off
            .with_clock(at(now - chrono::Duration::hours(1)))
            .send(confirm("amina@gmail.com", "asmt_1:amina@gmail.com"))
            .await
            .unwrap();
        // A sign-in code is never sent late.
        off.send(signin("amina@gmail.com")).await.unwrap();
        assert_eq!(off.send_waiting().await.unwrap(), 0, "email is still off");

        let transport = Arc::new(Recording::default());
        let on = off.with_transport(transport.clone()).with_clock(at(now));
        assert_eq!(on.send_waiting().await.unwrap(), 1);
        assert_eq!(on.send_waiting().await.unwrap(), 0, "only once");
        let sent = transport.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].outbox_id, newest.entries[0].id);
        assert_eq!(sent[0].to, "amina@gmail.com");
        assert!(sent[0].text.contains("Confirm the weekly fix"));
        let row = on.outbox().get(&sent[0].outbox_id).await.unwrap().unwrap();
        assert_eq!(row.status, Status::Sent);
    }

    struct QuietUntil(DateTime<Utc>);

    #[async_trait]
    impl QuietHours for QuietUntil {
        async fn quiet_until(
            &self,
            _account: &str,
            _site_id: Option<&str>,
            _now: DateTime<Utc>,
        ) -> Option<DateTime<Utc>> {
            Some(self.0)
        }
    }

    #[test]
    fn quiet_hours_end_at_the_next_morning_in_the_site_s_time() {
        use chrono::{NaiveTime, TimeZone};
        let t = |h| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        // 23:30 in Nairobi (20:30 UTC), quiet 22:00 to 06:00: ends 06:00 Nairobi = 03:00 UTC.
        let now = Utc.with_ymd_and_hms(2026, 11, 8, 20, 30, 0).unwrap();
        assert_eq!(
            quiet_end(t(22), t(6), 180, now),
            Some(Utc.with_ymd_and_hms(2026, 11, 9, 3, 0, 0).unwrap())
        );
        // 02:00 Nairobi: ends the same morning.
        let now = Utc.with_ymd_and_hms(2026, 11, 8, 23, 0, 0).unwrap();
        assert_eq!(
            quiet_end(t(22), t(6), 180, now),
            Some(Utc.with_ymd_and_hms(2026, 11, 9, 3, 0, 0).unwrap())
        );
        // 12:00 Nairobi is day time.
        let now = Utc.with_ymd_and_hms(2026, 11, 8, 9, 0, 0).unwrap();
        assert_eq!(quiet_end(t(22), t(6), 180, now), None);
        // A daytime window (13:00 to 14:00).
        let now = Utc.with_ymd_and_hms(2026, 11, 8, 10, 30, 0).unwrap();
        assert_eq!(
            quiet_end(t(13), t(14), 180, now),
            Some(Utc.with_ymd_and_hms(2026, 11, 8, 11, 0, 0).unwrap())
        );
    }

    #[tokio::test]
    async fn quiet_hours_hold_operational_messages_until_morning_and_never_security() {
        let transport = Arc::new(Recording::default());
        let morning = Utc::now() + chrono::Duration::hours(6);
        let m = messages()
            .await
            .with_transport(transport.clone())
            .with_quiet_hours(Arc::new(QuietUntil(morning)));
        let def = catalogue::get("owner.question_passed").unwrap();
        let sent = m
            .send_to_site_owner(
                "site_k",
                def.id,
                def.example_context(),
                "A visitor's question",
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            sent.entries.len(),
            1,
            "email only: no push device, no WhatsApp"
        );
        assert_eq!(sent.entries[0].status, Status::Held);
        assert_eq!(sent.entries[0].recipient, "wanjiru.kamau@gmail.com");
        assert!(transport.sent.lock().unwrap().is_empty());

        let def = catalogue::get("owner.new_device").unwrap();
        let sent = m
            .send_to_owner(
                "acct_w",
                None,
                def.id,
                def.example_context(),
                "New sign-in",
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            sent.entries[0].status,
            Status::Sent,
            "security alerts aren't held"
        );

        // Morning comes.
        let later = m.with_clock(Arc::new(move || morning + chrono::Duration::minutes(1)));
        assert_eq!(later.release_due().await.unwrap(), 1);
        assert_eq!(transport.sent.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn digests_need_their_stop_link() {
        let m = messages().await;
        let def = catalogue::get("digest.weekly").unwrap();
        let mut ctx = def.example_context();
        ctx.as_object_mut().unwrap().remove("unsubscribe_url");
        let err = m
            .send(Outgoing {
                message: def.id.into(),
                site_id: None,
                to: Recipient::email("reader@gmail.com", None),
                context: ctx,
                reason: "Friday digest".into(),
                dedupe_key: None,
            })
            .await;
        assert!(matches!(err, Err(SendError::Rule(_))));
    }

    #[test]
    fn chat_destinations_name_their_app() {
        let def = catalogue::get("owner.morning_brief").unwrap();
        let to = Recipient {
            name: Some("Wanjiru".into()),
            ..Recipient::default()
        };
        assert_eq!(
            label_for(def, &to, Channel::Chat, "telegram:@wanjiru_k"),
            "Wanjiru on Telegram · @wanjiru_k"
        );
        assert_eq!(
            label_for(def, &to, Channel::Chat, "+254700000000"),
            "Wanjiru on WhatsApp · +254700000000"
        );
    }
}
