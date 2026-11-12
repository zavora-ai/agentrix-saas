//! The live proof: an agent's token from the deployed site's issuer, traded with AWS, Google and
//! Microsoft Entra for each cloud's own credential naming the agent.
//!
//! Run it where the site's issuer key is (the node's `<data_dir>/federation`), after the cloud
//! objects exist (`RECREATE.md`, kept outside the repo). Every id comes from the environment and
//! none is printed: the output says what names the agent and masks account numbers.
//!
//! ```text
//! SAAS_FEDERATION_KEY_DIR=/var/lib/agentrix-saas/federation \
//! SAAS_PLATFORM_HOST=paypal.agentrix.website PROOF_SITE=kilele-bakery-sample PROOF_ROLE=cashier \
//! AWS_ROLE_ARN=… AWS_REGION=us-east-1 \
//! GOOGLE_PROJECT_NUMBER=… GOOGLE_POOL=… GOOGLE_PROVIDER=… \
//! ENTRA_TENANT_ID=… ENTRA_BLUEPRINT_APP_ID=… ENTRA_AGENT_IDENTITY_ID=… [ENTRA_RESOURCE=…] \
//! cargo run -p saas-federation --example live_proof
//! ```
//!
//! A cloud whose variables are missing is skipped and says so.

use std::path::PathBuf;

use base64::Engine;
use saas_core::identity::agent_did;
use saas_federation::{
    AwsWebIdentity, EntraAgentIdentity, GoogleWorkloadIdentity, SiteIssuer, site_host,
};
use serde_json::Value;

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

/// Digits runs of 6 or more (account, project numbers) and GUID-shaped ids, masked.
fn mask(s: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() >= 6 {
            out.push_str("******")
        } else {
            out.push_str(run)
        }
        run.clear();
    };
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            run.push(ch);
        } else {
            flush(&mut run, &mut out);
            out.push(ch);
        }
    }
    flush(&mut run, &mut out);
    out
}

fn claims_of(jwt: &str) -> Value {
    jwt.split('.')
        .nth(1)
        .and_then(|p| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(p)
                .ok()
        })
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let platform = env("SAAS_PLATFORM_HOST")
        .ok_or_else(|| anyhow::anyhow!("SAAS_PLATFORM_HOST is required"))?;
    let site = env("PROOF_SITE").unwrap_or_else(|| "kilele-bakery-sample".into());
    let role = env("PROOF_ROLE").unwrap_or_else(|| "cashier".into());
    let key_dir = PathBuf::from(
        env("SAAS_FEDERATION_KEY_DIR")
            .ok_or_else(|| anyhow::anyhow!("SAAS_FEDERATION_KEY_DIR is required"))?,
    );
    let host = site_host(&site, &platform);
    let key_file = key_dir.join(format!("oidc-{host}.pem"));
    if !key_file.is_file() {
        // A new key would not be the one the site publishes: never make one here.
        anyhow::bail!("no issuer key for {host} in {}", key_dir.display());
    }
    let issuer = SiteIssuer::for_host(&host, &key_dir)?;
    let did = agent_did(&host, &role);
    let impostor = agent_did(
        &host,
        if role == "payables" {
            "receivables"
        } else {
            "payables"
        },
    );
    println!("issuer {}  kid {}", issuer.issuer(), issuer.kid());
    println!("agent  {did}");
    let http = reqwest::Client::new();

    match (env("AWS_ROLE_ARN"), env("AWS_REGION")) {
        (Some(role_arn), Some(region)) => {
            let aws = AwsWebIdentity::new(role_arn, &region);
            match aws.exchange(&http, &issuer, &did, &role).await {
                Ok(c) => println!(
                    "aws sts: ACCEPTED; SubjectFromWebIdentityToken = {}; role {}; credentials until {}",
                    c.subject,
                    mask(&c.assumed_role_arn),
                    c.expiration.format("%H:%M UTC")
                ),
                Err(e) => println!("aws sts: REFUSED {}", mask(&e.to_string())),
            }
            match aws.exchange(&http, &issuer, &impostor, "impostor").await {
                Ok(_) => {
                    println!("aws sts: impostor ACCEPTED (the role's trust policy is too wide)")
                }
                Err(e) => println!(
                    "aws sts: impostor subject REFUSED ({})",
                    mask(&e.to_string()).chars().take(80).collect::<String>()
                ),
            }
        }
        _ => println!("aws: skipped (AWS_ROLE_ARN, AWS_REGION)"),
    }

    match (
        env("GOOGLE_PROJECT_NUMBER"),
        env("GOOGLE_POOL"),
        env("GOOGLE_PROVIDER"),
    ) {
        (Some(n), Some(pool), Some(provider)) => {
            let google = GoogleWorkloadIdentity::new(&n, &pool, &provider);
            match google.exchange(&http, &issuer, &did).await {
                Ok(t) => println!(
                    "google sts: ACCEPTED; federated access token until {}",
                    t.expires_at.format("%H:%M UTC")
                ),
                Err(e) => println!("google sts: REFUSED {}", mask(&e.to_string())),
            }
        }
        _ => println!("google: skipped (GOOGLE_PROJECT_NUMBER, GOOGLE_POOL, GOOGLE_PROVIDER)"),
    }

    match (
        env("ENTRA_TENANT_ID"),
        env("ENTRA_BLUEPRINT_APP_ID"),
        env("ENTRA_AGENT_IDENTITY_ID"),
    ) {
        (Some(tenant), Some(blueprint), Some(agent_identity)) => {
            let resource =
                env("ENTRA_RESOURCE").unwrap_or_else(|| "https://graph.microsoft.com".into());
            let entra = EntraAgentIdentity::new(tenant, blueprint, agent_identity.clone());
            match entra.exchange(&http, &issuer, &did, &resource).await {
                Ok(t) => {
                    let c = claims_of(t.access_token.expose());
                    let names_agent = c["sub"].as_str() == Some(agent_identity.as_str())
                        || c["appid"].as_str() == Some(agent_identity.as_str())
                        || c["oid"].as_str() == Some(agent_identity.as_str());
                    println!(
                        "entra: ACCEPTED; agent identity token until {}; names the agent identity: {names_agent}; xms_act_fct {}",
                        t.expires_at.format("%H:%M UTC"),
                        c["xms_act_fct"].as_str().unwrap_or("-")
                    );
                }
                Err(e) => println!("entra: REFUSED {}", mask(&e.to_string())),
            }
            match entra.exchange(&http, &issuer, &impostor, &resource).await {
                Ok(_) => {
                    println!("entra: impostor ACCEPTED (the federated credential is too wide)")
                }
                Err(e) => println!(
                    "entra: impostor subject REFUSED ({})",
                    mask(&e.to_string()).chars().take(80).collect::<String>()
                ),
            }
        }
        _ => println!(
            "entra: skipped (ENTRA_TENANT_ID, ENTRA_BLUEPRINT_APP_ID, ENTRA_AGENT_IDENTITY_ID)"
        ),
    }
    Ok(())
}
