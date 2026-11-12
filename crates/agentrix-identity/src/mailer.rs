//! The emails sign-in sends, as a port. W9 (messages) implements [`CodeMailer`] with its email
//! driver behind the Effects Gateway; node wires it. Each [`Mail`] carries structured facts
//! plus default words written to voice.md, so a driver can send it as it is or render its own
//! template from [`MailKind`].
//!
//! Emails never approve anything and never carry a link that signs someone in (06 §4.10).

use std::sync::Mutex;

use agentrix_types::tenancy::SiteHost;
use async_trait::async_trait;

/// Why a code was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodePurpose {
    /// Signing in to an existing account.
    SignIn,
    /// Creating an account (proves the email).
    Create,
    /// Proving the email of a signed-in account before ordering (A-15).
    Verify,
    /// Getting back in after losing a passkey device (A-16).
    Recover,
}

/// What an email is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailKind {
    /// A 6-digit code, good once, for `minutes`.
    Code {
        /// The code.
        code: String,
        /// Why it was sent.
        purpose: CodePurpose,
        /// How long it works.
        minutes: u32,
    },
    /// A device this account hasn't used before signed in.
    NewSignIn {
        /// "Chrome on a Windows PC".
        device: String,
        /// When, "Wed 11 Nov 2026, 21:14 Nairobi time".
        when: String,
    },
    /// An owner signed in with an email code or a recovery code (W0's condition: every such
    /// sign-in is announced on the owner's other channels; W9 routes this kind there).
    CodeSignIn {
        /// "Chrome on a Mac".
        device: String,
        /// When, in words.
        when: String,
        /// "an email code" or "a recovery code".
        method: String,
    },
    /// Someone asked to recover an owner account by email; nothing happens until `release`.
    RecoveryHold {
        /// When the hold ends, in words.
        release: String,
    },
    /// An owner invited someone to help run a site.
    TeamInvite {
        /// The business, for example "Kilele Bakery".
        business: String,
        /// Who invited them.
        inviter: String,
        /// The role in words, for example "a manager, who can approve decisions".
        role: String,
        /// The page that accepts it (after signing in with this email).
        link: String,
    },
    /// A recovery code was used to sign in.
    RecoveryCodeUsed {
        /// The device that used it.
        device: String,
        /// Codes left.
        remaining: usize,
    },
}

/// One email to a customer of a site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    /// The site it is from.
    pub site: SiteHost,
    /// The address.
    pub to: String,
    /// What it says.
    pub kind: MailKind,
}

impl Mail {
    /// The default subject line.
    pub fn subject(&self) -> String {
        let site = &self.site.name;
        match &self.kind {
            MailKind::Code {
                code,
                purpose: CodePurpose::Create,
                ..
            } => format!("{code} is your code to create your {site} account"),
            MailKind::Code { code, .. } => format!("{code} is your {site} sign-in code"),
            MailKind::NewSignIn { .. } => format!("New sign-in to your {site} account"),
            MailKind::RecoveryCodeUsed { .. } => {
                format!("A recovery code was used on your {site} account")
            }
            MailKind::CodeSignIn { .. } => format!("New sign-in to your {site} account"),
            MailKind::TeamInvite {
                business, inviter, ..
            } => {
                format!("{inviter} invited you to help run {business}")
            }
            MailKind::RecoveryHold { .. } => {
                format!("Someone asked to get back into your {site} account")
            }
        }
    }

    /// The default plain-text body.
    pub fn text(&self) -> String {
        let site = &self.site.name;
        let host = &self.site.host;
        let sample = if self.site.sample {
            format!(
                "\n\n{site} is a sample site on Agentrix. Nothing here is a real order or payment."
            )
        } else {
            String::new()
        };
        let never = format!(
            "{site} never asks for a password, a code or a card by email, WhatsApp or chat."
        );
        match &self.kind {
            MailKind::Code { code, minutes, .. } => format!(
                "Your code is {code}\n\nType it on {host}. It works once, for {minutes} minutes.\n\nDidn't ask for it? You can ignore this email; nobody can sign in without the code.\n\n{never}{sample}"
            ),
            MailKind::NewSignIn { device, when } => format!(
                "New sign-in on {device}. Not you?\n\n{when}. If it was you, there's nothing to do.\n\nIf it wasn't, open your account and choose Sign out everywhere:\n{host}/account/security\n\n{never}{sample}"
            ),
            MailKind::RecoveryCodeUsed { device, remaining } => format!(
                "Someone signed in to your {site} account with a recovery code, on {device}. You have {remaining} codes left.\n\nIf it wasn't you, open your account and choose Sign out everywhere:\n{host}/account/security\n\n{never}{sample}"
            ),
            MailKind::CodeSignIn {
                device,
                when,
                method,
            } => format!(
                "Someone signed in to your {site} account with {method}, on {device}, {when}.\n\nIf it was you, there's nothing to do. If it wasn't, sign in with your passkey and end the other sessions:\n{host}/console\n\nA code can't approve anything: approvals always need your passkey or your authenticator app.\n\n{never}"
            ),
            MailKind::TeamInvite {
                business,
                inviter,
                role,
                link,
            } => format!(
                "{inviter} invited you to help run {business} on {site}, as {role}.\n\nTo accept, open this page and sign in or create an account with this email address:\n{link}\n\nThe invitation works for 7 days, and only for this email address. Opening the link doesn't sign you in.\n\n{never}"
            ),
            MailKind::RecoveryHold { release } => format!(
                "Someone asked to get back into your {site} account with a code sent to this email.\n\nNothing changes until {release}. If it was you, come back then and sign in with a new code.\n\nIf it wasn't you, sign in with your passkey or your authenticator app before then and cancel it:\n{host}/console\n\n{never}"
            ),
        }
    }
}

/// Sends sign-in emails.
#[async_trait]
pub trait CodeMailer: Send + Sync {
    /// Sends `mail`. An error is logged and shown to the customer as "We couldn't send the
    /// email"; it is never retried silently.
    async fn send(&self, mail: Mail) -> Result<(), String>;

    /// Whether this mailer reaches real inboxes. When it doesn't (development, or before W9's
    /// driver is wired), sample sites show codes for reserved `example` addresses on the page.
    fn delivers(&self) -> bool {
        true
    }
}

/// Logs that an email would go out, without the code or the address. For development before
/// W9's driver is wired. Never use it where customers expect a real email.
#[derive(Debug, Default)]
pub struct LogMailer;

#[async_trait]
impl CodeMailer for LogMailer {
    async fn send(&self, mail: Mail) -> Result<(), String> {
        let domain = mail.to.rsplit('@').next().unwrap_or("?").to_owned();
        tracing::info!(site = %mail.site.site, %domain, subject_kind = kind_name(&mail.kind), "identity: email not sent (log-only mailer)");
        Ok(())
    }

    fn delivers(&self) -> bool {
        false
    }
}

/// Keeps every email in memory. For tests and local previews.
#[derive(Debug, Default)]
pub struct MemoryMailer {
    sent: Mutex<Vec<Mail>>,
}

impl MemoryMailer {
    /// Everything sent so far.
    pub fn sent(&self) -> Vec<Mail> {
        self.sent.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The last code sent to `to`, if any.
    pub fn last_code(&self, to: &str) -> Option<String> {
        self.sent().into_iter().rev().find_map(|m| match m.kind {
            MailKind::Code { code, .. } if m.to.eq_ignore_ascii_case(to) => Some(code),
            _ => None,
        })
    }
}

#[async_trait]
impl CodeMailer for MemoryMailer {
    async fn send(&self, mail: Mail) -> Result<(), String> {
        self.sent
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(mail);
        Ok(())
    }

    fn delivers(&self) -> bool {
        false
    }
}

fn kind_name(kind: &MailKind) -> &'static str {
    match kind {
        MailKind::Code { .. } => "code",
        MailKind::NewSignIn { .. } => "new-sign-in",
        MailKind::RecoveryCodeUsed { .. } => "recovery-code-used",
        MailKind::CodeSignIn { .. } => "code-sign-in",
        MailKind::RecoveryHold { .. } => "recovery-hold",
        MailKind::TeamInvite { .. } => "team-invite",
    }
}

/// Whether `email` is one of Agentrix's test customer agents (`test-agent-<name>@example.com`):
/// on a sample site its code is shown on the page, labelled, and no email is sent.
pub fn is_test_agent(email: &str) -> bool {
    let Some((local, domain)) = email.rsplit_once('@') else {
        return false;
    };
    let local = local.to_ascii_lowercase();
    domain.eq_ignore_ascii_case("example.com")
        && local.starts_with("test-agent-")
        && local.len() > "test-agent-".len()
        && local.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Whether `email` is a reserved example address (RFC 2606, RFC 6761): `example.com`,
/// `example.org`, `example.net`, or a domain ending in `.example` or `.test`. Codes for these
/// show on a sample site's page, because no inbox exists for them.
pub fn is_reserved_example(email: &str) -> bool {
    let Some((_, domain)) = email.rsplit_once('@') else {
        return false;
    };
    let domain = domain.to_ascii_lowercase();
    matches!(
        domain.as_str(),
        "example.com" | "example.org" | "example.net"
    ) || domain.ends_with(".example")
        || domain.ends_with(".test")
        || domain == "example"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_test_agents_at_example_com_are_test_agents() {
        assert!(is_test_agent("test-agent-amani@example.com"));
        assert!(is_test_agent("Test-Agent-Chiku@EXAMPLE.com"));
        assert!(!is_test_agent("test-agent-@example.com"));
        assert!(!is_test_agent("test-agent-amani@example.org"));
        assert!(!is_test_agent("amani@example.com"));
        assert!(!is_test_agent("test-agent-amani+x@example.com"));
        assert!(!is_test_agent("test-agent-amani@gmail.com"));
    }
    use agentrix_types::SiteId;

    fn site() -> SiteHost {
        SiteHost {
            site: SiteId::new("site_k"),
            host: "kilele-bakery-sample.agentrix.website".into(),
            origin: "https://kilele-bakery-sample.agentrix.website".into(),
            name: "Kilele Bakery".into(),
            sample: true,
        }
    }

    #[test]
    fn words_say_what_and_never_link_a_sign_in() {
        let mail = Mail {
            site: site(),
            to: "njeri@example.com".into(),
            kind: MailKind::Code {
                code: "482913".into(),
                purpose: CodePurpose::SignIn,
                minutes: 10,
            },
        };
        assert_eq!(mail.subject(), "482913 is your Kilele Bakery sign-in code");
        let text = mail.text();
        assert!(text.contains("It works once, for 10 minutes."));
        assert!(text.contains("sample site"));
        assert!(!text.contains("http"), "no link signs anyone in");
        assert!(!text.contains('—') && !text.contains('!'));
    }

    #[test]
    fn reserved_addresses() {
        assert!(is_reserved_example("njeri@example.com"));
        assert!(is_reserved_example("a@kilele.test"));
        assert!(is_reserved_example("a@shop.example"));
        assert!(!is_reserved_example("njeri@gmail.com"));
        assert!(!is_reserved_example("a@example.com.evil.io"));
    }
}
