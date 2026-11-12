//! Node's side of the `agentrix-host` seam (contract C11, `specs/platform/how-we-work.md`).
//!
//! - [`MODULES`]: every platform crate node mounts. Adding a crate is one line here (W0 only).
//! - Adapters from the first release's receipts, sign-in, approvals and content store to the seam's
//!   traits, so crates never depend on `agentrix-node`.
//! - [`ModuleHosts`]: the hosts modules serve, for the `Host` dispatcher and Caddy's TLS ask.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use uuid::Uuid;

use agentrix_host::{
    Approvals as HostApprovals, Blobs, HostRoutes, MigrateFn, ModuleFn, Mounted, NodeContext,
    Owner, OwnerAccount, OwnerAccounts, OwnerSessions, ReceiptBody, Receipts,
};
use agentrix_types::Digest;

use crate::auth::{Approvals, ApproveRequest, AuthState, NewAccount};
use crate::db::cas::Cas;
use crate::db::receipts::ReceiptStore;
use crate::site::{SiteState, TlsAsk, normalise_host, serves_host};

/// Every platform crate's `mount`, in mount order. W0 adds one line per crate at merge.
pub const MODULES: &[ModuleFn] = &[
    // W1: the living marks' script and styles on the platform host.
    agentrix_kernel::mount,
    // W4: onboarding's styles and scripts (its Assess pages wire into assess/ after handover).
    agentrix_onboarding::mount,
    // W7: the design system's tokens, fonts, styles and scripts at /static/ui/.
    agentrix_ui::mount,
    // W10: the home page at / (and /home), privacy, terms, the crawler page, robots.txt and the
    // sitemap. The first release's Start page moved to /assess.
    agentrix_home::mount_with_front_door,
    // W8: /status, /status.json, /api/numbers; loopback metrics; health checks and retention.
    agentrix_store::mount,
    // W11: generated images, the media library and voice; spoken "allow once" runs the same
    // effect runners as a tap.
    media,
    // W3: the owner's side of support grants (/support-access/*), on the platform host.
    agentrix_operator::mount_platform,
    // W5: the owner console as Agentrix OS (/os, /console/{site}/..., /static/os/*), with the
    // apps other crates plug into its dock.
    console_shell,
    // W9: the message catalogue, the outbox at /console/messages, quiet-hours release.
    agentrix_messages::mount,
    // W6: owner email codes, recovery codes and the 24-hour recovery hold, with W9's mailer.
    owner_identity,
    // W2: the three sample sites on their -sample hosts (records.db under data_dir), with
    // customers' sign-in codes sent through W9's catalogue and outbox.
    sample_sites,
    // W6: the Effects Gateway's PayPal webhook (POST /effects/paypal/webhook); payments stay off
    // until the sandbox keys exist.
    agentrix_effects::mount,
    // v3 J: the money layer on its own hosts (pay., books., and v2's money. and kilele.money.):
    // both companies' kernels and agents, settlement's agent doors, PayPal's webhook at
    // pay./paypal/webhook, the books on Zavora ERA. It starts in the background and answers 503
    // until it is up.
    saas_node::platform::mount,
    // W5: decision cards v2 (C4) at /decisions/v2/{id}/approve and /decline, through the frozen
    // approve contract, with every crate's effect runners.
    decisions,
    // W13: Telegram for a site's owner and customers (polling locally, the webhook on deploy),
    // the Telegram chat transport behind W9's outbox, and the owner app's API (/api/m/v1).
    channels,
    // W0: the concept stage, "Choose your new look" at /sites/{id}/concepts: three Zavora
    // agents design three looks in parallel for a claimed site (AGENTRIX_ZAVORA_BIN,
    // VULTR_INFERENCE_API_KEY; hidden with a plain message without them).
    agentrix_concepts::mount,
];

/// Every effect a decision card can run once approved: W5's (the Store puts a WebAgent on
/// duty), W1's dial raises and W10's weekly fix (agentrix.website's newsletter, sent to
/// confirmed readers once James approves the Editor's draft). Decisions and W11's spoken
/// decisions get the same list, so a tool without a runner is refused before any approval is
/// recorded.
fn effect_runners(ctx: &NodeContext) -> Vec<std::sync::Arc<dyn agentrix_decisions::EffectRunner>> {
    let mut runners = agentrix_shell::runners(ctx);
    runners.push(agentrix_kernel::dial_runner(ctx));
    runners.push(agentrix_home::newsletter_runner(ctx));
    runners
}

/// W5's decision cards v2 with [`effect_runners`].
fn decisions(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    agentrix_decisions::mount_with(ctx, effect_runners(ctx))
}

/// W13's channels with [`effect_runners`], so approving from the owner app runs the effect.
fn channels(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    agentrix_channels::mount_with(ctx, effect_runners(ctx))
}

/// W11's media and voice with [`effect_runners`].
fn media(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    agentrix_media::mount_with_runners(ctx, effect_runners(ctx))
}

/// W5's shell with the apps other crates add to the dock: W9's Messages (notification settings
/// and what was sent).
fn console_shell(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    agentrix_shell::mount_with(
        ctx,
        agentrix_shell::Plugins {
            apps: vec![
                agentrix_messages::console_app(ctx),
                // W13: Channels, link and unlink Telegram, and what each surface may decide.
                agentrix_channels::console_app(ctx),
                // W6: People, the owner's team and roles, with invitations through W9's mail.
                std::sync::Arc::new(agentrix_identity::PeopleApp::new(
                    agentrix_identity::Team::for_node(ctx, agentrix_messages::identity_mailer(ctx)),
                )),
            ]
            .into_iter()
            // W11: Talk (push to talk, the spoken brief) and Pictures (the media library).
            .chain(agentrix_media::apps::console_apps(ctx))
            // v3 J: face's Passkey, Badges and Wallet over the money layer (SAAS_MODULE=1).
            .chain(saas_node::platform::console_apps(ctx))
            .collect(),
            // W3: support-access requests, live grants and agency-role requests on the owner's
            // own site, in the lane's "Needs you".
            lanes: vec![agentrix_operator::support_lane(ctx)],
        },
    )
}

/// W2's sample sites, with W6's customer accounts sending codes through W9's catalogue and
/// outbox (the log-only mailer delivered nothing, so a customer without a passkey couldn't
/// finish signing up).
fn sample_sites(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    let mounted =
        agentrix_samples::mount_with_mailer(ctx, agentrix_messages::identity_mailer(ctx))?;
    // v3 J: Kilele Bakery's agent door on its sample host, in front of the sample's pages.
    Ok(saas_node::platform::with_sample_doors(ctx, mounted))
}

/// W6's owner sign-in routes, sending their codes through W9's catalogue and outbox.
fn owner_identity(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    agentrix_identity::mount_with_mailer(ctx, agentrix_messages::identity_mailer(ctx))
}

/// Modules mounted with the ops host's context (its approvals verify against the ops origin).
pub const OPS_MODULES: &[ModuleFn] = &[
    // W3: the operator console on ops.<platform host>, with v3 J's activity page in front.
    operator_console,
];

/// W3's operator console, with the money layer's activity page (`/activity`) in front of it on
/// the ops host, for the console's own signed-in operators.
fn operator_console(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    let mounted = agentrix_operator::mount(ctx)?;
    let host = agentrix_operator::ops_host(&ctx.platform_host);
    let mut config =
        agentrix_operator::OperatorConfig::new(agentrix_operator::ops_origin(&ctx.origin, &host));
    config.platform_origin = Some(ctx.origin.clone());
    let operators =
        agentrix_operator::OperatorState::new(config, ctx.pool.clone(), ctx.approvals.clone())?;
    Ok(saas_node::platform::with_activity(
        mounted,
        Arc::new(SignedInOperator(operators)),
    ))
}

/// Who may use the activity page, the dock's agents and the askline: an operator with a live
/// session and at least one platform role.
struct SignedInOperator(agentrix_operator::OperatorState);

#[async_trait]
impl saas_node::platform::Operators for SignedInOperator {
    async fn operator(&self, headers: &HeaderMap) -> Option<saas_node::platform::OperatorId> {
        let operator = self.0.operator_from(headers).await.ok().flatten()?;
        (!operator.roles.is_empty()).then(|| saas_node::platform::OperatorId {
            id: operator.account.to_string(),
            name: operator.name,
        })
    }
}

/// Every platform crate's own migrator, run once at startup before [`MODULES`] mount.
pub const MIGRATIONS: &[MigrateFn] = &[
    // W1: the roster of nine (webagent_sites, webagent_specs).
    agentrix_kernel::migrate,
    // W6: customer accounts on each site's host.
    agentrix_identity::migrate,
    // W8: visit counts, the usage rebuild for image and voice purposes, health hours.
    agentrix_store::migrate,
    // W5: the console's own records (freeze, attention).
    agentrix_shell::migrate,
    // W6: PayPal orders, captures, refunds and plan subscriptions (0610+).
    agentrix_effects::migrate,
    // W5: decision cards v2 (0550).
    agentrix_decisions::migrate,
    // W9: notification preferences and the outbox (own migrations table).
    agentrix_messages::migrate,
    // W4: the owner's plan and AI budget choice (0400, own migrations table).
    agentrix_onboarding::migrate,
    // W13: linked chats, one-time link codes, alerts and chat turns (1300+, own table).
    agentrix_channels::migrate,
    // W10: agentrix.website's "keep this report" contacts and the exact consent words
    // (1000+, own migrations table).
    agentrix_home::migrate,
    // W0: concept runs and the three designs, with the owner's choice (1400+, own table).
    agentrix_concepts::migrate,
];

/// Runs each crate's migrator on `control.db`.
///
/// # Errors
///
/// When a migrator fails; node then refuses to start, naming it.
pub async fn run_migrations(
    pool: &sqlx::SqlitePool,
    migrations: &[MigrateFn],
) -> anyhow::Result<()> {
    for (i, migrate) in migrations.iter().enumerate() {
        migrate(pool)
            .await
            .map_err(|e| anyhow::anyhow!("crate migrator {i} failed: {e:#}"))?;
    }
    Ok(())
}

/// The first release's receipt store as the seam's [`Receipts`].
pub struct NodeReceipts {
    /// The store.
    pub store: Arc<ReceiptStore>,
}

#[async_trait]
impl Receipts for NodeReceipts {
    async fn issue(&self, body: ReceiptBody) -> anyhow::Result<String> {
        Ok(self.store.issue(body).await?.digest.to_string())
    }

    async fn get_json(&self, digest: &str) -> anyhow::Result<Option<String>> {
        let digest: Digest = digest.parse()?;
        self.store.get_json(&digest).await
    }
}

/// The first release's owner sessions as the seam's [`OwnerSessions`].
pub struct NodeOwners {
    /// Sign-in.
    pub auth: AuthState,
}

#[async_trait]
impl OwnerSessions for NodeOwners {
    async fn owner(&self, headers: &HeaderMap) -> anyhow::Result<Option<Owner>> {
        Ok(self.auth.session_from(headers).await?.map(|s| Owner {
            account: s.account,
            method: s.method,
        }))
    }
}

/// The first release's owner accounts as the seam's [`OwnerAccounts`], for W6's email and recovery codes.
/// It reuses `auth/`'s own store and session start; the approve contract is untouched.
pub struct NodeOwnerAccounts {
    /// Sign-in.
    pub auth: AuthState,
}

#[async_trait]
impl OwnerAccounts for NodeOwnerAccounts {
    async fn by_email(&self, email: &str) -> anyhow::Result<Option<OwnerAccount>> {
        let email = email.trim().to_lowercase();
        let Some(account) = self.auth.store().account_by_email(&email).await? else {
            return Ok(None);
        };
        let has_passkey = !self.auth.store().credentials(account.id).await?.is_empty();
        Ok(Some(OwnerAccount {
            account: account.id,
            email: account.email,
            display_name: account.display_name,
            has_passkey,
            has_password: account.password_hash.is_some(),
        }))
    }

    async fn create(&self, email: &str, display_name: &str) -> anyhow::Result<Uuid> {
        let email = email.trim().to_lowercase();
        let id = Uuid::now_v7();
        self.auth
            .store()
            .create_account(
                id,
                NewAccount {
                    email: &email,
                    display_name,
                    password_hash: None,
                    passkey: None,
                    totp: None,
                },
            )
            .await?;
        Ok(id)
    }

    async fn start_session(
        &self,
        account: Uuid,
        method: &str,
    ) -> anyhow::Result<axum::http::HeaderValue> {
        Ok(self.auth.start_session(account, method).await?)
    }
}

/// The first release's approvals (design §4.1, frozen) as the seam's [`HostApprovals`].
pub struct NodeApprovals {
    /// The approvals API.
    pub approvals: Approvals,
}

#[async_trait]
impl HostApprovals for NodeApprovals {
    async fn passkey_options(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
    ) -> anyhow::Result<Option<serde_json::Value>> {
        self.approvals
            .passkey_options(account, fingerprint)
            .await?
            .map(|o| serde_json::to_value(&o.options))
            .transpose()
            .map_err(Into::into)
    }

    async fn verify(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
        request: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let request: ApproveRequest = serde_json::from_value(approve_shape(request))?;
        let proof = self.approvals.verify(account, fingerprint, request).await?;
        Ok(serde_json::to_value(proof)?)
    }
}

/// The first release's tagged approve body (`{"method": "totp", "code": …}`) from either shape crates
/// send: the tagged one, or the short one W1's dials and W3's operator pages use
/// (`{"totp": "123456"}`, `{"passkey": <credential>}`). Before this, every short-shape approval
/// failed to parse and was refused (found by W3).
fn approve_shape(request: serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Object(body) = &request else {
        return request;
    };
    if body.contains_key("method") {
        return request;
    }
    if let Some(code) = body.get("totp") {
        return serde_json::json!({ "method": "totp", "code": code });
    }
    if let Some(credential) = body.get("passkey") {
        return serde_json::json!({ "method": "passkey", "credential": credential });
    }
    request
}

#[cfg(test)]
mod approve_shape_tests {
    use super::approve_shape;
    use serde_json::json;

    #[test]
    fn both_shapes_reach_the_tagged_request() {
        assert_eq!(
            approve_shape(json!({"totp": "123456"})),
            json!({"method": "totp", "code": "123456"})
        );
        assert_eq!(
            approve_shape(json!({"passkey": {"id": "c1"}})),
            json!({"method": "passkey", "credential": {"id": "c1"}})
        );
        let tagged = json!({"method": "totp", "code": "1", "totp": "2"});
        assert_eq!(approve_shape(tagged.clone()), tagged);
        assert_eq!(approve_shape(json!({})), json!({}));
    }
}

/// The first release's content store as the seam's [`Blobs`].
pub struct NodeBlobs {
    /// The content store.
    pub cas: Arc<Cas>,
}

#[async_trait]
impl Blobs for NodeBlobs {
    async fn put(&self, bytes: Bytes) -> anyhow::Result<String> {
        Ok(self.cas.put(&bytes).await?.to_string())
    }

    async fn get(&self, digest: &str) -> anyhow::Result<Option<Bytes>> {
        let digest: Digest = digest.parse()?;
        Ok(self.cas.get(&digest).await?.map(Bytes::from))
    }
}

/// The hosts every module serves, with their routers.
#[derive(Clone, Default)]
pub struct ModuleHosts(pub Arc<Vec<HostRoutes>>);

impl ModuleHosts {
    /// The router of the first module that serves `host`.
    pub async fn router_for(&self, host: &str) -> anyhow::Result<Option<Router>> {
        for routes in self.0.iter() {
            if routes.matcher.serves(host).await? {
                return Ok(Some(routes.router.clone()));
            }
        }
        Ok(None)
    }
}

/// Everything the modules mounted, merged.
pub struct MountedAll {
    /// Routes on the platform host.
    pub platform: Router,
    /// Other hosts.
    pub hosts: ModuleHosts,
    /// Loopback-only routes.
    pub internal: Router,
}

/// Calls every module's `mount` with its context, spawns its tasks and merges what it returns.
///
/// # Errors
///
/// When a module refuses to mount; node then refuses to start, naming it.
pub fn mount_all(sets: &[(&NodeContext, &[ModuleFn])]) -> anyhow::Result<MountedAll> {
    let mut platform = Router::new();
    let mut internal = Router::new();
    let mut hosts = Vec::new();
    let modules = sets
        .iter()
        .flat_map(|(ctx, mounts)| mounts.iter().map(move |m| (*ctx, m)));
    for (i, (ctx, mount)) in modules.enumerate() {
        let Mounted {
            platform: p,
            hosts: h,
            internal: n,
            tasks,
        } = mount(ctx).map_err(|e| anyhow::anyhow!("module {i} failed to mount: {e:#}"))?;
        platform = platform.merge(p);
        internal = internal.merge(n);
        hosts.extend(h);
        for task in tasks {
            let name = task.name;
            tokio::spawn(async move {
                task.future.await;
                tracing::warn!(task = name, "a module's background task ended");
            });
        }
    }
    Ok(MountedAll {
        platform,
        hosts: ModuleHosts(Arc::new(hosts)),
        internal,
    })
}

#[derive(Clone)]
struct TlsState {
    site: SiteState,
    modules: ModuleHosts,
}

/// `GET /internal/tls-ask?domain=` for Caddy's on-demand TLS: yes for a host a module serves or
/// a signed release serves. Like `site::internal_router`, it answers only loopback requests that
/// did not come through the proxy.
pub fn internal_router(site: SiteState, modules: ModuleHosts) -> Router {
    Router::new()
        .route("/internal/tls-ask", get(tls_ask))
        .with_state(TlsState { site, modules })
}

async fn tls_ask(
    State(state): State<TlsState>,
    Query(q): Query<TlsAsk>,
    request: Request,
) -> Response {
    let peer_loopback = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_none_or(|c| c.0.ip().is_loopback());
    let proxied = request.headers().contains_key("x-forwarded-for")
        || request.headers().contains_key("forwarded");
    let host_internal = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(normalise_host)
        .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "[::1]");
    if !peer_loopback || proxied || !host_internal {
        return (StatusCode::NOT_FOUND, "Not found.").into_response();
    }
    let domain = normalise_host(&q.domain);
    match state.modules.router_for(&domain).await {
        Ok(Some(_)) => return StatusCode::OK.into_response(),
        Ok(None) => {}
        Err(e) => {
            tracing::error!(%e, "tls-ask module lookup failed");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    match serves_host(&state.site, &domain).await {
        Ok(true) => StatusCode::OK.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(%e, "tls-ask lookup failed");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentrix_host::HostMatch;
    use axum::routing::get;

    fn module(_ctx: &NodeContext) -> anyhow::Result<Mounted> {
        Ok(Mounted::default()
            .platform(Router::new().route("/m/hello", get(|| async { "hi" })))
            .host(
                HostMatch::Exact("ops.localhost".into()),
                Router::new().route("/", get(|| async { "ops" })),
            ))
    }

    /// James's rule for email codes: they let an owner in, but never approve anything. An owner
    /// created and signed in by email code has no passkey and no authenticator, so the frozen
    /// approve contract refuses both kinds of proof.
    #[tokio::test]
    async fn an_email_code_owner_cannot_approve() -> anyhow::Result<()> {
        use crate::auth::{AuthConfig, Sealer};
        let pool = crate::db::open_in_memory().await?;
        let auth = AuthState::new(
            AuthConfig::for_origin("http://localhost:18050".parse()?)?,
            pool,
            Sealer::from_key([7; 32]),
        )?;
        let accounts = NodeOwnerAccounts { auth: auth.clone() };
        let owner = accounts.create("Owner@Example.org", "Wanjiru").await?;
        let cookie = accounts.start_session(owner, "email-code").await?;
        assert!(!cookie.is_empty());
        let found = accounts
            .by_email("owner@example.org")
            .await?
            .expect("found");
        assert!(!found.has_passkey && !found.has_password);

        let approvals = NodeApprovals {
            approvals: auth.approvals(),
        };
        let fingerprint = [9; 32];
        assert!(
            approvals
                .passkey_options(owner, fingerprint)
                .await?
                .is_none()
        );
        let totp = serde_json::json!({ "method": "totp", "code": "123456" });
        assert!(approvals.verify(owner, fingerprint, totp).await.is_err());
        let passkey = serde_json::json!({
            "method": "passkey",
            "credential": {
                "id": "AAAA", "rawId": "AAAA", "type": "public-key", "extensions": {},
                "response": {
                    "authenticatorData": "AAAA", "clientDataJSON": "AAAA",
                    "signature": "AAAA", "userHandle": null
                }
            }
        });
        assert!(approvals.verify(owner, fingerprint, passkey).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn modules_mount_and_their_hosts_route() -> anyhow::Result<()> {
        let ctx = agentrix_host::testing::context().await?;
        let all = mount_all(&[(&ctx, &[module])])?;
        assert!(all.hosts.router_for("ops.localhost").await?.is_some());
        assert!(all.hosts.router_for("kilele.example").await?.is_none());
        Ok(())
    }
}
