//! `badge verify <did> [--local] [--operator <did:web:host | host>]`: resolves an agent's DID
//! from its site's own domain and checks its badge, every signature and link, printing each
//! check. Returns 0 when every check holds, 1 when one doesn't, 2 on a usage error.
//!
//! The node runs it as `agentrix-saas badge verify …`, passing its `platform_host` as the
//! operator; `agentrix-badge verify …` runs it standalone.
//!
//! - `--local` also resolves loopback hosts (`*.localhost`) over plain HTTP, and accepts a
//!   local run's stand-in approval (`local-owner-key`, reported as such), for local runs only.
//! - `--operator` names the voucher's issuer, as a DID or a host
//!   (`did:web:paypal.agentrix.website`, `paypal.agentrix.website`). Without it: the caller's
//!   default, then `AGENTRIX_OPERATOR`, then `AGENTRIX_PLATFORM_HOST`. Nothing is built in:
//!   every DID carries the platform host, which is configured.

use std::sync::Arc;

use adk_auth::web_bot_auth::{HttpKeyDirectorySource, KeyDirectorySource};
use saas_settle::LoopbackDirectorySource;

use crate::resolve::{DidKeyResolver, DidWebResolver, KeyDirectoryResolver, Resolvers};
use crate::verify::Verifier;

const USAGE: &str = "usage: badge verify <did:web:…> [--local] [--operator <did:web:host | host>]";

/// Runs `args` (starting with `verify`) and returns the exit code. `default_operator` is the
/// platform host (or its DID) when the caller knows it.
pub async fn run(args: &[String], default_operator: Option<&str>) -> i32 {
    let mut did = None;
    let mut local = false;
    let mut operator = default_operator
        .map(str::to_owned)
        .or_else(|| std::env::var("AGENTRIX_OPERATOR").ok())
        .or_else(|| std::env::var("AGENTRIX_PLATFORM_HOST").ok());
    let mut it = args.iter();
    if it.next().map(String::as_str) != Some("verify") {
        eprintln!("{USAGE}");
        return 2;
    }
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--local" => local = true,
            "--operator" => match it.next() {
                Some(o) => operator = Some(o.clone()),
                None => {
                    eprintln!("{USAGE}");
                    return 2;
                }
            },
            other if other.starts_with("did:") => did = Some(other.to_owned()),
            _ => {
                eprintln!("{USAGE}");
                return 2;
            }
        }
    }
    let Some(did) = did else {
        eprintln!("{USAGE}");
        return 2;
    };
    let Some(operator) = operator.filter(|o| !o.trim().is_empty()) else {
        eprintln!(
            "name the operator that vouches for sites: --operator <did:web:host | host>, AGENTRIX_OPERATOR or AGENTRIX_PLATFORM_HOST"
        );
        return 2;
    };
    let operator = operator_did(operator.trim());
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default();
    let (web, directories): (DidWebResolver, Arc<dyn KeyDirectorySource>) = if local {
        (
            DidWebResolver::new(http.clone()).allow_loopback(),
            Arc::new(LoopbackDirectorySource::new()),
        )
    } else {
        match HttpKeyDirectorySource::new() {
            Ok(source) => (DidWebResolver::new(http.clone()), Arc::new(source)),
            Err(e) => {
                eprintln!("can't build the key directory client: {e}");
                return 2;
            }
        }
    };
    let resolvers = Resolvers::new()
        .with(Arc::new(web.clone()))
        .with(Arc::new(DidKeyResolver))
        .with(Arc::new(KeyDirectoryResolver::new(directories)));
    let mut verifier = Verifier::new(web, resolvers, operator);
    if local {
        verifier = verifier.accept_local();
    }
    let report = verifier.verify(&did).await;
    println!("{}", report.did);
    for check in &report.checks {
        println!(
            "  {} {:<18} {}",
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
    if report.verified { 0 } else { 1 }
}

/// A DID as given, or a host (`paypal.agentrix.website`, `agentrix.localhost:18182`) as its
/// `did:web`.
pub fn operator_did(operator: &str) -> String {
    if operator.starts_with("did:") {
        operator.to_owned()
    } else {
        crate::did::site_did(operator)
    }
}
