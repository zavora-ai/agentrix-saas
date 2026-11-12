//! W6's [`CodeMailer`] port, implemented with the catalogue and the outbox.
//!
//! Node wires it into customer accounts (`Visitors::…with_mailer`) and owner sign-in
//! (`agentrix_identity::mount_with_mailer`) with [`crate::identity_mailer`]. Every code and
//! notice is then written in the catalogue's words, recorded in the outbox (codes replaced by
//! dots) and delivered when an email provider is set up. Until then [`CodeMailer::delivers`]
//! says false, so W6 shows codes on sample sites for reserved `example` addresses, as it
//! designed.
//!
//! Mail for owners comes with the site id `platform`: it goes to the owner messages, and the
//! account is found by email so the owner sees it on their Messages page and, for sign-ins with
//! a code, on every other channel they have.

use async_trait::async_trait;
use serde_json::{Value as Json, json};

use agentrix_identity::mailer::{CodeMailer, CodePurpose, Mail, MailKind};

use crate::catalogue::Channel;
use crate::outbox::Status;
use crate::send::{Messages, Outgoing, Readiness, Recipient};

/// The site id identity uses for the platform host (owner mail).
const PLATFORM: &str = "platform";

/// Sends W6's sign-in emails through the messages service.
#[derive(Clone)]
pub struct IdentityMailer {
    messages: Messages,
}

impl IdentityMailer {
    /// Over `messages`.
    pub fn new(messages: Messages) -> Self {
        Self { messages }
    }
}

fn mask(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first: String = local.chars().take(1).collect();
            format!("{first}•••@{domain}")
        }
        None => "your email".to_string(),
    }
}

/// The catalogue message, its context and the outbox reason for one of W6's mails to a
/// customer of a site.
pub(crate) fn customer(mail: &Mail) -> Option<(&'static str, Json, String)> {
    let site = &mail.site;
    let origin = site.origin.trim_end_matches('/');
    let site_json = json!({
        "name": site.name,
        "host": site.host,
        "sample": site.sample,
        "url": origin,
    });
    let account_url = format!("{origin}/account/security");
    Some(match &mail.kind {
        MailKind::Code {
            code,
            purpose,
            minutes,
        } => {
            let (message, why) = match purpose {
                CodePurpose::SignIn => ("customer.signin_code", "Someone asked for a sign-in code"),
                CodePurpose::Create => {
                    ("customer.create_code", "Someone started making an account")
                }
                CodePurpose::Verify => (
                    "customer.verify_email",
                    "A customer confirmed their email before ordering",
                ),
                CodePurpose::Recover => (
                    "customer.recovery_code",
                    "Someone started getting back into an account",
                ),
            };
            (
                message,
                json!({ "site": site_json, "code": code, "minutes": minutes }),
                format!("{why} at {}", site.host),
            )
        }
        MailKind::NewSignIn { device, when } | MailKind::CodeSignIn { device, when, .. } => (
            "customer.new_signin",
            json!({
                "site": site_json,
                "signin": { "device": device, "place": null, "when": when },
                "account_url": account_url,
            }),
            format!(
                "A device the customer hasn't used before signed in at {}",
                site.host
            ),
        ),
        MailKind::RecoveryCodeUsed { device, remaining } => (
            "customer.recovery_code_used",
            json!({
                "site": site_json,
                "recovery": { "device": device, "remaining": remaining },
                "account_url": account_url,
            }),
            format!("Someone signed in with a recovery code at {}", site.host),
        ),
        // Recovery holds are for owners only; a customer recovers with a code at once.
        MailKind::RecoveryHold { .. } => return None,
        // Team invitations go out from the platform, never from a customer's site.
        MailKind::TeamInvite { .. } => return None,
    })
}

/// What each of W6's roles can and can't do, in the words the invitation uses. The facts come
/// from `agentrix_identity::team::Role`: managers decide cards, billing pays invoices, and only
/// the owner changes who helps run the site.
fn role_facts(role: &str) -> (&str, &'static str, &'static str) {
    let (name, _) = role.split_once(", who ").unwrap_or((role, ""));
    let (can, cannot) = match name {
        "a manager" => ("Approve decisions", "Pay invoices or add people"),
        "a viewer" => ("See everything on the site", "Change anything"),
        "billing" => (
            "See the site and pay invoices",
            "Approve decisions or add people",
        ),
        _ => ("Help run the site", "Add people"),
    };
    (name, can, cannot)
}

/// The same for mail to an owner (site id `platform`). Sign-in mail is about the account, not
/// one site, so `console_url` is `/os` (agentrix-shell sends the owner on to their site's
/// console) and nothing is added to it.
pub(crate) fn owner(mail: &Mail) -> (&'static str, Json, String) {
    let origin = mail.site.origin.trim_end_matches('/');
    let console_url = format!("{origin}/os");
    match &mail.kind {
        MailKind::Code {
            code,
            purpose,
            minutes,
        } => {
            let (message, why) = match purpose {
                CodePurpose::SignIn => (
                    "owner.signin_code",
                    "An owner asked for an email sign-in code",
                ),
                CodePurpose::Recover => {
                    ("owner.recovery_code", "An owner started recovery by email")
                }
                CodePurpose::Create | CodePurpose::Verify => {
                    ("owner.verify_email", "An owner confirmed their email")
                }
            };
            (
                message,
                json!({ "code": code, "minutes": minutes }),
                why.to_string(),
            )
        }
        MailKind::CodeSignIn {
            device,
            when,
            method,
        } => (
            "owner.code_signin",
            json!({
                "signin": { "device": device, "when": when, "method": method },
                "console_url": console_url,
            }),
            format!("An owner signed in with {method}; every channel hears about it"),
        ),
        MailKind::NewSignIn { device, when } => (
            "owner.new_device",
            json!({
                "signin": { "device": device, "place": null, "when": when, "method": "a passkey or a password and code" },
                "console_url": console_url,
            }),
            "A device the owner hasn't used before signed in".to_string(),
        ),
        MailKind::RecoveryHold { release } => (
            "owner.recovery_started",
            json!({
                "recovery": {
                    "email": mask(&mail.to),
                    "opens_at": release,
                    // Signing in doesn't stop the hold by itself: agentrix-onboarding's
                    // /account/recover lists a signed-in owner's holds with "Stop the recovery".
                    "stop_url": format!("{origin}/account/signin?next=%2Faccount%2Frecover"),
                },
            }),
            "Account recovery by email started; the 24-hour hold began".to_string(),
        ),
        MailKind::TeamInvite {
            business,
            inviter,
            role,
            // W6's link is `/team/join?t=…`, and no page serves it yet: the invitation is
            // accepted only through `POST /team/api/invites/accept`. Until a join page exists,
            // the email links where the invitee can make their account (agentrix-onboarding's
            // `/account/new`) and says plainly that accepting online isn't ready.
            link: _,
        } => {
            let (role, can, cannot) = role_facts(role);
            // W6 keeps an invitation open for 7 days (agentrix-identity's invite mail says so).
            let expires = (chrono::Utc::now() + chrono::Duration::days(7))
                .format("%a %-d %b")
                .to_string();
            (
                "teammate.invite",
                json!({
                    "site": { "name": business, "host": mail.site.host, "sample": mail.site.sample },
                    "invite": {
                        "by": inviter,
                        "role": role,
                        "can": can,
                        "cannot": cannot,
                        "expires": expires,
                        "url": format!("{origin}/account/new"),
                    },
                }),
                format!("An owner invited someone to help run {business}"),
            )
        }
        MailKind::RecoveryCodeUsed { device, remaining } => (
            "owner.signin_method_changed",
            json!({
                "change": { "kind": "recovery_code_used", "device": device, "when": "Just now", "remaining": remaining },
                "console_url": console_url,
            }),
            "An owner signed in with a recovery code".to_string(),
        ),
    }
}

#[async_trait]
impl CodeMailer for IdentityMailer {
    async fn send(&self, mail: Mail) -> Result<(), String> {
        let is_owner = mail.site.site.to_string() == PLATFORM;
        let result = if is_owner {
            let (message, context, reason) = owner(&mail);
            let contact = self
                .messages
                .directory()
                .owner_by_email(&mail.to)
                .await
                .ok()
                .flatten();
            let to = Recipient {
                name: contact
                    .as_ref()
                    .and_then(|c| c.name.split_whitespace().next().map(str::to_string)),
                email: Some(mail.to.clone()),
                account_id: contact.map(|c| c.account),
                ..Recipient::default()
            };
            self.messages
                .send(Outgoing {
                    message: message.to_string(),
                    site_id: None,
                    to,
                    context,
                    reason,
                    dedupe_key: None,
                })
                .await
        } else {
            let Some((message, context, reason)) = customer(&mail) else {
                tracing::warn!("identity sent a customer a mail kind customers never get; skipped");
                return Ok(());
            };
            self.messages
                .send(Outgoing {
                    message: message.to_string(),
                    site_id: Some(mail.site.site.to_string()),
                    to: Recipient::email(mail.to.clone(), None),
                    context,
                    reason,
                    dedupe_key: None,
                })
                .await
        };
        let sent = result.map_err(|e| {
            tracing::error!(error = %e, "couldn't record a sign-in email");
            "We couldn't send the email. Try again in a minute.".to_string()
        })?;
        // The provider refusing it (a limit reached, an address it won't take): the person is
        // told, rather than waiting for a code that isn't coming.
        if let Some(e) = sent
            .entries
            .iter()
            .find(|e| e.channel == Channel::Email && e.status == Status::Failed)
        {
            tracing::warn!(outbox = %e.id, "the email provider refused a sign-in email");
            return Err("We couldn't send the email. Try again in a minute.".to_string());
        }
        Ok(())
    }

    fn delivers(&self) -> bool {
        self.messages.readiness(Channel::Email) == Readiness::Ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::{Contact, Fixed};
    use crate::outbox::{Outbox, Status};
    use agentrix_types::SiteId;
    use agentrix_types::tenancy::SiteHost;
    use std::sync::Arc;

    fn site(id: &str, name: &str) -> SiteHost {
        SiteHost {
            site: SiteId::new(id),
            host: "kilele-bakery-sample.agentrix.website".into(),
            origin: "https://kilele-bakery-sample.agentrix.website".into(),
            name: name.into(),
            sample: id != "platform",
        }
    }

    fn kinds() -> Vec<MailKind> {
        vec![
            MailKind::Code {
                code: "482913".into(),
                purpose: CodePurpose::SignIn,
                minutes: 10,
            },
            MailKind::Code {
                code: "482913".into(),
                purpose: CodePurpose::Create,
                minutes: 10,
            },
            MailKind::Code {
                code: "482913".into(),
                purpose: CodePurpose::Verify,
                minutes: 10,
            },
            MailKind::Code {
                code: "482913".into(),
                purpose: CodePurpose::Recover,
                minutes: 15,
            },
            MailKind::NewSignIn {
                device: "Chrome on a Windows PC".into(),
                when: "Wed 11 Nov 2026, 21:14 Nairobi time".into(),
            },
            MailKind::RecoveryCodeUsed {
                device: "Safari on an iPhone".into(),
                remaining: 1,
            },
            MailKind::CodeSignIn {
                device: "Chrome on a Mac".into(),
                when: "Sun 8 Nov, 07:40".into(),
                method: "an email code".into(),
            },
        ]
    }

    struct Refuses;

    #[async_trait]
    impl crate::send::Transport for Refuses {
        fn readiness(&self, _channel: Channel) -> Readiness {
            Readiness::Ready
        }
        async fn deliver(&self, _envelope: &crate::send::Envelope) -> crate::send::Delivery {
            crate::send::Delivery::Failed {
                reason: "refused".into(),
            }
        }
    }

    #[tokio::test]
    async fn a_code_the_provider_refuses_is_reported_not_announced() {
        let messages = Messages::new(
            Outbox::new(crate::outbox::tests::pool().await),
            Arc::new(Fixed::default()),
        )
        .with_transport(Arc::new(Refuses));
        let mailer = IdentityMailer::new(messages);
        let refused = mailer
            .send(Mail {
                site: site("kilele-bakery-sample", "Kilele Bakery"),
                to: "amina@gmail.com".into(),
                kind: kinds().remove(0),
            })
            .await;
        assert_eq!(
            refused,
            Err("We couldn't send the email. Try again in a minute.".to_string())
        );
    }

    #[tokio::test]
    async fn every_kind_of_customer_mail_lands_in_the_outbox() {
        let messages = Messages::new(
            Outbox::new(crate::outbox::tests::pool().await),
            Arc::new(Fixed::default()),
        );
        let mailer = IdentityMailer::new(messages.clone());
        assert!(!mailer.delivers());
        for kind in kinds() {
            mailer
                .send(Mail {
                    site: site("site_k", "Kilele Bakery"),
                    to: "njeri@gmail.com".into(),
                    kind,
                })
                .await
                .unwrap();
        }
        let rows = messages
            .outbox()
            .list_for_owner("nobody", &["site_k".into()], &Default::default())
            .await
            .unwrap();
        assert_eq!(rows.len(), 7);
        for row in rows {
            assert_eq!(row.status, Status::NotSent);
            assert!(row.message.starts_with("customer."));
            assert!(
                !row.body_text.contains("482"),
                "{}: code stored",
                row.message
            );
        }
    }

    #[tokio::test]
    async fn owner_mail_goes_to_owner_messages_on_their_page() {
        let directory = Arc::new(Fixed {
            sites: vec![],
            owners: vec![Contact {
                account: "acct_w".into(),
                email: "Wanjiru.Kamau@gmail.com".into(),
                name: "Wanjiru Kamau".into(),
            }],
        });
        let messages = Messages::new(Outbox::new(crate::outbox::tests::pool().await), directory);
        let mailer = IdentityMailer::new(messages.clone());
        let mut all = kinds();
        all.push(MailKind::RecoveryHold {
            release: "Mon 9 Nov, 07:41".into(),
        });
        for kind in all {
            mailer
                .send(Mail {
                    site: site("platform", "Agentrix"),
                    to: "wanjiru.kamau@gmail.com".into(),
                    kind,
                })
                .await
                .unwrap();
        }
        let rows = messages
            .outbox()
            .list_for_owner("acct_w", &[], &Default::default())
            .await
            .unwrap();
        assert_eq!(rows.len(), 8, "every owner mail shows on the owner's page");
        assert!(rows.iter().all(|r| r.message.starts_with("owner.")));
        assert!(rows.iter().all(|r| !r.body_text.contains("482")));
        let hold = rows
            .iter()
            .find(|r| r.message == "owner.recovery_started")
            .unwrap();
        assert!(hold.body_text.contains("w•••@gmail.com"));
        assert!(hold.body_text.contains("Mon 9 Nov, 07:41"));
    }
}
