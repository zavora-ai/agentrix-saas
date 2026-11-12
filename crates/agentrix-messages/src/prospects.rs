//! Messages to people who asked Agentrix itself for something on agentrix.website (W10): their
//! website check report, and the weekly fix (double opt-in, then one fix a week).
//!
//! Each send carries the exact words the person agreed to ([`Consent`]); the outbox keeps them
//! with the row, and sending refuses a prospect message without them. W10 keeps the contact and
//! the consent in agentrix-home's own table and calls these by name:
//!
//! ```no_run
//! # async fn demo(messages: agentrix_messages::Messages) -> Result<(), agentrix_messages::SendError> {
//! use agentrix_messages::prospects::{Consent, Report, lead_report};
//! let consent = Consent {
//!     words: "Email me the report".into(),
//!     given_at: "Sun 8 Nov, 10:12".into(),
//!     source: "agentrix.website check".into(),
//! };
//! let report = Report {
//!     site: "kilele-bakery.example".into(),
//!     checked_at: "Sun 8 Nov, 10:12".into(),
//!     headline: "We read 41 pages. Three things to fix first.".into(),
//!     findings: vec!["Your opening hours differ on two pages.".into()],
//!     report_url: "https://agentrix.website/assess/r/a_7k2m9q".into(),
//!     keep_url: "https://agentrix.website/start?from=a_7k2m9q".into(),
//!     expires: "Tue 8 Dec".into(),
//! };
//! messages.send(lead_report("wanjiru@example.com", Some("Wanjiru"), &report, &consent)).await?;
//! # Ok(()) }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::send::{Outgoing, Recipient};

/// What the person agreed to, in their own words on the page, and when and where.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consent {
    /// The words beside the box or button they used: "Email me the report".
    pub words: String,
    /// When, in words: "Sun 8 Nov, 10:12".
    pub given_at: String,
    /// Where: "agentrix.website check", "agentrix.website footer".
    pub source: String,
}

/// A website check, as the report page shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// The address checked: "kilele-bakery.example".
    pub site: String,
    /// When it was read: "Sun 8 Nov, 10:12".
    pub checked_at: String,
    /// One or two sentences: "We read 41 pages. Three things to fix first."
    pub headline: String,
    /// Up to three findings, one line each.
    pub findings: Vec<String>,
    /// The report page (kept 30 days).
    pub report_url: String,
    /// The one next step: keep the new site.
    pub keep_url: String,
    /// When the report page goes: "Tue 8 Dec".
    pub expires: String,
}

/// One issue of the weekly fix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    /// The fix, as a short instruction: "Put your opening hours where Google can read them".
    pub title: String,
    /// Five lines or fewer.
    pub lines: Vec<String>,
    /// An optional link with its words.
    #[serde(default)]
    pub link: Option<(String, String)>,
}

/// Why an issue can't be sent.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IssueError {
    /// The weekly fix promises five lines or fewer.
    #[error("the weekly fix is five lines or fewer, and this has {0}")]
    TooLong(usize),
    /// Nothing to say.
    #[error("the weekly fix needs at least one line")]
    Empty,
}

fn recipient(destination: &str, name: Option<&str>) -> Recipient {
    match destination.strip_prefix("whatsapp:") {
        Some(phone) => Recipient {
            name: name.map(str::to_string),
            phone: Some(phone.to_string()),
            ..Recipient::default()
        },
        None => Recipient::email(destination, name),
    }
}

/// The website check report, to the email the person gave (or `whatsapp:+254…` once the
/// utility template is approved).
pub fn lead_report(to: &str, name: Option<&str>, report: &Report, consent: &Consent) -> Outgoing {
    let findings: Vec<&String> = report.findings.iter().take(3).collect();
    Outgoing {
        message: "lead.report".into(),
        site_id: None,
        to: recipient(to, name),
        context: json!({
            "check": {
                "site": report.site, "checked_at": report.checked_at, "headline": report.headline,
                "findings": findings, "report_url": report.report_url, "keep_url": report.keep_url,
                "expires": report.expires,
            },
            "consent": consent,
        }),
        reason: format!(
            "Asked for the website check of {} at agentrix.website",
            report.site
        ),
        dedupe_key: Some(format!("lead.report:{}:{}", report.report_url, to)),
    }
}

/// The weekly fix's double opt-in: an email with a confirm link, or a WhatsApp message asking
/// for YES.
pub fn newsletter_confirm(to: &str, confirm_url: &str, day: &str, consent: &Consent) -> Outgoing {
    Outgoing {
        message: "newsletter.confirm".into(),
        site_id: None,
        to: recipient(to, None),
        context: json!({
            "newsletter": { "day": day },
            "confirm_url": confirm_url,
            "forget_after": "7 days",
            "consent": consent,
        }),
        reason: "Asked for the weekly fix at agentrix.website".into(),
        dedupe_key: None,
    }
}

/// One issue to one confirmed reader. `stop_url` must stop with one click (Gmail and Yahoo's
/// bulk-sender rules): it goes in the List-Unsubscribe header and accepts a POST.
pub fn newsletter_issue(
    to: &str,
    issue: &Issue,
    issue_key: &str,
    stop_url: &str,
    consent: &Consent,
) -> Result<Outgoing, IssueError> {
    match issue.lines.len() {
        0 => return Err(IssueError::Empty),
        n if n > 5 => return Err(IssueError::TooLong(n)),
        _ => {}
    }
    Ok(Outgoing {
        message: "newsletter.issue".into(),
        site_id: None,
        to: recipient(to, None),
        context: json!({
            "issue": {
                "title": issue.title,
                "lines": issue.lines,
                "link": issue.link.as_ref().map(|(label, url)| json!({ "label": label, "url": url })),
            },
            "unsubscribe_url": stop_url,
            "consent": consent,
        }),
        reason: format!("The weekly fix: {}", issue.title),
        dedupe_key: Some(format!("newsletter.issue:{issue_key}:{to}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::Fixed;
    use crate::outbox::Outbox;
    use crate::send::{Delivery, Envelope, Messages, Readiness, SendError, Transport};
    use crate::{Channel, render};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    fn consent() -> Consent {
        Consent {
            words: "Send me the weekly fix".into(),
            given_at: "Sun 8 Nov, 10:20".into(),
            source: "agentrix.website footer".into(),
        }
    }

    fn issue(lines: usize) -> Issue {
        Issue {
            title: "Put your opening hours where Google can read them".into(),
            lines: (1..=lines).map(|i| format!("Line {i}.")).collect(),
            link: None,
        }
    }

    #[derive(Default)]
    struct Recording(Mutex<Vec<Envelope>>);

    #[async_trait]
    impl Transport for Recording {
        fn readiness(&self, _channel: Channel) -> Readiness {
            Readiness::Ready
        }
        async fn deliver(&self, envelope: &Envelope) -> Delivery {
            self.0.lock().unwrap().push(envelope.clone());
            Delivery::Sent {
                provider: "test".into(),
                provider_ref: "r1".into(),
            }
        }
    }

    #[test]
    fn the_weekly_fix_is_five_lines_or_fewer() {
        let c = consent();
        assert!(
            newsletter_issue(
                "a@gmail.com",
                &issue(5),
                "2026-46",
                "https://agentrix.website/fix/stop?token=t",
                &c
            )
            .is_ok()
        );
        assert_eq!(
            newsletter_issue(
                "a@gmail.com",
                &issue(6),
                "2026-46",
                "https://agentrix.website/fix/stop?token=t",
                &c
            )
            .unwrap_err(),
            IssueError::TooLong(6)
        );
        assert_eq!(
            newsletter_issue(
                "a@gmail.com",
                &issue(0),
                "2026-46",
                "https://agentrix.website/fix/stop?token=t",
                &c
            )
            .unwrap_err(),
            IssueError::Empty
        );
    }

    #[tokio::test]
    async fn sends_keep_the_consent_words_and_carry_one_click_unsubscribe() {
        let transport = Arc::new(Recording::default());
        let messages = Messages::new(
            Outbox::new(crate::outbox::tests::pool().await),
            Arc::new(Fixed::default()),
        )
        .with_transport(transport.clone());
        let out = newsletter_issue(
            "reader@gmail.com",
            &issue(3),
            "2026-46",
            "https://agentrix.website/fix/stop?token=t",
            &consent(),
        )
        .unwrap();
        let sent = messages.send(out).await.unwrap();
        let row = &sent.entries[0];
        assert!(
            row.consent
                .as_deref()
                .unwrap()
                .contains("Send me the weekly fix")
        );
        let env = &transport.0.lock().unwrap()[0];
        assert!(
            env.headers
                .iter()
                .any(|(k, v)| k == "List-Unsubscribe" && v.contains("/fix/stop"))
        );
        assert!(
            env.headers
                .iter()
                .any(|(k, v)| k == "List-Unsubscribe-Post" && v == "List-Unsubscribe=One-Click")
        );
        assert!(
            env.text
                .contains("with the words \"Send me the weekly fix\""),
            "the footer says what they agreed to"
        );
        assert_eq!(env.from_name, "Agentrix");
    }

    #[tokio::test]
    async fn no_consent_words_no_message() {
        let messages = Messages::new(
            Outbox::new(crate::outbox::tests::pool().await),
            Arc::new(Fixed::default()),
        );
        let mut out = newsletter_confirm(
            "reader@gmail.com",
            "https://agentrix.website/fix/confirm?token=t",
            "Tuesday",
            &consent(),
        );
        out.context.as_object_mut().unwrap().remove("consent");
        assert!(matches!(messages.send(out).await, Err(SendError::Rule(_))));
    }

    #[test]
    fn whatsapp_confirm_asks_for_yes_and_the_issue_says_stop() {
        let confirm = newsletter_confirm(
            "whatsapp:+254700000001",
            "https://agentrix.website/fix/confirm?token=t",
            "Tuesday",
            &consent(),
        );
        assert_eq!(confirm.to.phone.as_deref(), Some("+254700000001"));
        let chat = render::render(&confirm.message, &confirm.context)
            .unwrap()
            .chat
            .unwrap();
        assert_eq!(chat.replies, vec!["YES"]);
        let out = newsletter_issue(
            "whatsapp:+254700000001",
            &issue(2),
            "2026-46",
            "https://agentrix.website/fix/stop?token=t",
            &consent(),
        )
        .unwrap();
        let chat = render::render(&out.message, &out.context)
            .unwrap()
            .chat
            .unwrap();
        assert!(chat.text.ends_with("Reply STOP to stop these."));
    }

    #[test]
    fn the_report_renders_with_its_one_next_step() {
        let report = Report {
            site: "kilele-bakery.example".into(),
            checked_at: "Sun 8 Nov, 10:12".into(),
            headline: "We read 41 pages. Three things to fix first.".into(),
            findings: vec![
                "One.".into(),
                "Two.".into(),
                "Three.".into(),
                "Four.".into(),
            ],
            report_url: "https://agentrix.website/assess/r/a_7k2m9q".into(),
            keep_url: "https://agentrix.website/start?from=a_7k2m9q".into(),
            expires: "Tue 8 Dec".into(),
        };
        let out = lead_report("wanjiru@gmail.com", Some("Wanjiru"), &report, &consent());
        let email = render::render(&out.message, &out.context)
            .unwrap()
            .email
            .unwrap();
        assert_eq!(
            email.subject,
            "Your website check for kilele-bakery.example"
        );
        assert!(
            email
                .text
                .contains("Keep my new site: https://agentrix.website/start?from=a_7k2m9q")
        );
        assert!(!email.text.contains("Four."), "three findings at most");
    }
}
