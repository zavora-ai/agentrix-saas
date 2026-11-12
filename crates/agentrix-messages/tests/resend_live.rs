//! A real send through Resend, one email for each way the platform sends: W6's codes and team
//! invitations (through [`IdentityMailer`]), an order email and the Friday digest's confirm link.
//! The visitor emails on agentrix.website (the newsletter, the kept report) have their own check
//! in `agentrix-home/tests/resend_live.rs`.
//!
//! Ignored unless asked for, since it sends real email and counts against Resend's daily limit:
//!
//! ```sh
//! RESEND_API_KEY=… AGENTRIX_RESEND_CHECK_TO=delivered@resend.dev \
//!   scripts/wcargo test -p agentrix-messages --test resend_live -- --ignored --nocapture
//! ```
//!
//! `delivered@resend.dev` is Resend's test inbox: it accepts every message and delivers nowhere.
//! Each email goes to `delivered+<message>@resend.dev`, so Resend's log names it. The key is read
//! from the environment as node reads it, and never printed.

use std::sync::Arc;

use agentrix_identity::mailer::{CodeMailer, CodePurpose, Mail, MailKind};
use agentrix_messages::directory::Fixed;
use agentrix_messages::email::{EmailConfig, ResendTransport};
use agentrix_messages::identity::IdentityMailer;
use agentrix_messages::{Channel, Messages, Outbox, Outgoing, Recipient, Status};
use agentrix_types::SiteId;
use agentrix_types::tenancy::SiteHost;

/// `delivered+<label>@resend.dev` from `delivered@resend.dev`.
fn labelled(to: &str, label: &str) -> String {
    match to.split_once('@') {
        Some((local, domain)) => format!("{local}+{}@{domain}", label.replace(['.', '_'], "-")),
        None => to.to_owned(),
    }
}

fn site(id: &str, name: &str, sample: bool) -> SiteHost {
    let host = format!("{id}.agentrix.website");
    SiteHost {
        site: SiteId::new(id),
        origin: format!("https://{host}"),
        host,
        name: name.into(),
        sample,
    }
}

#[tokio::test]
#[ignore = "sends real email through Resend; needs RESEND_API_KEY and AGENTRIX_RESEND_CHECK_TO"]
async fn each_way_the_platform_sends_email_reaches_resend() {
    let (Some(cfg), Ok(to)) = (
        EmailConfig::from_env(),
        std::env::var("AGENTRIX_RESEND_CHECK_TO"),
    ) else {
        panic!("set RESEND_API_KEY and AGENTRIX_RESEND_CHECK_TO to run this check");
    };
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_messages::migrate(&ctx.pool).await.unwrap();
    let messages = Messages::new(Outbox::new(ctx.pool.clone()), Arc::new(Fixed::default()))
        .with_transport(Arc::new(ResendTransport::new(ctx.http.clone(), cfg)));

    // W6's codes, notices and invitations: what node wires as `identity_mailer`.
    let mailer = IdentityMailer::new(messages.clone());
    let kilele = site("kilele-bakery-sample", "Kilele Bakery", true);
    let platform = site("platform", "Agentrix", false);
    let mails = [
        (
            "customer.signin_code",
            kilele.clone(),
            MailKind::Code {
                code: "482913".into(),
                purpose: CodePurpose::SignIn,
                minutes: 10,
            },
        ),
        (
            "owner.signin_code",
            platform.clone(),
            MailKind::Code {
                code: "482913".into(),
                purpose: CodePurpose::SignIn,
                minutes: 10,
            },
        ),
        (
            "teammate.invite",
            platform,
            MailKind::TeamInvite {
                business: "Kilele Bakery".into(),
                inviter: "Wanjiru".into(),
                role: "a manager, who can approve decisions".into(),
                link: "https://agentrix.website/console/people/accept?invite=check".into(),
            },
        ),
    ];
    for (label, site, kind) in mails {
        let sent = mailer
            .send(Mail {
                site,
                to: labelled(&to, label),
                kind,
            })
            .await;
        assert_eq!(sent, Ok(()), "{label}");
    }

    // An order email to a customer and the Friday digest's confirm link, as the samples send
    // them through `agentrix_messages::orders`, with the catalogue's own example values.
    for id in ["customer.order_received", "digest.confirm"] {
        let def = agentrix_messages::catalogue::get(id).unwrap();
        messages
            .send(Outgoing {
                message: id.into(),
                site_id: Some("kilele-bakery-sample".into()),
                to: Recipient::email(labelled(&to, id), Some("Amina")),
                context: def.example_context(),
                reason: "The live email check".into(),
                dedupe_key: None,
            })
            .await
            .unwrap();
    }

    let rows = sqlx::query_as::<_, (String, String, Option<String>, Option<String>)>(
        "SELECT message, status, status_reason, provider_ref FROM message_outbox WHERE channel = ? ORDER BY created_at",
    )
    .bind(Channel::Email.as_str())
    .fetch_all(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 5);
    for (message, status, reason, provider_ref) in &rows {
        println!(
            "{message}: {status} {}",
            provider_ref.as_deref().or(reason.as_deref()).unwrap_or("")
        );
        assert_eq!(status, Status::Sent.as_str(), "{message}: {reason:?}");
    }
}
