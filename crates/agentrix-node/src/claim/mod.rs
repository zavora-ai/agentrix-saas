//! Claim: prove you control the site (R3, design §4.1). The owner adds a DNS TXT record or
//! uploads one file; the platform checks it **from the sandbox VM** (R3.1) and records what it
//! saw. Nothing non-public is replayed, copied or rebuilt until a claim is verified (R3.2):
//! the Move asks [`Claims::is_verified`] first.
//!
//! Lane: `control-plane` (specs/first-release/design.md §3.0).
//!
//! | Route | Body | Reply |
//! |---|---|---|
//! | `GET /sites/{id}/claim` | `?method=dns\|file` | The claim page (`Page::Claim`); signed out: sign-in, then back |
//! | `POST /sites/{id}/claim` | — | [`Instructions`]: the TXT record and the file |
//! | `POST /sites/{id}/claim/verify` | `{method?: "dns" \| "file"}` | [`Outcome`]: verified, or what was seen in plain words |
//!
//! The page's form posts `method=dns|file` to the same verify route; a form post gets the page
//! back (what was seen, in words) or, once proved, a redirect to the page in its verified state.
//!
//! The proof formats match `concept/journeys/owner-domain.html`:
//! - TXT at `_agentrix.<domain>`: `agentrix-site-verification=<code>`;
//! - file at `https://<domain>/.well-known/agentrix-verify.txt` holding that line.
//!
//! Wiring (the integration lead): `claim::router(Claims::new(pool, probe, sites, events, cfg))`.
//! The probe is the broker: a sandbox `fetch` job per check (see [`ClaimProbe`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agentrix_console::{Console, Page, view};
use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::SqlitePool;
use url::Url;
use uuid::Uuid;

use crate::auth::{AuthState, Session};

/// The schema of the claims table, for the data lane's migrator.
pub const SCHEMA_SQL: &str = include_str!("schema.sql");

/// The TXT record's label.
pub const TXT_LABEL: &str = "_agentrix";
/// The proof file's path.
pub const FILE_PATH: &str = "/.well-known/agentrix-verify.txt";
/// The value both proofs carry, before the code.
pub const PREFIX: &str = "agentrix-site-verification=";
/// The least time between two checks of one claim: a check is a sandbox job, and the owner is
/// told what was seen rather than retried in a loop.
pub const MIN_CHECK_INTERVAL: Duration = Duration::from_secs(10);

// ---- what other parts supply ------------------------------------------------------------------

/// Looks things up from the sandbox VM (R3.1). The production implementation is the broker:
/// - `txt`: a `fetch` job doing DNS over HTTPS (`https://dns.google/resolve?name=…&type=TXT`)
///   with allowlist `[dns.google]`, so the lookup leaves through the egress proxy;
/// - `file`: a `fetch` job for the URL with allowlist `[<domain>]`, no redirects to other
///   hosts, at most 4 KiB of body.
///
/// Results are what the sandbox saw; nothing is fetched from the control VM.
#[async_trait]
pub trait ClaimProbe: Send + Sync + 'static {
    /// The TXT strings at `name` (each record's strings joined).
    async fn txt(&self, name: &str) -> Result<Vec<String>, ProbeError>;
    /// The status and body of `url`.
    async fn file(&self, url: &Url) -> Result<FileSeen, ProbeError>;
}

/// What a file fetch saw.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSeen {
    /// HTTP status.
    pub status: u16,
    /// Up to 4 KiB of body, as text.
    pub body: String,
}

/// A probe that could not run, in plain words (for example "the sandbox is not reachable").
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct ProbeError(pub String);

/// The sites table belongs to another lane; the claim needs two things from it.
#[async_trait]
pub trait SiteDirectory: Send + Sync + 'static {
    /// The site's domain (from its assessment), if the site exists.
    async fn domain_of(&self, site: &str) -> Option<String>;
    /// Records `account` as the site's owner once a claim is verified.
    async fn set_owner(&self, site: &str, account: Uuid) -> Result<(), String>;
}

/// Told when a claim is verified, so the data lane's receipts store can write the
/// `claim.verified` receipt (design §4.4).
#[async_trait]
pub trait ClaimEvents: Send + Sync + 'static {
    /// One verified claim.
    async fn verified(&self, claim: &Verified);
}

/// A verified claim, for its receipt.
#[derive(Debug, Clone, Serialize)]
pub struct Verified {
    /// Claim id.
    pub claim: Uuid,
    /// Site id.
    pub site: String,
    /// Who proved it.
    pub account: Uuid,
    /// The domain.
    pub domain: String,
    /// `dns` or `file`.
    pub method: &'static str,
    /// What the sandbox saw.
    pub evidence: serde_json::Value,
    /// Unix seconds.
    pub at: i64,
}

/// Claim settings.
#[derive(Debug, Clone, Default)]
pub struct ClaimConfig {
    /// Zavora's own fixture domain (R3.3): its claims are labelled as Zavora's fixture.
    pub fixture_domain: Option<String>,
}

// ---- the service ------------------------------------------------------------------------------

/// Claims over SQLite, checked through a [`ClaimProbe`].
#[derive(Clone)]
pub struct Claims {
    pool: SqlitePool,
    probe: Arc<dyn ClaimProbe>,
    sites: Arc<dyn SiteDirectory>,
    events: Arc<dyn ClaimEvents>,
    config: Arc<ClaimConfig>,
    last_check: Arc<Mutex<HashMap<Uuid, Instant>>>,
}

impl std::fmt::Debug for Claims {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Claims")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// What the owner is asked to do.
#[derive(Debug, Clone, Serialize)]
pub struct Instructions {
    /// Claim id.
    pub claim: Uuid,
    /// The domain being claimed.
    pub domain: String,
    /// The code inside both proofs.
    pub code: String,
    /// The TXT record to add.
    pub dns: TxtProof,
    /// The file to upload.
    pub file: FileProof,
    /// R3.3: this is Zavora's own fixture site.
    pub fixture: bool,
    /// Already verified (asking again shows the same proof).
    pub verified: bool,
}

/// The TXT record.
#[derive(Debug, Clone, Serialize)]
pub struct TxtProof {
    /// `_agentrix.<domain>`.
    pub name: String,
    /// The label to type at most registrars (`_agentrix`).
    pub host: String,
    /// `agentrix-site-verification=<code>`.
    pub value: String,
}

/// The proof file.
#[derive(Debug, Clone, Serialize)]
pub struct FileProof {
    /// Where it must open.
    pub url: String,
    /// The file name to download.
    pub filename: String,
    /// Its whole content.
    pub body: String,
}

/// The result of a check.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    /// Proved. The Move may start.
    Verified {
        /// `dns` or `file`.
        method: &'static str,
        /// Unix seconds.
        at: i64,
        /// "kilele-bakery.example is yours · proved by DNS record".
        message: String,
    },
    /// Not proved yet; each line says what was seen.
    NotYet {
        /// Plain words, one per method checked.
        seen: Vec<String>,
    },
}

/// Claim errors, answered as `{message}`.
#[derive(Debug, thiserror::Error)]
pub enum ClaimError {
    /// Not signed in.
    #[error("Sign in first.")]
    SignedOut,
    /// Unknown site.
    #[error("There's no site with that id.")]
    NoSite,
    /// The site's domain cannot be claimed (an IP address, localhost, a single label…).
    #[error("{0}")]
    BadDomain(String),
    /// The request body was not understood.
    #[error("{0}")]
    BadRequest(String),
    /// No claim has been started for this site by this owner.
    #[error("Start the claim first, then add the record or file.")]
    NotStarted,
    /// Checked too recently.
    #[error("We checked less than 10 seconds ago. Wait a few seconds, then press Check again.")]
    TooSoon,
    /// Another account proved this site first.
    #[error(
        "Someone else has already proved they control this site. Contact support if that's wrong."
    )]
    OwnedElsewhere,
    /// A fault on our side.
    #[error("Something went wrong on our side. Nothing was changed. Try again in a moment.")]
    Internal(String),
}

impl IntoResponse for ClaimError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::SignedOut => StatusCode::UNAUTHORIZED,
            Self::NoSite => StatusCode::NOT_FOUND,
            Self::BadDomain(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotStarted => StatusCode::CONFLICT,
            Self::TooSoon => StatusCode::TOO_MANY_REQUESTS,
            Self::OwnedElsewhere => StatusCode::FORBIDDEN,
            Self::Internal(detail) => {
                tracing::error!(%detail, "claim: internal error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, Json(json!({ "message": self.to_string() }))).into_response()
    }
}

impl From<sqlx::Error> for ClaimError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// `xxxx-xxxx-xxxx-xxxx`, 64 random bits, lower-case hex.
fn new_code() -> String {
    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let hex = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}",
        &hex[0..4],
        &hex[4..8],
        &hex[8..12],
        &hex[12..16]
    )
}

/// A domain someone can claim: a public DNS name, lower-case ASCII, no trailing dot.
pub fn normalise_domain(raw: &str) -> Result<String, ClaimError> {
    let trimmed = raw.trim().trim_end_matches('.');
    let host = url::Host::parse(trimmed)
        .map_err(|_| ClaimError::BadDomain(format!("“{trimmed}” is not a domain name.")))?;
    let url::Host::Domain(domain) = host else {
        return Err(ClaimError::BadDomain(
            "An IP address can't be claimed; use the site's domain name.".into(),
        ));
    };
    let domain = domain.to_ascii_lowercase();
    let single_label = !domain.contains('.');
    let local = domain == "localhost"
        || [".localhost", ".local", ".internal", ".lan", ".home.arpa"]
            .iter()
            .any(|s| domain.ends_with(s));
    if single_label || local || domain.len() > 253 {
        return Err(ClaimError::BadDomain(format!(
            "{domain} is not a public domain name."
        )));
    }
    Ok(domain)
}

/// The proof line for a code.
pub fn proof_line(code: &str) -> String {
    format!("{PREFIX}{code}")
}

/// Does a TXT answer prove `code`? Registrars sometimes keep quotes or add spaces.
pub fn txt_proves(records: &[String], code: &str) -> bool {
    let want = proof_line(code);
    records
        .iter()
        .any(|r| r.trim().trim_matches('"').trim() == want)
}

/// Does a file body prove `code`? Any line equal to the proof line, or to the bare code.
pub fn file_proves(body: &str, code: &str) -> bool {
    let want = proof_line(code);
    body.lines()
        .map(str::trim)
        .any(|line| line == want || line == code)
}

impl Claims {
    /// Claims over `pool` (whose schema includes [`SCHEMA_SQL`]).
    pub fn new(
        pool: SqlitePool,
        probe: Arc<dyn ClaimProbe>,
        sites: Arc<dyn SiteDirectory>,
        events: Arc<dyn ClaimEvents>,
        config: ClaimConfig,
    ) -> Self {
        Self {
            pool,
            probe,
            sites,
            events,
            config: Arc::new(config),
            last_check: Arc::default(),
        }
    }

    /// Applies `schema.sql` (tests, and before the data lane's migrator includes it).
    pub async fn ensure_schema(&self) -> Result<(), ClaimError> {
        sqlx::raw_sql(SCHEMA_SQL).execute(&self.pool).await?;
        Ok(())
    }

    /// Whether the site has a verified claim (R3.2: the Move's gate).
    pub async fn is_verified(&self, site: &str) -> Result<bool, ClaimError> {
        Ok(self.owner(site).await?.is_some())
    }

    /// The account whose claim on the site is verified, if any.
    pub async fn owner(&self, site: &str) -> Result<Option<Uuid>, ClaimError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT account_id FROM claims WHERE site_id = ? AND state = 'verified' ORDER BY verified_at LIMIT 1")
                .bind(site)
                .fetch_optional(&self.pool)
                .await?;
        row.map(|(a,)| Uuid::parse_str(&a).map_err(|e| ClaimError::Internal(e.to_string())))
            .transpose()
    }

    /// Starts (or shows again) this owner's claim on the site.
    pub async fn start(&self, site: &str, account: Uuid) -> Result<Instructions, ClaimError> {
        let domain =
            normalise_domain(&self.sites.domain_of(site).await.ok_or(ClaimError::NoSite)?)?;
        if let Some(owner) = self.owner(site).await? {
            if owner != account {
                return Err(ClaimError::OwnedElsewhere);
            }
        }
        let existing: Option<(String, String, String)> = sqlx::query_as(
            "SELECT id, code, state FROM claims WHERE site_id = ? AND account_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(site)
        .bind(account.to_string())
        .fetch_optional(&self.pool)
        .await?;
        let (id, code, verified) = match existing {
            Some((id, code, state)) => (
                Uuid::parse_str(&id).map_err(|e| ClaimError::Internal(e.to_string()))?,
                code,
                state == "verified",
            ),
            None => {
                let id = Uuid::now_v7();
                let code = new_code();
                sqlx::query("INSERT INTO claims (id, site_id, account_id, domain, code, state, created_at) VALUES (?, ?, ?, ?, ?, 'pending', ?)")
                    .bind(id.to_string())
                    .bind(site)
                    .bind(account.to_string())
                    .bind(&domain)
                    .bind(&code)
                    .bind(now())
                    .execute(&self.pool)
                    .await?;
                (id, code, false)
            }
        };
        Ok(self.instructions(id, &domain, &code, verified))
    }

    fn instructions(&self, claim: Uuid, domain: &str, code: &str, verified: bool) -> Instructions {
        Instructions {
            claim,
            domain: domain.to_owned(),
            code: code.to_owned(),
            dns: TxtProof {
                name: format!("{TXT_LABEL}.{domain}"),
                host: TXT_LABEL.to_owned(),
                value: proof_line(code),
            },
            file: FileProof {
                url: format!("https://{domain}{FILE_PATH}"),
                filename: "agentrix-verify.txt".into(),
                body: format!("{}\n", proof_line(code)),
            },
            fixture: self.config.fixture_domain.as_deref() == Some(domain),
            verified,
        }
    }

    /// Checks the proof from the sandbox. `method` limits the check to one proof.
    pub async fn verify(
        &self,
        site: &str,
        account: Uuid,
        method: Option<Method>,
    ) -> Result<Outcome, ClaimError> {
        let row: Option<(String, String, String, String)> = sqlx::query_as(
            "SELECT id, domain, code, state FROM claims WHERE site_id = ? AND account_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(site)
        .bind(account.to_string())
        .fetch_optional(&self.pool)
        .await?;
        let (id, domain, code, state) = row.ok_or(ClaimError::NotStarted)?;
        let id = Uuid::parse_str(&id).map_err(|e| ClaimError::Internal(e.to_string()))?;
        if state == "verified" {
            return Ok(Outcome::Verified {
                method: "earlier",
                at: now(),
                message: format!("{domain} is yours. It was proved earlier."),
            });
        }
        if let Some(owner) = self.owner(site).await? {
            if owner != account {
                return Err(ClaimError::OwnedElsewhere);
            }
        }
        {
            let mut last = self.last_check.lock().unwrap_or_else(|p| p.into_inner());
            if last
                .get(&id)
                .is_some_and(|t| t.elapsed() < MIN_CHECK_INTERVAL)
            {
                return Err(ClaimError::TooSoon);
            }
            last.insert(id, Instant::now());
        }

        let mut seen = Vec::new();
        let mut evidence = serde_json::Map::new();
        let mut proved: Option<&'static str> = None;

        if method != Some(Method::File) {
            let name = format!("{TXT_LABEL}.{domain}");
            match self.probe.txt(&name).await {
                Ok(records) => {
                    evidence.insert("dns".into(), json!({ "name": name, "records": records }));
                    if txt_proves(&records, &code) {
                        proved = Some("dns");
                    } else if records.is_empty() {
                        seen.push(format!(
                            "No TXT record at {name} yet. New records often take a few minutes to show, sometimes an hour or more."
                        ));
                    } else {
                        seen.push(format!("There's a TXT record at {name}, but not with the value above. Copy the value exactly, then press Check again."));
                    }
                }
                Err(e) => {
                    evidence.insert("dns".into(), json!({ "name": name, "error": e.0 }));
                    seen.push(format!(
                        "We couldn't look up {name} from the sandbox: {e}. Press Check again in a minute."
                    ));
                }
            }
        }
        if proved.is_none() && method != Some(Method::Dns) {
            let url = Url::parse(&format!("https://{domain}{FILE_PATH}"))
                .map_err(|e| ClaimError::Internal(e.to_string()))?;
            match self.probe.file(&url).await {
                Ok(file) => {
                    let body: String = file.body.chars().take(512).collect();
                    evidence.insert(
                        "file".into(),
                        json!({ "url": url.as_str(), "status": file.status, "body": body }),
                    );
                    if file.status == 200 && file_proves(&file.body, &code) {
                        proved = Some("file");
                    } else if file.status == 200 {
                        seen.push(format!("{url} opens, but it holds a different code. Download the file again and upload it."));
                    } else if file.status == 404 {
                        seen.push(format!("Nothing at {url} yet: your site answered \"not found\" (HTTP 404). Check that the file is in a folder named .well-known."));
                    } else {
                        seen.push(format!("Your site answered HTTP {} for {url} instead of the file. Open that address in your browser to see what it shows.", file.status));
                    }
                }
                Err(e) => {
                    evidence.insert("file".into(), json!({ "url": url.as_str(), "error": e.0 }));
                    seen.push(format!(
                        "We couldn't open {url} from the sandbox: {e}. Press Check again in a minute."
                    ));
                }
            }
        }

        let at = now();
        let evidence = serde_json::Value::Object(evidence);
        let Some(method) = proved else {
            sqlx::query("UPDATE claims SET evidence = ?, checked_at = ? WHERE id = ?")
                .bind(evidence.to_string())
                .bind(at)
                .bind(id.to_string())
                .execute(&self.pool)
                .await?;
            return Ok(Outcome::NotYet { seen });
        };
        sqlx::query("UPDATE claims SET state = 'verified', method = ?, evidence = ?, checked_at = ?, verified_at = ? WHERE id = ?")
            .bind(method)
            .bind(evidence.to_string())
            .bind(at)
            .bind(at)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        self.sites
            .set_owner(site, account)
            .await
            .map_err(ClaimError::Internal)?;
        self.events
            .verified(&Verified {
                claim: id,
                site: site.to_owned(),
                account,
                domain: domain.clone(),
                method,
                evidence,
                at,
            })
            .await;
        let how = if method == "dns" {
            "DNS record"
        } else {
            "file"
        };
        Ok(Outcome::Verified {
            method,
            at,
            message: format!("{domain} is yours · proved by {how}"),
        })
    }
}

/// Which proof to check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    /// The TXT record.
    Dns,
    /// The file.
    File,
}

// ---- routes -----------------------------------------------------------------------------------

/// State for the claim routes: the claims, sign-in (for the session) and the console templates.
#[derive(Debug, Clone)]
pub struct ClaimState {
    /// The claims.
    pub claims: Claims,
    /// Sign-in, for [`Session`].
    pub auth: AuthState,
    /// Renders the claim page.
    pub console: Arc<Console>,
    /// W4's pages, drawn as `concept/journeys/owner-domain.html`. Without them the console's
    /// template renders.
    pub pages: Option<Arc<agentrix_onboarding::Pages>>,
}

impl axum::extract::FromRef<ClaimState> for AuthState {
    fn from_ref(state: &ClaimState) -> Self {
        state.auth.clone()
    }
}

#[derive(Debug, Default, Deserialize)]
struct VerifyBody {
    method: Option<Method>,
}

#[derive(Debug, Default, Deserialize)]
struct PageQuery {
    method: Option<String>,
}

/// `GET /sites/{id}/claim` (the page), `POST /sites/{id}/claim` and `POST /sites/{id}/claim/verify`.
pub fn router(state: ClaimState) -> Router {
    Router::new()
        .route("/sites/{id}/claim", get(page).post(start))
        .route("/sites/{id}/claim/verify", post(verify))
        .with_state(state)
}

fn method_of(raw: Option<&str>) -> Option<Method> {
    match raw.map(str::trim) {
        Some("dns") => Some(Method::Dns),
        Some("file") => Some(Method::File),
        _ => None,
    }
}

/// Sign-in, then back to this site's claim page.
fn sign_in_first(site: &str) -> Response {
    let back: String =
        url::form_urlencoded::byte_serialize(format!("/sites/{site}/claim").as_bytes()).collect();
    Redirect::to(&format!("/auth/signin?next={back}")).into_response()
}

/// A plain page for a claim that can't be shown (no such site, owned by someone else, a fault).
fn plain_error(error: ClaimError) -> Response {
    let status = error.into_response().status();
    let words = match status {
        StatusCode::NOT_FOUND => "We can't find that site. Open the link from your health report again, or run a new check.".to_owned(),
        StatusCode::FORBIDDEN => "Someone else has already proved they control this site. Contact support if that's wrong.".to_owned(),
        StatusCode::UNPROCESSABLE_ENTITY => "This site's address can't be claimed. Check the site again by its domain name, like your-business.example.".to_owned(),
        _ => "Something went wrong on our side. Nothing was changed. Try again in a moment.".to_owned(),
    };
    (status, Html(format!("<!doctype html><title>Claim</title><link rel=\"stylesheet\" href=\"/static/app.css\"><main class=\"stack\" style=\"padding:24px\"><p>{words}</p><p><a href=\"/\">Back to Agentrix</a></p></main>"))).into_response()
}

/// What the last check saw, for the page.
struct Checked {
    state: view::ClaimState,
    said: Option<String>,
    at: i64,
}

/// Renders the claim page for this owner (starting the claim if needed).
async fn render_page(
    state: &ClaimState,
    site: &str,
    account: Uuid,
    method: Option<Method>,
    checked: Option<Checked>,
) -> Response {
    let ins = match state.claims.start(site, account).await {
        Ok(ins) => ins,
        Err(e) => return plain_error(e),
    };
    let owner = match state.auth.account_summary(account).await {
        Ok(summary) => summary.map(|s| s.display_name).unwrap_or_default(),
        Err(e) => {
            tracing::warn!(error = %e, "claim page: account summary");
            String::new()
        }
    };
    let (claim_state, said, checked_words) = match (ins.verified, checked) {
        (true, _) => (view::ClaimState::Verified, None, None),
        (false, Some(c)) => {
            let at = chrono::DateTime::from_timestamp(c.at, 0).unwrap_or_default();
            (
                c.state,
                c.said,
                Some(format!(
                    "Checked {} UTC from the sandbox",
                    at.format("%H:%M")
                )),
            )
        }
        (false, None) => (view::ClaimState::Pending, None, None),
    };
    let view = view::ClaimView {
        chrome: view::Chrome {
            site: view::SiteRef {
                id: site.to_owned(),
                host: ins.domain.clone(),
                name: ins.domain.clone(),
                state_words: "In setup".into(),
                fixture: ins.fixture,
                live_url: None,
            },
            nav: view::Nav {
                active: view::NavTab::Move,
                decisions_waiting: 0,
            },
            owner: view::OwnerRef { name: owner },
        },
        claim: view::Claim {
            method: if method == Some(Method::File) {
                view::ClaimMethod::File
            } else {
                view::ClaimMethod::Dns
            },
            dns_name: ins.dns.name,
            dns_value: ins.dns.value,
            file_name: ins.file.filename,
            file_url: ins.file.url,
            token: ins.file.body.trim_end().to_owned(),
            state: claim_state,
            checked_words,
            said,
        },
    };
    let html = match &state.pages {
        Some(p) => p
            .render_claim(&agentrix_onboarding::view::ClaimPage::from(view))
            .map_err(|e| e.to_string()),
        None => state
            .console
            .render(Page::Claim, &view)
            .map_err(|e| e.to_string()),
    };
    match html {
        Ok(html) => Html(html).into_response(),
        Err(e) => plain_error(ClaimError::Internal(format!(
            "rendering the claim page: {e}"
        ))),
    }
}

/// `GET /sites/{id}/claim`: the page the assessment's "Move this site" lands on after sign-in.
async fn page(
    State(state): State<ClaimState>,
    headers: HeaderMap,
    Path(site): Path<String>,
    Query(q): Query<PageQuery>,
) -> Response {
    let session = match state.auth.session_from(&headers).await {
        Ok(Some(session)) => session,
        Ok(None) => return sign_in_first(&site),
        Err(e) => return plain_error(ClaimError::Internal(e.to_string())),
    };
    render_page(
        &state,
        &site,
        session.account,
        method_of(q.method.as_deref()),
        None,
    )
    .await
}

async fn start(
    State(state): State<ClaimState>,
    session: Session,
    Path(site): Path<String>,
) -> Result<Json<Instructions>, ClaimError> {
    Ok(Json(state.claims.start(&site, session.account).await?))
}

fn is_form(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"))
}

/// The page's "Check now": the page again with what was seen, or the verified page.
async fn verify_form(state: &ClaimState, site: &str, account: Uuid, body: &[u8]) -> Response {
    let method = url::form_urlencoded::parse(body)
        .find(|(k, _)| k == "method")
        .and_then(|(_, v)| method_of(Some(&v)));
    let checked = match state.claims.verify(site, account, method).await {
        Ok(Outcome::Verified { .. }) => {
            let back = format!("/sites/{site}/claim");
            return Redirect::to(&back).into_response();
        }
        Ok(Outcome::NotYet { seen }) => Checked {
            state: view::ClaimState::Failed,
            said: Some(seen.join(" ")),
            at: now(),
        },
        Err(e @ (ClaimError::TooSoon | ClaimError::NotStarted)) => Checked {
            state: view::ClaimState::Failed,
            said: Some(e.to_string()),
            at: now(),
        },
        Err(e) => return plain_error(e),
    };
    render_page(state, site, account, method, Some(checked)).await
}

async fn verify(
    State(state): State<ClaimState>,
    headers: HeaderMap,
    Path(site): Path<String>,
    body: axum::body::Bytes,
) -> Result<Response, ClaimError> {
    if is_form(&headers) {
        return Ok(match state.auth.session_from(&headers).await {
            Ok(Some(session)) => verify_form(&state, &site, session.account, &body).await,
            Ok(None) => sign_in_first(&site),
            Err(e) => plain_error(ClaimError::Internal(e.to_string())),
        });
    }
    let session = state
        .auth
        .session_from(&headers)
        .await
        .map_err(|e| ClaimError::Internal(e.to_string()))?
        .ok_or(ClaimError::SignedOut)?;
    // The body is optional: `{}` or nothing checks both proofs.
    let method = if body.iter().all(u8::is_ascii_whitespace) {
        None
    } else {
        serde_json::from_slice::<VerifyBody>(&body)
            .map_err(|_| {
                ClaimError::BadRequest(
                    "Send {\"method\": \"dns\"} or {\"method\": \"file\"}, or nothing.".into(),
                )
            })?
            .method
    };
    Ok(Json(state.claims.verify(&site, session.account, method).await?).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoProof;
    #[async_trait]
    impl ClaimProbe for NoProof {
        async fn txt(&self, _name: &str) -> Result<Vec<String>, ProbeError> {
            Ok(vec![])
        }
        async fn file(&self, _url: &Url) -> Result<FileSeen, ProbeError> {
            Ok(FileSeen {
                status: 404,
                body: String::new(),
            })
        }
    }
    struct OneSite;
    #[async_trait]
    impl SiteDirectory for OneSite {
        async fn domain_of(&self, site: &str) -> Option<String> {
            (site == "site_k").then(|| "kilele-bakery.example".to_owned())
        }
        async fn set_owner(&self, _site: &str, _account: Uuid) -> Result<(), String> {
            Ok(())
        }
    }
    struct Quiet;
    #[async_trait]
    impl ClaimEvents for Quiet {
        async fn verified(&self, _claim: &Verified) {}
    }

    /// The browser path: "Move this site" signs in, then GETs the claim page; its form posts.
    #[tokio::test]
    async fn page_and_form() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt as _;

        let pool = crate::db::open_in_memory().await.unwrap();
        let auth = AuthState::new(
            crate::auth::AuthConfig::for_origin(Url::parse("http://localhost:18080").unwrap())
                .unwrap(),
            pool.clone(),
            crate::auth::Sealer::from_key([7; 32]),
        )
        .unwrap();
        auth.store().ensure_schema().await.unwrap();
        let account = Uuid::now_v7();
        sqlx::query("INSERT INTO accounts (id, email, display_name, created_at) VALUES (?, 'o@kilele-bakery.example', 'Wanjiru', 0)")
            .bind(account.to_string())
            .execute(&pool)
            .await
            .unwrap();
        let claims = Claims::new(
            pool,
            Arc::new(NoProof),
            Arc::new(OneSite),
            Arc::new(Quiet),
            ClaimConfig {
                fixture_domain: None,
            },
        );
        claims.ensure_schema().await.unwrap();
        let cookie = auth.start_session(account, "passkey").await.unwrap();
        let cookie = cookie
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let app = router(ClaimState {
            claims,
            auth,
            console: Arc::new(Console::new().unwrap()),
            pages: None,
        });
        let send = |req: Request<Body>| app.clone().oneshot(req);
        let text = |r: Response| async move {
            String::from_utf8(
                axum::body::to_bytes(r.into_body(), 1 << 20)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap()
        };

        // Signed out: sign in, then back here.
        let r = send(
            Request::get("/sites/site_k/claim")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            r.headers()[header::LOCATION],
            "/auth/signin?next=%2Fsites%2Fsite_k%2Fclaim"
        );

        // Signed in: an HTML page with the TXT record, not a JSON body the browser saves.
        let r = send(
            Request::get("/sites/site_k/claim")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert!(
            r.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        let html = text(r).await;
        assert!(html.contains("_agentrix.kilele-bakery.example"), "{html}");
        assert!(html.contains("agentrix-site-verification="));

        // The file tab.
        let html = text(
            send(
                Request::get("/sites/site_k/claim?method=file")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
        )
        .await;
        assert!(
            html.contains("https://kilele-bakery.example/.well-known/agentrix-verify.txt"),
            "{html}"
        );

        // "Check now" from the page: the page again, saying what was seen.
        let form = Request::post("/sites/site_k/claim/verify")
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("method=dns"))
            .unwrap();
        let r = send(form).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let html = text(r).await;
        assert!(
            html.contains("No TXT record at _agentrix.kilele-bakery.example yet"),
            "{html}"
        );

        // The JSON API is unchanged.
        let r = send(
            Request::post("/sites/site_k/claim")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert!(text(r).await.contains("\"dns\""));

        // Unknown site: a plain page, not a download.
        let r = send(
            Request::get("/sites/site_x/claim")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert!(
            r.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
    }

    #[test]
    fn domains() {
        assert_eq!(
            normalise_domain(" Kilele-Bakery.Example. ").unwrap(),
            "kilele-bakery.example"
        );
        assert_eq!(
            normalise_domain("bücher.example").unwrap(),
            "xn--bcher-kva.example"
        );
        for bad in [
            "",
            "localhost",
            "intranet",
            "printer.local",
            "10.0.0.1",
            "[::1]",
            "a b.example",
        ] {
            assert!(normalise_domain(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn proofs() {
        let code = "k1l3-7f2c-91aa-0b3d";
        assert!(txt_proves(
            &[format!("\"agentrix-site-verification={code}\"")],
            code
        ));
        assert!(txt_proves(
            &[
                "v=spf1 -all".into(),
                format!(" agentrix-site-verification={code} ")
            ],
            code
        ));
        assert!(!txt_proves(
            &["agentrix-site-verification=0000-0000-0000-0000".into()],
            code
        ));
        assert!(!txt_proves(&[], code));
        assert!(file_proves(
            &format!("agentrix-site-verification={code}\n"),
            code
        ));
        assert!(file_proves(code, code));
        assert!(!file_proves("<html>Not found</html>", code));
        let fresh = new_code();
        assert_eq!(fresh.len(), 19);
        assert_eq!(fresh.matches('-').count(), 3);
    }
}
