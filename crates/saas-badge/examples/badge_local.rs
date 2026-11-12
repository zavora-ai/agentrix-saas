//! Badges on this machine: Agentrix (`agentrix.localhost:18182`, the operator) and Kilele
//! Bakery (`kilele.localhost:18182`, a sample site) served by `Host` on one listener, then
//! Kilele's Cashier verified from its DID alone, as `agentrix-badge verify` does.
//!
//! ```text
//! scripts/wcargo run -p saas-badge --example badge_local            # verify and exit
//! scripts/wcargo run -p saas-badge --example badge_local -- --serve # keep serving
//! curl http://kilele.localhost:18182/agents/cashier/did.json
//! scripts/wcargo run -p saas-badge --bin agentrix-badge -- verify \
//!     did:web:kilele.localhost%3A18182:agents:cashier --local --operator did:web:agentrix.localhost%3A18182
//! ```
//!
//! The keys are the ones the sandbox settlement doors already sign with
//! (`~/.config/agentrix-saas/settle/keys/`). Kilele's PayPal binding is built from PayPal's own
//! read-back of the wallet Kilele saved with Agentrix's sandbox app. With no phone at hand here,
//! the link's ceremony is answered by a key file shaped like a passkey assertion and labelled
//! `local-owner-key` in the record, never `passkey`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agentrix_effects::paypal::{Environment, PayPalClient, PayPalConfig};
use axum::extract::Request;
use axum::http::header::HOST;
use saas_badge::binding::{BindingConfig, BindingDesk, MemoryAudit};
use saas_badge::ceremony::{Ceremony, LOCAL_KIND, ed25519_evidence};
use saas_badge::did::agent_did;
use saas_badge::resolve::{DidKeyResolver, DidWebResolver, KeyDirectoryResolver, Resolvers};
use saas_badge::site::{self, AgentSpec, Operator, SiteIdentity, SiteState};
use saas_badge::verify::Verifier;
use saas_core::CompanyId;
use saas_core::identity::Sponsor;
use saas_settle::LoopbackDirectorySource;
use saas_settle::rails::VaultRail;
use serde_json::json;
use tower::ServiceExt;

const AGENTRIX: &str = "agentrix.localhost:18182";
const KILELE: &str = "kilele.localhost:18182";

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

fn read_env() -> HashMap<String, String> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.env");
    if !path.exists() {
        path = home().join("Developer/hackathons/agentrix-saas/.env");
    }
    let mut out = HashMap::new();
    for line in std::fs::read_to_string(path).unwrap_or_default().lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            if !v.starts_with('#') {
                out.insert(k.trim().to_owned(), v.to_owned());
            }
        }
    }
    out
}

async fn link_paypal(site: &Arc<SiteState>) -> anyhow::Result<()> {
    let env = read_env();
    let wallets = saas_settle::Store::new(
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(home().join(".config/agentrix-saas/settle/agentrix.db"))
                    .create_if_missing(true),
            )
            .await?,
    );
    saas_settle::migrate(wallets.pool()).await?;
    let saved = wallets
        .vault("http://kilele.localhost:18182")
        .await?
        .and_then(|row| Some((row.vault_id?, row.setup_token.unwrap_or_default())));
    let (Some((vault_id, setup_token)), Some(id), Some(secret)) = (
        saved,
        env.get("PAYPAL_CLIENT_ID"),
        env.get("PAYPAL_CLIENT_SECRET"),
    ) else {
        println!("No saved Kilele wallet or sandbox keys here: the badge shows no PayPal link.");
        return Ok(());
    };
    anyhow::ensure!(
        env.get("PAYPAL_ENV").map(String::as_str) == Some("sandbox"),
        "sandbox only"
    );
    let vault = VaultRail::new(Arc::new(PayPalClient::new(
        PayPalConfig {
            client_id: id.clone(),
            client_secret: secret.clone(),
            webhook_id: None,
            environment: Environment::Sandbox,
        },
        reqwest::Client::new(),
    )));
    let audit = Arc::new(MemoryAudit::default());
    let desk = BindingDesk::new(BindingConfig {
        site: site.clone(),
        vault,
        owner: "kilele-owner (local run)".into(),
        rp_id: "agentrix.localhost".into(),
        audit,
        pool: sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?,
        public_base: format!("http://{KILELE}/paypal-link"),
        ceremony_url: None,
        approvals: None,
        owner_account: None,
        agents: vec![agent_did(KILELE, "cashier")],
        accept_local: true,
    })
    .await?;
    let binding_id = desk.adopt(&vault_id, &setup_token).await?;
    let subject = desk
        .subject(&binding_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let fp: [u8; 32] = hex::decode(subject.fingerprint.trim_start_matches("sha256:"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("bad fingerprint"))?;
    // No phone here: a key file stands in, and the record says so.
    let stand_in = adk_auth::web_bot_auth::Ed25519SigningKey::from_seed([42; 32])?;
    let evidence = ed25519_evidence(
        &stand_in,
        &fp,
        "agentrix.localhost",
        "http://agentrix.localhost:18182",
        LOCAL_KIND,
    );
    let sealed = desk
        .seal(&binding_id, evidence)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!(
        "Kilele's PayPal binding {}: saved payment method …{} ({}), from PayPal's answer to {} ({}), audit seq {}",
        sealed.record["binding_id"].as_str().unwrap_or_default(),
        vault_id
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>(),
        sealed.record["environment"].as_str().unwrap_or_default(),
        format_args!("GET /v3/vault/payment-tokens/{vault_id}"),
        sealed.record["paypal"]["response_digest"]
            .as_str()
            .unwrap_or_default(),
        sealed.audit_seq,
    );
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,saas_badge=info".into()),
        )
        .init();
    let serve = std::env::args().any(|a| a == "--serve");
    let keys = home().join(".config/agentrix-saas/settle/keys");
    let agentrix_company = CompanyId::agentrix();
    let kilele_company = CompanyId::kilele();
    let agentrix = SiteIdentity::load(
        AGENTRIX,
        "Agentrix",
        agentrix_company.clone(),
        false,
        vec![AgentSpec::cashier(&agentrix_company, "receivables")],
        &keys.join("agentrix"),
    )?;
    let kilele = SiteIdentity::load(
        KILELE,
        "Kilele Bakery",
        kilele_company.clone(),
        true,
        vec![AgentSpec::cashier(&kilele_company, "payables")],
        &keys.join("kilele"),
    )?;
    let sponsor = |name: &str| Sponsor {
        owner: format!("{name} (local run)"),
        name: Some(name.into()),
        passkey: None,
        approved_at: None,
    };
    let agentrix_site = SiteState::new(agentrix.clone(), sponsor("Agentrix's founder"));
    let kilele_site = SiteState::new(kilele.clone(), sponsor("Kilele Bakery's owner"));
    let operator = Operator::new(agentrix);

    // The routes, by Host, on one listener.
    let routers = (
        site::router(agentrix_site.clone()),
        site::router(kilele_site.clone()),
    );
    let app = axum::Router::new().fallback(move |request: Request| {
        let routers = routers.clone();
        async move {
            let host = request
                .headers()
                .get(HOST)
                .and_then(|h| h.to_str().ok())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let router = if host.starts_with("kilele.") {
                routers.1
            } else {
                routers.0
            };
            router.oneshot(request).await.unwrap_or_else(|e| match e {})
        }
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 18182)).await?;
    tokio::spawn(async move { axum::serve(listener, app).await.ok() });

    link_paypal(&kilele_site).await?;
    kilele_site.set_card(json!({
        "name": "Kilele Bakery agent door",
        "description": "Kilele Bakery's agents settle invoices here. Requests must be signed with Web Bot Auth.",
        "url": format!("http://{KILELE}/a2a"),
        "version": env!("CARGO_PKG_VERSION"),
        "supportedInterfaces": [{"url": format!("http://{KILELE}/a2a"), "protocolBinding": "JSONRPC", "protocolVersion": "1.0"}],
        "capabilities": {}, "defaultInputModes": ["application/json"], "defaultOutputModes": ["application/json"], "skills": [],
    })).await?;
    operator.vouch(&kilele_site)?;
    operator.vouch(&agentrix_site)?;

    // Verify from the DID alone, over HTTP, as anyone would.
    let http = saas_settle::keys::loopback_client(&[
        "http://agentrix.localhost:18182",
        "http://kilele.localhost:18182",
    ])?;
    let web = DidWebResolver::new(http).allow_loopback();
    let resolvers = Resolvers::new()
        .with(Arc::new(web.clone()))
        .with(Arc::new(DidKeyResolver))
        .with(Arc::new(KeyDirectoryResolver::new(Arc::new(
            LoopbackDirectorySource::new(),
        ))));
    let did = agent_did(KILELE, "cashier");
    let verifier =
        Verifier::new(web, resolvers, saas_badge::did::site_did(AGENTRIX)).accept_local();
    let report = verifier.verify(&did).await;
    println!("{did}");
    for check in &report.checks {
        println!(
            "  {} {:<46} {}",
            if check.ok { "✓" } else { "✗" },
            check.check,
            check.detail
        );
    }
    println!(
        "{}",
        if report.verified {
            "verified"
        } else {
            "NOT verified"
        }
    );
    if serve {
        println!(
            "serving on 127.0.0.1:18182 (agentrix.localhost, kilele.localhost); Ctrl-C to stop"
        );
        tokio::signal::ctrl_c().await?;
    }
    Ok(())
}
