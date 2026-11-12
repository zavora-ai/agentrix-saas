//! A real send through Resend from agentrix.website's own visitor paths, as node wires them
//! (W10's keep step and newsletter over W9's outbox): the newsletter sign-up's confirm link, a
//! kept report, and one week's fix. The platform's other emails have their check in
//! `agentrix-messages/tests/resend_live.rs`.
//!
//! Ignored unless asked for, since it sends real email and counts against Resend's daily limit:
//!
//! ```sh
//! RESEND_API_KEY=… AGENTRIX_RESEND_CHECK_TO=delivered@resend.dev \
//!   scripts/wcargo test -p agentrix-home --test resend_live -- --ignored --nocapture
//! ```
//!
//! `delivered@resend.dev` is Resend's test inbox: it accepts every message and delivers nowhere.
//! Each email goes to `delivered+<message>@resend.dev`, so Resend's log names it. The key is read
//! from the environment as node reads it, and never printed.

use std::sync::Arc;

use agentrix_home::keep::{Keep, KeepForm, OutboxSender, ReportSender};
use agentrix_messages::directory::Fixed;
use agentrix_messages::email::{EmailConfig, ResendTransport};
use agentrix_messages::{Messages, Outbox};

fn labelled(to: &str, label: &str) -> String {
    match to.split_once('@') {
        Some((local, domain)) => format!("{local}+{}@{domain}", label.replace(['.', '_'], "-")),
        None => to.to_owned(),
    }
}

#[tokio::test]
#[ignore = "sends real email through Resend; needs RESEND_API_KEY and AGENTRIX_RESEND_CHECK_TO"]
async fn agentrix_website_s_visitor_emails_reach_resend() {
    let (Some(cfg), Ok(to)) = (
        EmailConfig::from_env(),
        std::env::var("AGENTRIX_RESEND_CHECK_TO"),
    ) else {
        panic!("set RESEND_API_KEY and AGENTRIX_RESEND_CHECK_TO to run this check");
    };
    let ctx = agentrix_host::testing::context().await.unwrap();
    let pool = ctx.pool.clone();
    agentrix_messages::migrate(&pool).await.unwrap();
    agentrix_home::migrate(&pool).await.unwrap();
    let messages = Messages::new(Outbox::new(pool.clone()), Arc::new(Fixed::default()))
        .with_transport(Arc::new(ResendTransport::new(ctx.http.clone(), cfg)));
    let sender = Arc::new(OutboxSender(messages));
    let keep = Keep::new(Some(pool.clone()), Some(sender.clone()));
    let now = chrono::Utc::now();

    // Screen 7's sign-up: the confirm link, and the words the visitor reads.
    let joined = keep.join(&labelled(&to, "newsletter.confirm"), now).await;
    println!("newsletter sign-up: {} ({})", joined.status, joined.message);
    assert_eq!(joined.status, "sent", "{}", joined.message);
    assert!(joined.message.starts_with("Check "));

    // The keep step after a free check: the report (no check on record, so its plain words).
    let kept = keep
        .keep(
            &KeepForm {
                email: labelled(&to, "lead.report"),
                weekly: None,
                assessment: "asmt_resendcheck".into(),
            },
            "/assess/asmt_resendcheck",
            now,
        )
        .await;
    println!("kept report: {} ({})", kept.status, kept.message);
    assert_eq!(kept.status, "sent", "{}", kept.message);

    // One week's fix, as the Editor's approved issue goes to a confirmed reader.
    let issue = agentrix_messages::catalogue::get("newsletter.issue")
        .unwrap()
        .example_context();
    let sent = sender
        .send(
            "newsletter.issue",
            &labelled(&to, "newsletter.issue"),
            issue,
            "resend-check",
        )
        .await
        .unwrap();
    println!(
        "weekly fix issue: {}",
        if sent { "sent" } else { "not sent" }
    );
    assert!(sent);

    let rows = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT message, status, provider_ref FROM message_outbox WHERE channel = 'email' ORDER BY created_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for (message, status, provider_ref) in &rows {
        println!(
            "{message}: {status} {}",
            provider_ref.as_deref().unwrap_or("")
        );
    }
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|(_, status, _)| status == "sent"));
}
