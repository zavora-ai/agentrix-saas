//! The Messages app inside W5's shell, on the shell's sample bakery, for checking it in a
//! browser: `scripts/wcargo run -p agentrix-messages --example console -- 18902`, then open
//! <http://localhost:18902/console/site_kilele_sample/apps/messages>.
//! Every message goes through the real send path with no provider, so rows show "Not sent", as
//! they will until a provider is chosen. Nothing here talks to a real node.

use std::sync::Arc;

use agentrix_host::testing::FixedOwner;
use agentrix_messages::directory::{Contact, Fixed, SiteRef};
use agentrix_messages::{Messages, Outbox, Outgoing, Recipient, catalogue};
use agentrix_shell::{Shell, ShellState, sample::SamplePort, store::ShellStore};

const OWNER: &str = "0190f5c2-0000-7000-8000-000000000001";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(18902);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    agentrix_messages::migrate(&pool).await?;
    let shell_store = ShellStore::open(pool.clone()).await?;
    let site = agentrix_shell::sample::SITE;
    let directory = Arc::new(Fixed {
        sites: vec![SiteRef {
            id: site.into(),
            name: "Kilele Bakery".into(),
            domain: "kilele-bakery-sample.agentrix.website".into(),
            owner_account: Some(OWNER.into()),
            utc_offset_minutes: 180,
        }],
        owners: vec![Contact {
            account: OWNER.into(),
            email: "wanjiru@example.com".into(),
            name: "Wanjiru Kamau".into(),
        }],
    });
    let messages = Messages::new(Outbox::new(pool), directory);
    for (id, to, reason) in [
        (
            "customer.signin_code",
            "amina@gmail.com",
            "Amina asked for a sign-in code at kilele-bakery-sample.agentrix.website",
        ),
        (
            "customer.order_received",
            "njeri@gmail.com",
            "Njeri paid for order KB-1042",
        ),
        (
            "customer.request_held",
            "njeri@example.com",
            "Njeri asked for a custom cake for Amina (KB-1043); it waits for your OK",
        ),
        (
            "customer.payment_receipt",
            "njeri@gmail.com",
            "PayPal confirmed the payment for KB-1042 (USD 19.77)",
        ),
    ] {
        let def = catalogue::get(id).unwrap();
        messages
            .send(Outgoing {
                message: id.into(),
                site_id: Some(site.into()),
                to: Recipient::email(
                    to,
                    Some(if to.starts_with("amina") {
                        "Amina"
                    } else {
                        "Njeri"
                    }),
                ),
                context: def.example_context(),
                reason: reason.into(),
                dedupe_key: None,
            })
            .await?;
    }
    for (id, reason) in [
        (
            "owner.question_passed",
            "The Front desk couldn't answer a visitor's question from the site",
        ),
        ("owner.morning_brief", "Quiet hours ended at 07:00"),
    ] {
        let def = catalogue::get(id).unwrap();
        messages
            .send_to_site_owner(site, id, def.example_context(), reason, None)
            .await?;
    }
    let app = Arc::new(agentrix_messages::app::MessagesApp::new(
        messages.clone(),
        shell_store.clone(),
    ));
    let shell = agentrix_shell::router(ShellState {
        shell: Arc::new(Shell::new()?),
        port: Arc::new(SamplePort::default()),
        store: shell_store,
        apps: vec![app],
        lanes: Vec::new(),
        freeze_enabled: false,
    });
    let account: uuid::Uuid = OWNER.parse()?;
    let outbox_routes = agentrix_messages::web::routes(agentrix_messages::web::WebState {
        messages,
        owners: Arc::new(FixedOwner::passkey(account)),
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("http://localhost:{port}/console/{site}/apps/messages");
    axum::serve(
        listener,
        shell.merge(outbox_routes).merge(agentrix_ui::assets()),
    )
    .await?;
    Ok(())
}
