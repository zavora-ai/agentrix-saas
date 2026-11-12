//! The face inside Agentrix OS (one binary): the passkey moments, the badges and the wallet as
//! three console apps (contract C9), and the scripts and styles they load.
//!
//! The node registers [`console_apps`] in `agentrix_shell::Plugins::apps` and mounts
//! [`assets`] on the platform host. The shell signs the owner in, checks they own the site, and
//! calls each app for `GET|POST /console/{site}/apps/{passkey,badges,wallet}/…`; the app answers
//! from the same [`console_router`] this crate serves on its own hosts, rendering only each
//! page's field so the shell draws the rest. Each app's id is the router's first path segment,
//! so the pages' own links (`{base}/badges`, `{base}/passkey/…`) land on the right app.
//!
//! On a sample site's host the node puts [`crate::apps::public_router`] (`/agents`, the badges
//! and Verify for anyone) in front of the samples' router.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agentrix_shell::apps::{AppContext, AppPage, AppReply, AppSpec, ConsoleApp};
use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use serde_json::Value;
use tower::ServiceExt;

use crate::Access;
use crate::apps::{SiteApps, console_router, saas_assets};
use crate::pages::FragmentTitle;

/// The scripts and styles the apps load: `/static/saas/…`, and the AG Grid wallet component at
/// `/static/saas/wallet-grid.js` from `grid_js` (`apps::default_grid_js()` in a checkout).
pub fn assets(grid_js: Option<PathBuf>) -> Router {
    saas_assets(grid_js)
}

/// The three apps, for `agentrix_shell::Plugins::apps`. `sites` maps the shell's site id to
/// that site's apps (its moments, badges and wallet seams, names and passkeys); a site not in
/// it doesn't show them. `console_host` is the console's host as the browser sees it
/// (`agentrix.website`): the passkey's relying party when a page is drawn.
pub fn console_apps(
    sites: HashMap<String, SiteApps>,
    console_host: &str,
) -> Vec<Arc<dyn ConsoleApp>> {
    let shared = Arc::new(Sites {
        sites,
        console_host: console_host.to_owned(),
    });
    [Kind::Passkey, Kind::Badges, Kind::Wallet]
        .into_iter()
        .map(|kind| {
            Arc::new(App {
                kind,
                sites: Arc::clone(&shared),
            }) as Arc<dyn ConsoleApp>
        })
        .collect()
}

struct Sites {
    sites: HashMap<String, SiteApps>,
    console_host: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Passkey,
    Badges,
    Wallet,
}

impl Kind {
    fn id(self) -> &'static str {
        match self {
            Kind::Passkey => "passkey",
            Kind::Badges => "badges",
            Kind::Wallet => "wallet",
        }
    }
}

struct App {
    kind: Kind,
    sites: Arc<Sites>,
}

impl App {
    /// The site's apps as the shell serves them: under its `/console/{site}/apps`, field only,
    /// and open to whoever the shell let through (it has checked they own the site).
    fn site(&self, ctx: &AppContext) -> Option<SiteApps> {
        let mut apps = self.sites.sites.get(&ctx.site_id)?.clone();
        apps.base = format!("{}/apps", ctx.console.trim_end_matches('/'));
        apps.fragment = true;
        apps.access = Access {
            owner_token: None,
            open: true,
        };
        if !ctx.owner_name.is_empty() {
            apps.owner = ctx.owner_name.clone();
        }
        Some(apps)
    }

    fn uri(&self, path: &str, query: &str) -> String {
        let mut uri = format!("/{}", self.kind.id());
        if !path.is_empty() {
            uri.push('/');
            uri.push_str(path);
        }
        if !query.is_empty() {
            uri.push('?');
            uri.push_str(query);
        }
        uri
    }
}

fn location(res: &axum::response::Response) -> Option<String> {
    res.headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn is_json(res: &axum::response::Response) -> bool {
    res.headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[async_trait]
impl ConsoleApp for App {
    fn spec(&self) -> AppSpec {
        let (name, title, icon, webagent, order) = match self.kind {
            Kind::Passkey => (
                "Passkey",
                "Passkey: linking PayPal, the mandate, and each step up, signed by you",
                "grid",
                None,
                70,
            ),
            Kind::Badges => (
                "Badges",
                "Badges: who each agent is, who answers for it, and Verify",
                "grid",
                None,
                71,
            ),
            Kind::Wallet => (
                "Wallet",
                "Wallet: every PayPal move the Cashier made, matched to PayPal and the books",
                "billing",
                Some("Cashier".to_owned()),
                72,
            ),
        };
        AppSpec {
            id: self.kind.id().to_owned(),
            name: name.to_owned(),
            title: title.to_owned(),
            icon: icon.to_owned(),
            webagent,
            order,
        }
    }

    async fn available(&self, ctx: &AppContext) -> bool {
        self.sites.sites.contains_key(&ctx.site_id)
    }

    /// The Passkey app's badge: the moments waiting for the owner.
    async fn badge(&self, ctx: &AppContext) -> anyhow::Result<u32> {
        let (Kind::Passkey, Some(apps)) = (self.kind, self.sites.sites.get(&ctx.site_id)) else {
            return Ok(0);
        };
        let waiting = apps
            .ceremonies
            .pending(&apps.site)
            .await
            .map(|m| m.iter().filter(|s| s.state == "pending").count())
            .unwrap_or(0);
        Ok(u32::try_from(waiting).unwrap_or(u32::MAX))
    }

    async fn page(&self, ctx: &AppContext, path: &str, query: &str) -> anyhow::Result<AppPage> {
        let Some(apps) = self.site(ctx) else {
            return Ok(AppPage::NotFound);
        };
        let req = Request::builder()
            .method(Method::GET)
            .uri(self.uri(path, query))
            .header(header::HOST, &self.sites.console_host)
            .header(header::ACCEPT, "text/html")
            .body(Body::empty())?;
        let res = console_router(apps).oneshot(req).await?;
        if let Some(to) = location(&res).filter(|_| res.status().is_redirection()) {
            return Ok(AppPage::Redirect(to));
        }
        let status = res.status();
        let title = res.extensions().get::<FragmentTitle>().map(|t| t.0.clone());
        let json = is_json(&res);
        let body = axum::body::to_bytes(res.into_body(), 8 * 1024 * 1024).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(match title {
            Some(title) => AppPage::Field {
                title,
                html: text.into_owned(),
            },
            None if status == StatusCode::NOT_FOUND => AppPage::NotFound,
            // A JSON read opened as a page (a form without its script): show it as text.
            None if json => AppPage::Field {
                title: self.spec().name,
                html: format!(r#"<pre class="cmd">{}</pre>"#, esc(&text)),
            },
            None => AppPage::Field {
                title: self.spec().name,
                html: format!(r#"<p class="lead">{}</p>"#, esc(&text)),
            },
        })
    }

    async fn post(
        &self,
        ctx: &AppContext,
        path: &str,
        headers: &HeaderMap,
        body: Bytes,
    ) -> anyhow::Result<AppReply> {
        let Some(apps) = self.site(ctx) else {
            return Ok(AppReply::NotFound);
        };
        // The browser's own host and origin: the passkey's relying party is checked on them.
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or(&self.sites.console_host)
            .to_owned();
        let mut req = Request::builder()
            .method(Method::POST)
            .uri(self.uri(path, ""))
            .header(header::HOST, host);
        for name in [header::CONTENT_TYPE, header::ACCEPT] {
            if let Some(v) = headers.get(&name) {
                req = req.header(name, v);
            }
        }
        let res = console_router(apps)
            .oneshot(req.body(Body::from(body))?)
            .await?;
        let status = res.status();
        if let Some(to) = location(&res).filter(|_| status.is_redirection()) {
            return Ok(AppReply::Redirect(to));
        }
        let json = is_json(&res);
        let bytes = axum::body::to_bytes(res.into_body(), 8 * 1024 * 1024).await?;
        if json {
            let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            return Ok(AppReply::Json(status, v));
        }
        if status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED {
            return Ok(AppReply::NotFound);
        }
        let words = String::from_utf8_lossy(&bytes).trim().to_owned();
        Ok(AppReply::Error(
            status,
            if words.is_empty() {
                format!("The app answered {status}.")
            } else {
                words
            },
        ))
    }
}
