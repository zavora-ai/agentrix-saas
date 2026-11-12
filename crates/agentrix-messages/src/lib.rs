//! Every message the platform sends to owners and customers (W9).
//!
//! - [`catalogue`]: the list of messages, who gets each, on which channels, and why.
//! - [`render`]: one template per message, rendered to plain text and HTML email, a push line
//!   and a chat message, all from the same words.
//! - [`lint`]: the copy check from `specs/platform/voice.md`, run on every example.
//!
//! # Context every message can use
//!
//! | Key | Meaning |
//! |---|---|
//! | `site` | `{name, host, sample, owner, accent, url}`: the site the message is about. Customer messages are sent in its name; `sample: true` adds the sample band |
//! | `person` | `{name}`: the recipient's first name, or `null` |
//! | `console_url` | For owner messages: `{origin}/console/{site}` when the message is about one site, `{origin}/os` when it's about the account. Printed as it is, never with a path added |
//! | `unsubscribe_url` | Required for digest messages: the one-tap stop link |
//! | `preferences_url` | Optional for owner messages: where they choose channels and quiet hours |
//!
//! Each message's own keys are in its example beside the template
//! (`templates/<audience>/<name>.json`). A missing key is an error, never a blank. A link to
//! a page deeper than the console (billing, People, an order) is its own key, and every link
//! must be a page that exists: the tests in `links.rs` check every template and builder
//! against the routes listed there.
//!
//! # Sending from another crate
//!
//! Build the service once in your `mount` with [`from_context`] and call
//! [`Messages::send`] (any recipient) or [`Messages::send_to_site_owner`] (looks up the owner).
//! Every message is recorded in the outbox before delivery; until an email provider is set up,
//! it is kept there as "Not sent" and the owner sees it on the console's Messages page.

pub mod app;
pub mod catalogue;
pub mod channels;
pub mod directory;
pub mod email;
pub mod identity;
#[cfg(test)]
mod links;
pub mod lint;
pub mod markup;
pub mod orders;
pub mod outbox;
pub mod prefs;
pub mod prospects;
pub mod quiet;
pub mod render;
pub mod send;
pub mod web;

use std::sync::Arc;
use std::time::Duration;

use agentrix_host::{Mounted, NodeContext};

pub use catalogue::{Audience, Channel, Family, Kind, MessageDef, Priority};
pub use channels::register_chat_transport;
pub use outbox::{Entry, Outbox, Status};
pub use render::{Chat, Email, Push, RenderError, Rendered, render, render_stored};
pub use send::{
    Delivery, Envelope, Messages, Outgoing, QuietHours, Readiness, Recipient, SendError, Sent,
    Transport, Undelivered, quiet_end,
};

/// Runs this crate's migrations (W9's range, 1000–1099) with their own table, before node
/// mounts the crate. Node lists it in its `MIGRATIONS`.
pub fn migrate(pool: &sqlx::SqlitePool) -> futures::future::BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let mut m = sqlx::migrate!("./migrations");
        m.set_ignore_missing(true);
        m.dangerous_set_table_name("_sqlx_migrations_agentrix_messages");
        m.run(pool)
            .await
            .map_err(|e| anyhow::anyhow!("agentrix-messages migrations: {e}"))?;
        Ok(())
    })
}

/// The messages service on node's `control.db`, configured the same way for every crate that
/// sends. Cheap to call; clones share nothing but the database pool.
pub fn from_context(ctx: &NodeContext) -> Messages {
    let directory: Arc<dyn directory::Directory> =
        Arc::new(directory::ControlDb::new(ctx.pool.clone()));
    let messages = Messages::new(Outbox::new(ctx.pool.clone()), directory.clone())
        .with_quiet_hours(Arc::new(quiet::ShellQuietHours::new(
            agentrix_shell::store::ShellStore::new(ctx.pool.clone()),
            directory,
        )));
    let email_transport: Arc<dyn Transport> = match email::EmailConfig::from_env() {
        Some(cfg) => Arc::new(email::ResendTransport::new(ctx.http.clone(), cfg)),
        None => Arc::new(Undelivered),
    };
    messages.with_transport(Arc::new(channels::ChannelRouter::new(email_transport)))
}

/// The Messages app for W5's shell (contract C9): node passes it to
/// `agentrix_shell::mount_with`.
pub fn console_app(ctx: &NodeContext) -> Arc<dyn agentrix_shell::apps::ConsoleApp> {
    Arc::new(app::MessagesApp::new(
        from_context(ctx),
        agentrix_shell::store::ShellStore::new(ctx.pool.clone()),
    ))
}

/// W6's mailer for customer sign-in codes and notices, recorded in the outbox and sent through
/// the catalogue. Node passes it to `Visitors::with_mailer`.
pub fn identity_mailer(ctx: &NodeContext) -> Arc<dyn agentrix_identity::CodeMailer> {
    Arc::new(identity::IdentityMailer::new(from_context(ctx)))
}

/// Mounts the console's Messages page on the platform host and the task that delivers
/// messages held for quiet hours once they end (and, at start, what people asked for while
/// email was off: [`Messages::send_waiting`]).
pub fn mount(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    let messages = from_context(ctx);
    let routes = web::routes(web::WebState {
        messages: messages.clone(),
        owners: ctx.owners.clone(),
    });
    let release = messages.clone();
    let routes = match email::EmailConfig::from_env().and_then(|c| c.webhook_secret) {
        Some(secret) => routes.merge(email::webhook_routes(messages.outbox().clone(), secret)),
        None => routes,
    };
    Ok(Mounted::default().platform(routes).task(
        "messages: deliver held messages when quiet hours end",
        Box::pin(async move {
            // What people asked for while email was off goes once, when node starts with it on.
            match release.send_waiting().await {
                Ok(0) => {}
                Ok(n) => tracing::info!(sent = n, "sent emails kept while email was off"),
                Err(e) => {
                    tracing::warn!(error = %e, "couldn't send emails kept while email was off")
                }
            }
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tick.tick().await;
                if let Err(e) = release.release_due().await {
                    tracing::warn!(error = %e, "couldn't release held messages");
                }
            }
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn texts(r: &Rendered) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(e) = &r.email {
            out.push(e.subject.clone());
            out.push(e.preheader.clone());
            out.push(e.text.clone());
        }
        if let Some(p) = &r.push {
            out.push(p.body.clone());
        }
        if let Some(c) = &r.chat {
            out.push(c.text.clone());
            out.extend(c.replies.iter().cloned());
        }
        out
    }

    #[test]
    fn every_message_renders_its_example_on_every_channel() {
        let mut failures = Vec::new();
        for def in catalogue::all() {
            match render(def.id, &def.example_context()) {
                Ok(r) => {
                    for ch in def.channels {
                        let present = match ch {
                            Channel::Email => r.email.is_some(),
                            Channel::Push => r.push.is_some(),
                            Channel::Chat => r.chat.is_some(),
                        };
                        if !present {
                            failures.push(format!("{}: no {:?} version", def.id, ch));
                        }
                    }
                }
                Err(e) => failures.push(format!("{e:#}")),
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn every_message_passes_the_copy_check() {
        let mut failures = Vec::new();
        for def in catalogue::all() {
            let r = render(def.id, &def.example_context()).expect("renders");
            for text in texts(&r) {
                for p in lint::check(&text) {
                    failures.push(format!("{}: {} ({:?})", def.id, p.rule, p.found));
                }
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn ids_are_unique_and_match_their_audience() {
        let mut seen = std::collections::HashSet::new();
        for def in catalogue::all() {
            assert!(seen.insert(def.id), "{} twice", def.id);
            let prefix = def.id.split('.').next().unwrap();
            let expected: &[&str] = match def.audience {
                Audience::Customer => &["customer"],
                Audience::Subscriber => &["digest"],
                Audience::Owner => &["owner"],
                Audience::Teammate => &["teammate"],
                Audience::Prospect => &["lead", "newsletter"],
            };
            assert!(expected.contains(&prefix), "{}", def.id);
        }
    }

    #[test]
    fn subjects_and_headings_are_short_enough_to_read_on_a_phone() {
        let mut failures = Vec::new();
        for def in catalogue::all() {
            let r = render(def.id, &def.example_context()).unwrap();
            if let Some(e) = &r.email {
                let n = e.subject.chars().count();
                if n > 78 {
                    failures.push(format!("{}: subject is {n} characters", def.id));
                }
            }
            if let Some(p) = &r.push
                && p.body.chars().count() > 80
            {
                failures.push(format!("{}: push is too long", def.id));
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn secrets_never_reach_the_stored_copy() {
        for def in catalogue::all().iter().filter(|d| !d.secret.is_empty()) {
            let ctx = def.example_context();
            let code = ctx["code"].as_str().unwrap().to_string();
            let spaced = markup::spaced_code(&code);
            let live = render(def.id, &ctx).unwrap();
            let stored = render_stored(def.id, &ctx).unwrap();
            let live_text = texts(&live).join("\n");
            assert!(
                live_text.contains(&spaced) || live_text.contains(&code),
                "{}: code missing",
                def.id
            );
            for text in texts(&stored) {
                assert!(
                    !text.contains(&code) && !text.contains(&spaced),
                    "{}: code stored",
                    def.id
                );
            }
            let html = stored.email.unwrap().html;
            assert!(
                !html.contains(&code) && !html.contains(&spaced),
                "{}: code in stored HTML",
                def.id
            );
        }
    }

    #[test]
    fn a_missing_value_is_an_error_not_a_blank() {
        let mut ctx = catalogue::get("customer.signin_code")
            .unwrap()
            .example_context();
        ctx.as_object_mut().unwrap().remove("minutes");
        assert!(render("customer.signin_code", &ctx).is_err());
        ctx["minutes"] = json!(null);
        assert!(render("customer.signin_code", &ctx).is_err());
    }

    #[test]
    fn the_morning_brief_says_this_months_ai_spend_as_the_console_does() {
        let def = catalogue::get("owner.morning_brief").unwrap();
        let mut ctx = def.example_context();
        let r = render(def.id, &ctx).unwrap();
        let line = "AI spend KSh 1,240 of KSh 3,870 this month.";
        assert!(r.chat.unwrap().text.contains(line));
        let e = r.email.unwrap();
        assert!(e.preheader.contains(line), "{}", e.preheader);
        assert!(e.text.contains("KSh 1,240 of KSh 3,870 this month"));
        assert!(!e.text.contains("USD"), "{}", e.text);

        // No monthly budget set: the spend alone.
        ctx["brief"]["spend"]["budget"] = json!(null);
        ctx["brief"]["spend"]["words"] = json!("KSh 1,240 this month");
        let chat = render(def.id, &ctx).unwrap().chat.unwrap().text;
        assert!(chat.contains("AI spend KSh 1,240 this month."), "{chat}");

        // Spend that couldn't be read is left out, never guessed.
        ctx["brief"]["spend"] = json!({ "used": null, "budget": null, "currency": null, "period": "month", "words": null });
        let r = render(def.id, &ctx).unwrap();
        assert!(!r.chat.unwrap().text.contains("AI spend"));
        assert!(!r.email.unwrap().text.contains("AI spend"));
    }

    #[test]
    fn chat_never_decides_money_or_high_risk() {
        let def = catalogue::get("owner.decision_waiting").unwrap();
        let mut ctx = def.example_context();
        let medium = render(def.id, &ctx).unwrap().chat.unwrap();
        assert_eq!(medium.replies, vec!["Decline"], "chat never approves");
        assert!(medium.link.is_some());
        ctx["card"]["money"] = json!(true);
        let money = render(def.id, &ctx).unwrap().chat.unwrap();
        assert!(money.replies.is_empty(), "money: {:?}", money.replies);
        assert!(money.link.is_some());
        ctx["card"]["money"] = json!(false);
        ctx["card"]["risk"] = json!("high");
        let high = render(def.id, &ctx).unwrap().chat.unwrap();
        assert!(high.replies.is_empty());
    }

    #[test]
    fn push_is_a_doorbell() {
        // The lock screen never shows what a decision is about (05 §7).
        let def = catalogue::get("owner.decision_waiting").unwrap();
        let ctx = def.example_context();
        let push = render(def.id, &ctx).unwrap().push.unwrap();
        assert_eq!(push.body, "A decision is waiting.");
        assert_eq!(push.title, "Kilele · Agentrix");
        assert!(!push.body.contains(ctx["card"]["title"].as_str().unwrap()));
    }

    #[test]
    fn sample_sites_say_sample_and_real_sites_dont() {
        let def = catalogue::get("customer.order_received").unwrap();
        let mut ctx = def.example_context();
        let sample = render(def.id, &ctx).unwrap().email.unwrap();
        assert!(sample.text.starts_with("Sample:"));
        assert!(sample.html.contains("t a real business. It"));
        assert_eq!(sample.from_name, "Kilele Bakery (sample)");
        ctx["site"]["sample"] = json!(false);
        let real = render(def.id, &ctx).unwrap().email.unwrap();
        assert!(!real.text.starts_with("Sample:"));
        assert!(!real.html.contains("a real business"));
        assert_eq!(real.from_name, "Kilele Bakery");
    }

    #[test]
    fn digests_carry_their_stop_link() {
        let def = catalogue::get("digest.weekly").unwrap();
        let ctx = def.example_context();
        let e = render(def.id, &ctx).unwrap().email.unwrap();
        assert!(e.text.contains(ctx["unsubscribe_url"].as_str().unwrap()));
    }

    #[test]
    fn customer_emails_link_only_to_their_site_and_owners_get_our_whatsapp() {
        let def = catalogue::get("customer.order_received").unwrap();
        let mut ctx = def.example_context();
        let e = render(def.id, &ctx).unwrap().email.unwrap();
        assert!(
            !e.text.contains("wa.me"),
            "a site's customers never get Zavora's contact"
        );
        ctx["site"]["contact_url"] = json!("https://kilele-bakery-sample.agentrix.website/visit");
        let e = render(def.id, &ctx).unwrap().email.unwrap();
        assert!(e.text.contains(
            "Questions? Ask Kilele Bakery: https://kilele-bakery-sample.agentrix.website/visit"
        ));
        ctx["site"]["contact_url"] = json!("https://wa.me/254700000000");
        assert!(
            render(def.id, &ctx).is_err(),
            "off-site links break the footer's promise"
        );
        ctx["site"]["contact_url"] = json!(null);
        ctx["order"]["url"] = json!("https://evil.example/pay");
        assert!(render(def.id, &ctx).is_err());

        let owner = catalogue::get("owner.morning_brief").unwrap();
        let e = render(owner.id, &owner.example_context())
            .unwrap()
            .email
            .unwrap();
        assert!(
            e.text
                .contains("Questions? Message us on WhatsApp: https://wa.me/254721490933")
        );
        let lead = catalogue::get("lead.report").unwrap();
        let e = render(lead.id, &lead.example_context())
            .unwrap()
            .email
            .unwrap();
        assert!(
            e.text
                .contains("Questions? Message us on WhatsApp: https://wa.me/254721490933")
        );
    }
}
