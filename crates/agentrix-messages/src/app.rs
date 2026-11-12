//! The Messages app in W5's shell (contract C9): the outbox for one site, one message, every
//! message the platform can send, and the owner's notification settings.
//!
//! Node registers it: `agentrix_shell::mount_with(ctx, vec![agentrix_messages::console_app(ctx)])`.
//! The shell signs the owner in, checks they own the site, and wraps the field this app draws.
//! Paths under `/console/{site}/apps/messages`: `""` (sent and not sent), `m/{id}`,
//! `catalogue`, `notifications` (GET and POST).

use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::HeaderMap;
use chrono::{FixedOffset, Utc};
use serde::Serialize;

use agentrix_shell::apps::{AppContext, AppPage, AppReply, AppSpec, ConsoleApp};
use agentrix_shell::store::ShellStore;

use crate::catalogue::{self, Channel};
use crate::outbox::{Filter, Outbox, Status};
use crate::prefs::{self, TUNABLE};
use crate::send::{Messages, Readiness};
use crate::web::{Fact, catalogue_groups, env, row, urlencode, when};

/// The Messages app.
pub struct MessagesApp {
    messages: Messages,
    shell: ShellStore,
}

impl MessagesApp {
    /// Over the messages service and the shell's store (for quiet hours).
    pub fn new(messages: Messages, shell: ShellStore) -> Self {
        Self { messages, shell }
    }
}

#[derive(Serialize)]
struct Tab {
    label: String,
    href: String,
    current: bool,
}

fn tabs(base: &str, current: &str) -> Vec<Tab> {
    [
        ("Sent and not sent", ""),
        ("Notifications", "notifications"),
        ("Every message", "catalogue"),
    ]
    .iter()
    .map(|(label, path)| Tab {
        label: (*label).to_string(),
        href: if path.is_empty() {
            base.to_string()
        } else {
            format!("{base}/{path}")
        },
        current: *path == current,
    })
    .collect()
}

fn query_value(query: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn offset(ctx: &AppContext) -> FixedOffset {
    FixedOffset::east_opt(ctx.utc_offset_minutes * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero"))
}

fn render(name: &str, ctx: impl Serialize) -> anyhow::Result<String> {
    Ok(env().get_template(name)?.render(ctx)?)
}

impl MessagesApp {
    async fn list(&self, ctx: &AppContext, query: &str) -> anyhow::Result<AppPage> {
        let status = query_value(query, "status")
            .as_deref()
            .and_then(Status::parse);
        let before = query_value(query, "before")
            .and_then(|b| chrono::DateTime::parse_from_rfc3339(&b).ok())
            .map(|t| t.with_timezone(&Utc));
        let sites = vec![ctx.site_id.clone()];
        let outbox = self.messages.outbox();
        let filter = Filter {
            status,
            channel: None,
            before,
            limit: Some(50),
        };
        let entries = outbox.list_for_owner(&ctx.account, &sites, &filter).await?;
        let counts = outbox.counts_for_owner(&ctx.account, &sites).await?;
        let now = Utc::now();
        let rows: Vec<_> = entries.iter().map(|e| row(e, offset(ctx), now)).collect();
        let older = (entries.len() == 50).then(|| {
            let last = entries.last().expect("50 rows").created_at.to_rfc3339();
            let mut href = format!("{}?before={}", ctx.base, urlencode(&last));
            if let Some(s) = status {
                href.push_str(&format!("&status={}", s.as_str()));
            }
            href
        });
        let chip = |label: &str, n: i64, s: Option<Status>| Tab {
            label: format!("{label} · {n}"),
            href: match s {
                Some(s) => format!("{}?status={}", ctx.base, s.as_str()),
                None => ctx.base.clone(),
            },
            current: s == status,
        };
        let mut chips = vec![
            chip("All", counts.all, None),
            chip("Sent", counts.sent, Some(Status::Sent)),
            chip("Not sent", counts.not_sent, Some(Status::NotSent)),
        ];
        if counts.held > 0 || status == Some(Status::Held) {
            chips.push(chip("Held", counts.held, Some(Status::Held)));
        }
        if counts.failed > 0 || status == Some(Status::Failed) {
            chips.push(chip("Failed", counts.failed, Some(Status::Failed)));
        }
        let html = render(
            "messages/app_list.html",
            minijinja::context! {
                site_name => ctx.site_name.clone(),
                base => ctx.base.clone(),
                tabs => tabs(&ctx.base, ""),
                chips => chips,
                rows => rows,
                older => older,
                filtered => status.is_some(),
                email_ready => self.messages.readiness(Channel::Email) == Readiness::Ready,
            },
        )?;
        Ok(AppPage::Field {
            title: "Messages".into(),
            html,
        })
    }

    async fn detail(&self, ctx: &AppContext, id: &str) -> anyhow::Result<AppPage> {
        let Some(e) = self.messages.outbox().get(id).await? else {
            return Ok(AppPage::NotFound);
        };
        if !Outbox::visible_to(&e, &ctx.account, std::slice::from_ref(&ctx.site_id)) {
            return Ok(AppPage::NotFound);
        }
        let now = Utc::now();
        let off = offset(ctx);
        let def = catalogue::get(&e.message);
        let m = row(&e, off, now);
        let mut facts = vec![
            Fact::new("To", &m.to),
            Fact::new("Channel", m.channel),
            Fact::new("From", &e.from_name),
            Fact::new("Why", &e.reason),
        ];
        if let Some(d) = def {
            facts.push(Fact::new("Sent when", d.trigger));
            facts.push(Fact::new("Kind", d.kind.label()));
        }
        facts.push(Fact::new("Recorded", &when(e.created_at, off, now)));
        if let Some(t) = e.hold_until {
            facts.push(Fact::new("Held until", &when(t, off, now)));
        }
        if let Some(t) = e.sent_at {
            let by = e
                .provider
                .as_deref()
                .map(|p| format!(" by {p}"))
                .unwrap_or_default();
            facts.push(Fact::new("Sent", &format!("{}{by}", when(t, off, now))));
        }
        let details = crate::web::details_for(&e);
        let replies: Vec<String> = e
            .payload
            .as_ref()
            .and_then(|p| p.get("replies"))
            .and_then(|r| r.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let title = e.subject.clone();
        let html = render(
            "messages/app_message.html",
            minijinja::context! {
                base => ctx.base.clone(),
                m => m,
                facts => facts,
                details => details,
                channel_key => e.channel.as_str(),
                from_name => e.from_name.clone(),
                body_text => e.body_text.clone(),
                has_html => e.body_html.is_some(),
                replies => replies,
            },
        )?;
        Ok(AppPage::Field { title, html })
    }

    fn catalogue(&self, ctx: &AppContext) -> anyhow::Result<AppPage> {
        let html = render(
            "messages/app_catalogue.html",
            minijinja::context! {
                tabs => tabs(&ctx.base, "catalogue"),
                groups => catalogue_groups(),
                count => catalogue::all().len(),
            },
        )?;
        Ok(AppPage::Field {
            title: "Every message".into(),
            html,
        })
    }

    async fn notifications(&self, ctx: &AppContext, query: &str) -> anyhow::Result<AppPage> {
        let families = self
            .messages
            .preferences()
            .for_account(&ctx.account)
            .await?;
        let quiet = self.shell.quiet_hours(&ctx.site_id).await?;
        let state = |c: Channel| match self.messages.readiness(c) {
            Readiness::Ready => "On".to_string(),
            Readiness::NotReady(reason) => reason,
        };
        #[derive(Serialize)]
        struct Col {
            label: &'static str,
            state: String,
            ready: bool,
        }
        let channels: Vec<Col> = Channel::ALL
            .iter()
            .map(|c| Col {
                label: c.label(),
                state: state(*c),
                ready: self.messages.readiness(*c) == Readiness::Ready,
            })
            .collect();
        let not_on: Vec<&Col> = channels.iter().filter(|c| !c.ready).collect();
        let html = render(
            "messages/app_notifications.html",
            minijinja::context! {
                site_name => ctx.site_name.clone(),
                base => ctx.base.clone(),
                console => ctx.console.clone(),
                tabs => tabs(&ctx.base, "notifications"),
                families => families,
                not_on => not_on,
                channels => channels,
                saved => query_value(query, "saved").is_some(),
                quiet_words => quiet.words(),
                whatsapp_state => state(Channel::Chat),
                push_state => state(Channel::Push),
            },
        )?;
        Ok(AppPage::Field {
            title: "Notifications".into(),
            html,
        })
    }

    async fn save_notifications(&self, ctx: &AppContext, body: &[u8]) -> anyhow::Result<AppReply> {
        let on: Vec<String> = url::form_urlencoded::parse(body)
            .map(|(k, _)| k.into_owned())
            .collect();
        for family in TUNABLE {
            let key = prefs::family_key(family);
            for channel in Channel::ALL {
                let field = format!("{key}.{}", channel.as_str());
                self.messages
                    .preferences()
                    .set(&ctx.account, family, channel, on.contains(&field))
                    .await?;
            }
        }
        Ok(AppReply::Redirect(format!(
            "{}/notifications?saved=1",
            ctx.base
        )))
    }
}

#[async_trait]
impl ConsoleApp for MessagesApp {
    fn spec(&self) -> AppSpec {
        AppSpec {
            id: "messages".into(),
            name: "Messages".into(),
            title: "Messages: what your site sent to customers and to you, and why".into(),
            icon: "bell".into(),
            webagent: None,
            order: 90,
        }
    }

    async fn badge(&self, ctx: &AppContext) -> anyhow::Result<u32> {
        let counts = self
            .messages
            .outbox()
            .counts_for_owner(&ctx.account, std::slice::from_ref(&ctx.site_id))
            .await?;
        Ok(u32::try_from(counts.failed).unwrap_or(u32::MAX))
    }

    async fn page(&self, ctx: &AppContext, path: &str, query: &str) -> anyhow::Result<AppPage> {
        match path.trim_end_matches('/') {
            "" => self.list(ctx, query).await,
            "catalogue" => self.catalogue(ctx),
            "notifications" => self.notifications(ctx, query).await,
            p => match p.strip_prefix("m/") {
                Some(id) if !id.contains('/') => self.detail(ctx, id).await,
                _ => Ok(AppPage::NotFound),
            },
        }
    }

    async fn post(
        &self,
        ctx: &AppContext,
        path: &str,
        _headers: &HeaderMap,
        body: Bytes,
    ) -> anyhow::Result<AppReply> {
        match path {
            "notifications" => self.save_notifications(ctx, &body).await,
            _ => Ok(AppReply::NotFound),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::Fixed;
    use crate::send::{Outgoing, Recipient};
    use std::sync::Arc;

    fn ctx() -> AppContext {
        AppContext {
            site_id: "site_k".into(),
            site_name: "Kilele Bakery".into(),
            host: "kilele-bakery-sample.agentrix.website".into(),
            sample: true,
            account: "acct_w".into(),
            owner_name: "Wanjiru".into(),
            utc_offset_minutes: 180,
            base: "/console/site_k/apps/messages".into(),
            console: "/console/site_k".into(),
        }
    }

    async fn app() -> (MessagesApp, Messages) {
        let pool = crate::outbox::tests::pool().await;
        let shell = ShellStore::open(pool.clone()).await.unwrap();
        let messages = Messages::new(Outbox::new(pool), Arc::new(Fixed::default()));
        (MessagesApp::new(messages.clone(), shell), messages)
    }

    fn field(page: AppPage) -> String {
        match page {
            AppPage::Field { html, .. } => html,
            other => panic!("not a field: {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_app_lists_shows_and_explains_messages() {
        let (app, messages) = app().await;
        let empty = field(app.page(&ctx(), "", "").await.unwrap());
        assert!(empty.contains("No messages yet"));
        let def = catalogue::get("customer.order_received").unwrap();
        let sent = messages
            .send(Outgoing {
                message: def.id.into(),
                site_id: Some("site_k".into()),
                to: Recipient::email("njeri@gmail.com", Some("Njeri")),
                context: def.example_context(),
                reason: "Njeri paid for order KB-1042".into(),
                dedupe_key: None,
            })
            .await
            .unwrap();
        let list = field(app.page(&ctx(), "", "").await.unwrap());
        assert!(list.contains("Kilele Bakery has your order KB-1042"));
        assert!(list.contains("Email isn't switched on yet"));
        let id = &sent.entries[0].id;
        let one = field(app.page(&ctx(), &format!("m/{id}"), "").await.unwrap());
        assert!(one.contains("Njeri paid for order KB-1042"));
        assert!(one.contains(&format!("/console/messages/{id}/view")));
        // Another site's console can't open it.
        let mut other = ctx();
        other.site_id = "site_x".into();
        other.account = "acct_x".into();
        assert!(matches!(
            app.page(&other, &format!("m/{id}"), "").await.unwrap(),
            AppPage::NotFound
        ));
        assert!(field(app.page(&ctx(), "catalogue", "").await.unwrap()).contains("Every message"));
    }

    #[tokio::test]
    async fn notifications_save_and_show() {
        let (app, messages) = app().await;
        let page = field(app.page(&ctx(), "notifications", "").await.unwrap());
        assert!(page.contains("Always sent"));
        assert!(page.contains("22:00"));
        // Turn everything off except email for running the site.
        let reply = app
            .post(
                &ctx(),
                "notifications",
                &HeaderMap::new(),
                Bytes::from_static(b"owner_operations.email=on"),
            )
            .await
            .unwrap();
        assert!(
            matches!(reply, AppReply::Redirect(ref to) if to.ends_with("/notifications?saved=1"))
        );
        let brief = catalogue::get("owner.morning_brief").unwrap();
        let prefs = messages.preferences();
        assert!(prefs.wants("acct_w", brief, Channel::Email).await.unwrap());
        assert!(!prefs.wants("acct_w", brief, Channel::Push).await.unwrap());
        let page = field(app.page(&ctx(), "notifications", "saved=1").await.unwrap());
        assert!(page.contains("Saved."));
    }
}
