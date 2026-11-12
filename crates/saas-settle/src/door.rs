//! Each company's agent door: A2A v1 (JSON-RPC `SendMessage` and `GetTask`) behind Web Bot
//! Auth (RFC 9421 HTTP Message Signatures, IETF working-group draft).
//!
//! Every `POST /a2a` must carry a signature covering `@authority`, `@method`, `@path` and
//! `Content-Digest` (RFC 9530), made with a key the sender's company publishes at
//! `{Signature-Agent origin}/.well-known/http-message-signatures-directory`, with a nonce the
//! door hasn't seen. Unsigned, wrongly signed, edited and replayed requests are refused here,
//! before the company's handler runs, so no mandate is read and no PayPal call is made for them.
//!
//! A message is one [`SettleMessage`] in a data part. The door records each as an A2A task
//! through adk-server's v1 executor and task store: completed with the [`DoorAnswer`] as its
//! artifact, rejected with the refusal in plain words, or failed when PayPal or the books
//! failed. `GetTask` reads a caller's own tasks back.

use std::sync::Arc;

use a2a_protocol_types::artifact::{Artifact, ArtifactId};
use a2a_protocol_types::{
    AgentCapabilities, AgentSkill, Message, MessageId, MessageRole, Part, PartContent, Task,
    TaskState,
};
use adk_auth::web_bot_auth::{
    ComponentId, Ed25519SigningKey, HttpSignatureSigner, HttpSignatureVerifier, KeyDirectorySource,
    RequestHead, SignatureInput, VerifiedRequester,
};
use adk_server::a2a::v1::V1Executor;
use adk_server::a2a::v1::card::build_v1_agent_card;
use adk_server::a2a::v1::convert::internal_task_to_wire;
use adk_server::a2a::v1::task_store::{InMemoryTaskStore, TaskStore};
use async_trait::async_trait;
use axum::Json;
use axum::body::to_bytes;
use axum::extract::{OriginalUri, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::events::{EventSender, SettleEvent};
use crate::keys::CompanyKeys;
use crate::protocol::{DoorAnswer, MESSAGE_MEDIA_TYPE, RefusalReceipt, SettleMessage};
use crate::seal::{SealKind, Sealed};
use crate::store::Store;
use crate::tokens::{AgentTokens, FederatedPrincipal, bearer};

/// The `Content-Digest` field (RFC 9530), lowercase.
pub const CONTENT_DIGEST: &str = "content-digest";
/// Where the door takes A2A JSON-RPC.
pub const A2A_PATH: &str = "/a2a";
/// The A2A agent card.
pub const AGENT_CARD_PATH: &str = "/.well-known/agent-card.json";
/// The name of the artifact a refused task carries its sealed refusal receipt in.
pub const REFUSAL_ARTIFACT: &str = "refusal_receipt";
/// The largest request body the door reads.
pub const MAX_BODY_BYTES: usize = 256 * 1024;

/// A requester whose signature verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedAgent {
    /// `wba:{directory origin}#{key thumbprint}`.
    pub id: String,
    /// The origin whose key directory published the key.
    pub origin: String,
    /// The JWK thumbprint of the key that signed.
    pub key_thumbprint: String,
    /// Who it is in its own cloud, when it also presented an agent token that checked
    /// (`Authorization: Bearer`, on a door that accepts tokens).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<FederatedPrincipal>,
}

impl From<VerifiedRequester> for VerifiedAgent {
    fn from(r: VerifiedRequester) -> Self {
        Self {
            id: r.id,
            origin: r.directory_origin,
            key_thumbprint: r.key_thumbprint,
            principal: None,
        }
    }
}

/// A refused request or message, in plain words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The HTTP status (for refusals before the handler).
    pub status: StatusCode,
    /// A short code, for example `signature_required` or `mandate_refused`.
    pub code: &'static str,
    /// Plain words.
    pub message: String,
}

impl Refusal {
    /// A refusal.
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    /// The message was understood and refused (a mandate or plan check): the task is rejected.
    pub fn rejected(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::OK, code, message)
    }

    /// Something failed after the checks (PayPal, the books): the task failed.
    pub fn failed(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, code, message)
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        self.into_response_with(None)
    }
}

impl Refusal {
    /// The JSON-RPC error answer, with the sealed refusal receipt in `error.data.receipt`.
    pub fn into_response_with(self, receipt: Option<Sealed>) -> Response {
        (
            self.status,
            Json(json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": {"code": -32001, "message": self.message, "data": {"code": self.code, "receipt": receipt}},
            })),
        )
            .into_response()
    }
}

/// What a company does with a message that passed the door.
#[async_trait]
pub trait DoorHandler: Send + Sync {
    /// Handles one message from a verified agent: an answer, or a refusal in plain words.
    async fn handle(
        &self,
        from: &VerifiedAgent,
        message: SettleMessage,
    ) -> Result<DoorAnswer, Refusal>;
}

/// The signature check in front of the door.
pub struct DoorAuth {
    verifier: HttpSignatureVerifier,
    directories: Arc<dyn KeyDirectorySource>,
    pub(crate) store: Store,
    trusted: Vec<String>,
    any_signed: bool,
}

impl std::fmt::Debug for DoorAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DoorAuth")
            .field("trusted", &self.trusted)
            .finish_non_exhaustive()
    }
}

impl DoorAuth {
    /// Verifies against directories from `directories`, for agents of the `trusted` origins
    /// only (the counterparties this company deals with), with nonces kept in `store`.
    pub fn new(
        directories: Arc<dyn KeyDirectorySource>,
        store: Store,
        trusted: impl IntoIterator<Item = String>,
    ) -> Self {
        let verifier = HttpSignatureVerifier::builder(Arc::clone(&directories))
            .require_component(ComponentId::new("@method"))
            .require_component(ComponentId::new("@path"))
            .require_component(ComponentId::new(CONTENT_DIGEST))
            .default_scheme("https")
            .build();
        Self {
            verifier,
            directories,
            store,
            trusted: trusted
                .into_iter()
                .map(|o| o.trim_end_matches('/').to_owned())
                .collect(),
            any_signed: false,
        }
    }

    /// Verifies any agent whose signature checks against the key directory its request names
    /// (`Signature-Agent`), not only listed counterparties: for a door strangers' agents may
    /// use, such as a sample site's shop door. Content-Digest, the signature and a fresh nonce
    /// are still required.
    pub fn any_signed(directories: Arc<dyn KeyDirectorySource>, store: Store) -> Self {
        Self {
            any_signed: true,
            ..Self::new(directories, store, [])
        }
    }

    /// Where counterparties' key directories are read from.
    pub fn directories(&self) -> Arc<dyn KeyDirectorySource> {
        Arc::clone(&self.directories)
    }

    /// Verifies one request's head and body.
    pub async fn verify(
        &self,
        method: &http::Method,
        uri: &http::Uri,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<VerifiedAgent, Refusal> {
        check_content_digest(headers, body)?;
        let head = RequestHead::new(method, uri, headers);
        let agent: VerifiedAgent = match self.verifier.verify(head).await {
            Ok(Some(requester)) => requester.into(),
            Ok(None) => {
                return Err(Refusal::new(
                    StatusCode::UNAUTHORIZED,
                    "signature_required",
                    "Only signed agents may use this door. Sign each request with Web Bot Auth (RFC 9421), cover @authority, @method, @path and content-digest, and name your key directory in Signature-Agent. Nothing was done.",
                ));
            }
            Err(e) => {
                return Err(Refusal::new(
                    StatusCode::UNAUTHORIZED,
                    "signature_invalid",
                    format!(
                        "Your request signature didn't verify against your published keys: {e}. Nothing was done."
                    ),
                ));
            }
        };
        if !self.any_signed && !self.trusted.contains(&agent.origin) {
            return Err(Refusal::new(
                StatusCode::FORBIDDEN,
                "agent_not_trusted",
                format!(
                    "{} isn't a counterparty of this company. Nothing was done.",
                    agent.origin
                ),
            ));
        }
        let (nonce, expires) = signature_nonce(headers).ok_or_else(|| {
            Refusal::new(
                StatusCode::UNAUTHORIZED,
                "nonce_required",
                "Sign each request with a fresh nonce. Nothing was done.",
            )
        })?;
        match self.store.fresh_nonce(&agent.origin, &nonce, expires).await {
            Ok(true) => Ok(agent),
            Ok(false) => Err(Refusal::new(
                StatusCode::UNAUTHORIZED,
                "replayed_request",
                "This signed request was already received; each one is used once. Nothing was done.",
            )),
            Err(e) => Err(Refusal::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("The door couldn't record the request ({e}). Nothing was done."),
            )),
        }
    }
}

/// The nonce and expiry of the request's Web Bot Auth signature.
fn signature_nonce(headers: &HeaderMap) -> Option<(String, i64)> {
    let field = headers
        .get_all(adk_auth::web_bot_auth::SIGNATURE_INPUT)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect::<Vec<_>>()
        .join(", ");
    let inputs = SignatureInput::parse_field(&field).ok()?;
    inputs
        .iter()
        .find(|(_, input)| input.tag() == Some(adk_auth::web_bot_auth::WEB_BOT_AUTH_TAG))
        .and_then(|(_, input)| Some((input.nonce()?.to_owned(), input.expires()?)))
}

/// `Content-Digest: sha-256=:…:` must be the SHA-256 of the body, so the signature that
/// covers the field covers the body.
fn check_content_digest(headers: &HeaderMap, body: &[u8]) -> Result<(), Refusal> {
    let missing = || {
        Refusal::new(
            StatusCode::UNAUTHORIZED,
            "content_digest_required",
            "Send Content-Digest: sha-256=:<base64>: for the body (RFC 9530) and cover it in your signature. Nothing was done.",
        )
    };
    let field = headers
        .get(CONTENT_DIGEST)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(missing)?;
    let offered = field
        .split(',')
        .filter_map(|member| member.trim().split_once('='))
        .find(|(name, _)| name.trim() == "sha-256")
        .map(|(_, value)| value.trim().trim_matches(':').to_owned())
        .ok_or_else(missing)?;
    if offered != STANDARD.encode(Sha256::digest(body)) {
        return Err(Refusal::new(
            StatusCode::UNAUTHORIZED,
            "content_digest_mismatch",
            "The body doesn't match its Content-Digest. Nothing was done.",
        ));
    }
    Ok(())
}

/// The `Content-Digest` value for `body`.
pub fn content_digest(body: &[u8]) -> String {
    format!("sha-256=:{}:", STANDARD.encode(Sha256::digest(body)))
}

/// One company's door: its keys, its handler, the signature check and the A2A task store.
pub struct Door {
    name: String,
    keys: CompanyKeys,
    auth: DoorAuth,
    handler: Arc<dyn DoorHandler>,
    executor: Arc<V1Executor>,
    events: Option<EventSender>,
    tokens: std::sync::OnceLock<(Arc<dyn AgentTokens>, String)>,
}

impl std::fmt::Debug for Door {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Door")
            .field("name", &self.name)
            .field("origin", &self.keys.origin)
            .finish_non_exhaustive()
    }
}

impl Door {
    /// A door for the company `name` with `keys`, answering through `handler`.
    pub fn new(
        name: impl Into<String>,
        keys: CompanyKeys,
        auth: DoorAuth,
        handler: Arc<dyn DoorHandler>,
        events: Option<EventSender>,
    ) -> Arc<Self> {
        let store: Arc<dyn TaskStore> = Arc::new(InMemoryTaskStore::new());
        Arc::new(Self {
            name: name.into(),
            keys,
            auth,
            handler,
            executor: Arc::new(V1Executor::new(store)),
            events,
            tokens: std::sync::OnceLock::new(),
        })
    }

    /// Also checks agent tokens (Entra Agent ID, AgentCore, Google, or another site's), issued
    /// for `audience` (this door's origin), when a signed request carries one in
    /// `Authorization: Bearer`. A token that checks names the requester's principal to the
    /// handler and in receipts; one that doesn't is refused with a sealed receipt. The Web Bot
    /// Auth signature is still required: money moves only on keys the door can check. Set once;
    /// `false` if already set.
    pub fn accept_tokens(&self, tokens: Arc<dyn AgentTokens>, audience: impl Into<String>) -> bool {
        self.tokens.set((tokens, audience.into())).is_ok()
    }

    /// The token check, after the signature: the requester with its principal, or the refusal.
    async fn check_token(
        &self,
        mut from: VerifiedAgent,
        headers: &HeaderMap,
    ) -> Result<VerifiedAgent, Refusal> {
        let (Some(token), Some((tokens, audience))) = (bearer(headers), self.tokens.get()) else {
            return Ok(from);
        };
        match tokens.principal(token, audience).await {
            Ok(principal) => {
                tracing::info!(door = %self.name, agent = %principal.principal.id, vouched_by = %principal.vouched_by(), "settle: agent token checked");
                from.principal = Some(principal);
                Ok(from)
            }
            Err(e) => Err(Refusal::new(
                StatusCode::UNAUTHORIZED,
                "token_refused",
                format!("Your agent token was refused: {e}. Nothing was done."),
            )),
        }
    }

    /// Seals a refusal receipt with this company's agent key, keeps it, and tells the
    /// company's kernel. `None` only if the company has no agent key (it always has one).
    pub async fn refusal_receipt(
        &self,
        refusal: &Refusal,
        requester: Option<&VerifiedAgent>,
        headers: Option<&HeaderMap>,
        message: Option<&str>,
        body: Option<&[u8]>,
    ) -> Option<Sealed> {
        seal_refusal(
            &self.name,
            &self.keys,
            &self.auth.store,
            self.events.as_ref(),
            refusal,
            requester,
            headers,
            message,
            body,
        )
        .await
    }

    /// The routes: the key directory, the A2A agent card and `POST /a2a`. Mount at the root
    /// of the company's door host (`pay.agentrix.website`, `kilele.money.agentrix.website`).
    pub fn router(self: &Arc<Self>) -> axum::Router {
        let card = build_v1_agent_card(
            &format!("{} agent door", self.name),
            &format!(
                "{}'s agents settle invoices with counterparties here: invoice offers, settlement under AP2-style mandates, and instalment plans. Every request must be signed with Web Bot Auth (RFC 9421) by a key in the sender's published directory.",
                self.name
            ),
            &format!("{}{A2A_PATH}", self.keys.origin),
            env!("CARGO_PKG_VERSION"),
            vec![AgentSkill {
                id: "settle".into(),
                name: "Settle invoices".into(),
                description: "Invoice offers, settlement under AP2-style mandates on PayPal, and instalment plans.".into(),
                tags: vec!["payments".into(), "paypal".into(), "ap2-style".into()],
                examples: None,
                input_modes: Some(vec![MESSAGE_MEDIA_TYPE.into()]),
                output_modes: Some(vec![MESSAGE_MEDIA_TYPE.into()]),
                security_requirements: None,
            }],
            AgentCapabilities::default(),
        );
        let card = serde_json::to_value(card).unwrap_or(Value::Null);
        let a2a = axum::Router::new()
            .route(A2A_PATH, post(serve_a2a))
            .with_state(Arc::clone(self));
        self.keys
            .directory_router()
            .route(
                AGENT_CARD_PATH,
                get(move || {
                    let card = card.clone();
                    async move { Json(card) }
                }),
            )
            .merge(a2a)
    }

    async fn send_message(
        &self,
        from: &VerifiedAgent,
        params: &Value,
        body: &[u8],
    ) -> Result<Task, Refusal> {
        let message: Message = serde_json::from_value(
            params.get("message").cloned().unwrap_or(Value::Null),
        )
        .map_err(|e| {
            Refusal::new(
                StatusCode::BAD_REQUEST,
                "bad_message",
                format!("That isn't an A2A message: {e}. Nothing was done."),
            )
        })?;
        let settle = message
            .parts
            .iter()
            .find_map(|p| match &p.content {
                PartContent::Data(data) => Some(data.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                Refusal::new(
                    StatusCode::BAD_REQUEST,
                    "bad_message",
                    "Send the settlement message as a data part. Nothing was done.",
                )
            })
            .and_then(|data| {
                serde_json::from_value::<SettleMessage>(data).map_err(|e| {
                    Refusal::new(
                        StatusCode::BAD_REQUEST,
                        "bad_message",
                        format!("That isn't a settlement message: {e}. Nothing was done."),
                    )
                })
            })?;
        let internal = |e: adk_server::a2a::v1::error::A2aError| {
            Refusal::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("The door couldn't record the task: {e}"),
            )
        };
        let task_id = uuid::Uuid::now_v7().to_string();
        let context_id = message
            .context_id
            .as_ref()
            .map(|c| c.0.clone())
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        let kind = settle.kind();
        self.executor
            .create_task_for(&task_id, &context_id, Some(&from.id))
            .await
            .map_err(internal)?;
        self.executor
            .task_store()
            .add_history_message(&task_id, message.clone())
            .await
            .map_err(internal)?;
        tracing::info!(door = %self.name, from = %from.id, kind, task = %task_id, "settle: message");
        match self.handler.handle(from, settle).await {
            Ok(answer) => {
                self.executor
                    .transition_state(&task_id, &context_id, TaskState::Working, None)
                    .await
                    .map_err(internal)?;
                let data = serde_json::to_value(&answer).map_err(|e| {
                    Refusal::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
                })?;
                let artifact = Artifact {
                    id: ArtifactId::new(format!("{kind}-answer")),
                    name: Some(answer_name(&answer).into()),
                    description: None,
                    parts: vec![Part::data(data)],
                    extensions: None,
                    metadata: None,
                };
                self.executor
                    .record_artifact(&task_id, &context_id, artifact)
                    .await
                    .map_err(internal)?;
                self.executor
                    .transition_state(&task_id, &context_id, TaskState::Completed, None)
                    .await
                    .map_err(internal)?;
            }
            Err(refusal) => {
                tracing::info!(door = %self.name, from = %from.id, kind, code = refusal.code, "settle: refused");
                let state = if refusal.status == StatusCode::OK {
                    TaskState::Rejected
                } else {
                    TaskState::Failed
                };
                if let Some(sealed) = self
                    .refusal_receipt(&refusal, Some(from), None, Some(kind), Some(body))
                    .await
                {
                    let artifact = Artifact {
                        id: ArtifactId::new(format!("{kind}-refusal")),
                        name: Some(REFUSAL_ARTIFACT.into()),
                        description: None,
                        parts: vec![Part::data(
                            serde_json::to_value(&sealed).unwrap_or(Value::Null),
                        )],
                        extensions: None,
                        metadata: None,
                    };
                    self.executor
                        .record_artifact(&task_id, &context_id, artifact)
                        .await
                        .map_err(internal)?;
                }
                self.executor
                    .transition_state(
                        &task_id,
                        &context_id,
                        state,
                        Some(format!("[{}] {}", refusal.code, refusal.message)),
                    )
                    .await
                    .map_err(internal)?;
            }
        }
        let entry = self
            .executor
            .task_store()
            .get_task(&task_id)
            .await
            .map_err(internal)?;
        internal_task_to_wire(&entry).map_err(internal)
    }

    async fn get_task(&self, from: &VerifiedAgent, params: &Value) -> Result<Task, Refusal> {
        let id = params["id"].as_str().unwrap_or_default();
        let not_found = || {
            Refusal::new(
                StatusCode::NOT_FOUND,
                "task_not_found",
                format!("No task {id} of yours is here."),
            )
        };
        let entry = self
            .executor
            .task_store()
            .get_task(id)
            .await
            .map_err(|_| not_found())?;
        if entry.owner() != Some(from.id.as_str()) {
            return Err(not_found());
        }
        internal_task_to_wire(&entry)
            .map_err(|e| Refusal::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))
    }
}

fn answer_name(answer: &DoorAnswer) -> &'static str {
    match answer {
        DoorAnswer::InvoiceRecorded { .. } => "invoice_recorded",
        DoorAnswer::Settled { .. } => "settlement_receipt",
        DoorAnswer::InstalmentOffer { .. } => "instalment_offer",
        DoorAnswer::InstalmentAgreed { .. } => "instalment_agreement",
        DoorAnswer::ReceiptPosted { .. } => "receipt_posted",
        DoorAnswer::InstalmentAccepted { .. } => "instalment_accepted",
    }
}

async fn serve_a2a(State(door): State<Arc<Door>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY_BYTES).await else {
        return Refusal::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_too_large",
            "That request is too large. Nothing was done.",
        )
        .into_response();
    };
    // Nested under a prefix, the router sees a shortened path; the agent signed the full one.
    let uri = parts
        .extensions
        .get::<OriginalUri>()
        .map(|u| u.0.clone())
        .unwrap_or_else(|| parts.uri.clone());
    let from = match door
        .auth
        .verify(&parts.method, &uri, &parts.headers, &bytes)
        .await
    {
        Ok(agent) => agent,
        Err(refusal) => {
            tracing::info!(door = %door.name, code = refusal.code, "settle: request refused at the door");
            let receipt = door
                .refusal_receipt(&refusal, None, Some(&parts.headers), None, Some(&bytes))
                .await;
            return refusal.into_response_with(receipt);
        }
    };
    let from = match door.check_token(from.clone(), &parts.headers).await {
        Ok(from) => from,
        Err(refusal) => {
            tracing::info!(door = %door.name, code = refusal.code, "settle: agent token refused at the door");
            let receipt = door
                .refusal_receipt(&refusal, Some(&from), None, None, Some(&bytes))
                .await;
            return refusal.into_response_with(receipt);
        }
    };
    let rpc: Value = match serde_json::from_slice(&bytes) {
        Ok(rpc) => rpc,
        Err(e) => {
            return rpc_error(Value::Null, -32700, &format!("Parse error: {e}"), "parse");
        }
    };
    let id = rpc.get("id").cloned().unwrap_or(Value::Null);
    let params = rpc.get("params").cloned().unwrap_or(Value::Null);
    let result = match rpc["method"].as_str() {
        Some("SendMessage") => door.send_message(&from, &params, &bytes).await,
        Some("GetTask") => door.get_task(&from, &params).await,
        other => {
            return rpc_error(
                id,
                -32601,
                &format!("Method not found: {}", other.unwrap_or("(none)")),
                "method_not_found",
            );
        }
    };
    match result {
        Ok(task) => {
            Json(json!({"jsonrpc": "2.0", "id": id, "result": {"task": task}})).into_response()
        }
        Err(refusal) => rpc_error(id, -32001, &refusal.message, refusal.code),
    }
}

/// Seals a refusal receipt for `company` with its agent key, keeps it in `store`, and tells the
/// company's kernel through `events`.
#[allow(clippy::too_many_arguments)]
async fn seal_refusal(
    company: &str,
    keys: &CompanyKeys,
    store: &Store,
    events: Option<&EventSender>,
    refusal: &Refusal,
    requester: Option<&VerifiedAgent>,
    headers: Option<&HeaderMap>,
    message: Option<&str>,
    body: Option<&[u8]>,
) -> Option<Sealed> {
    let claimed_agent = match requester {
        Some(_) => None,
        None => headers
            .and_then(|h| h.get(adk_auth::web_bot_auth::SIGNATURE_AGENT))
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
    };
    let receipt = RefusalReceipt {
        refusal_id: format!("rr_{}", uuid::Uuid::now_v7().simple()),
        refused_by: keys.origin.clone(),
        company: company.to_owned(),
        requester: requester.map(|r| r.id.clone()),
        claimed_agent,
        message: message.map(str::to_owned),
        code: refusal.code.to_owned(),
        reason: refusal.message.clone(),
        request_digest: body.map(|b| format!("sha256:{}", hex::encode(Sha256::digest(b)))),
        // A PayPal answer we couldn't read is the one refusal where money may have moved.
        nothing_moved: refusal.code != "paypal",
        at: chrono::Utc::now(),
        principal: requester.and_then(|r| r.principal.clone()),
    };
    let key = keys.agents.values().next()?;
    let sealed = Sealed::seal(SealKind::RefusalReceipt, &receipt, &keys.origin, key).ok()?;
    if let Ok(json) = serde_json::to_string(&sealed) {
        if let Err(e) = store
            .put_refusal(&receipt.refusal_id, &receipt.code, &json)
            .await
        {
            tracing::warn!(error = %e, "settle: refusal receipt not kept");
        }
    }
    if let Some(events) = events {
        let _ = events.send(SettleEvent::Refused { receipt });
    }
    Some(sealed)
}

/// Web Bot Auth in front of any route: a sample site's shop door (MCP), a checkout API. A
/// request passes only when it is signed (RFC 9421, covering `@method`, `@path` and
/// `content-digest`) by a key in the directory its `Signature-Agent` names, with a matching
/// `Content-Digest` and a fresh nonce; the handler then finds the [`VerifiedAgent`] in the
/// request's extensions. Anything else is refused before the handler runs, with a JSON-RPC
/// error carrying a refusal receipt sealed by the company's agent key, kept and sent to its
/// kernel like the settlement door's.
///
/// ```ignore
/// let gate = SignatureGate::new("Kilele Bakery", keys, DoorAuth::any_signed(directories, store), Some(events));
/// let shop = shop_router.layer(axum::middleware::from_fn_with_state(gate, SignatureGate::require));
/// // in a handler: Extension(agent): Extension<VerifiedAgent>
/// ```
pub struct SignatureGate {
    name: String,
    keys: CompanyKeys,
    auth: DoorAuth,
    events: Option<EventSender>,
}

impl std::fmt::Debug for SignatureGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignatureGate")
            .field("name", &self.name)
            .field("origin", &self.keys.origin)
            .finish_non_exhaustive()
    }
}

impl SignatureGate {
    /// A gate for the company `name`, sealing refusals with `keys`, checking with `auth`.
    pub fn new(
        name: impl Into<String>,
        keys: CompanyKeys,
        auth: DoorAuth,
        events: Option<EventSender>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            keys,
            auth,
            events,
        })
    }

    /// The middleware, for `axum::middleware::from_fn_with_state(gate, SignatureGate::require)`.
    pub async fn require(
        State(gate): State<Arc<SignatureGate>>,
        request: Request,
        next: axum::middleware::Next,
    ) -> Response {
        let (parts, body) = request.into_parts();
        let Ok(bytes) = to_bytes(body, MAX_BODY_BYTES).await else {
            return Refusal::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_too_large",
                "That request is too large. Nothing was done.",
            )
            .into_response();
        };
        let uri = parts
            .extensions
            .get::<OriginalUri>()
            .map(|u| u.0.clone())
            .unwrap_or_else(|| parts.uri.clone());
        match gate
            .auth
            .verify(&parts.method, &uri, &parts.headers, &bytes)
            .await
        {
            Ok(agent) => {
                tracing::debug!(company = %gate.name, agent = %agent.id, "settle: signed agent let through");
                let mut request = Request::from_parts(parts, axum::body::Body::from(bytes));
                request.extensions_mut().insert(agent);
                next.run(request).await
            }
            Err(refusal) => {
                tracing::info!(company = %gate.name, code = refusal.code, path = %uri.path(), "settle: unsigned or unverified agent refused");
                let receipt = seal_refusal(
                    &gate.name,
                    &gate.keys,
                    &gate.auth.store,
                    gate.events.as_ref(),
                    &refusal,
                    None,
                    Some(&parts.headers),
                    None,
                    Some(&bytes),
                )
                .await;
                let id = serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .and_then(|v| v.get("id").cloned())
                    .unwrap_or(Value::Null);
                (
                    refusal.status,
                    Json(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32001, "message": refusal.message, "data": {"code": refusal.code, "receipt": receipt}},
                    })),
                )
                    .into_response()
            }
        }
    }
}

fn rpc_error(id: Value, code: i64, message: &str, data: &str) -> Response {
    Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message, "data": {"code": data}},
    }))
    .into_response()
}

/// What a door answered.
#[derive(Debug, Clone, PartialEq)]
pub struct DoorReply {
    /// The A2A task id.
    pub task_id: String,
    /// The task's final state.
    pub state: TaskState,
    /// The answer, when completed.
    pub answer: Option<DoorAnswer>,
    /// The refusal in plain words, when rejected or failed.
    pub refusal: Option<String>,
    /// The door's sealed refusal receipt, when rejected or failed.
    pub refusal_receipt: Option<Sealed>,
}

/// What went wrong reaching another company's door.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The door refused the request itself (signature, nonce, trust) or the JSON-RPC call.
    #[error("the door refused ({status} {code}): {message}")]
    Refused {
        /// HTTP status.
        status: u16,
        /// A short code.
        code: String,
        /// The door's words.
        message: String,
        /// The door's sealed refusal receipt.
        receipt: Option<Box<Sealed>>,
    },
    /// Anything else.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<reqwest::Error> for ClientError {
    fn from(e: reqwest::Error) -> Self {
        Self::Other(e.into())
    }
}

/// One agent talking to another company's door, signing every request with its own key.
pub struct DoorClient {
    http: reqwest::Client,
    door: url::Url,
    origin: String,
    signer: HttpSignatureSigner,
    token: std::sync::RwLock<Option<String>>,
}

impl std::fmt::Debug for DoorClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DoorClient")
            .field("door", &self.door.as_str())
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl DoorClient {
    /// An agent whose key directory is at `origin`, signing with `key`, talking to the door at
    /// `door` (its origin; `/a2a` is appended), over `http`.
    pub fn new(
        http: reqwest::Client,
        door: url::Url,
        key: Arc<Ed25519SigningKey>,
        origin: &str,
    ) -> anyhow::Result<Self> {
        let origin = origin.trim_end_matches('/').to_owned();
        let signer = HttpSignatureSigner::builder(key, origin.clone())
            .components(vec![
                ComponentId::new("@authority"),
                ComponentId::new("@method"),
                ComponentId::new("@path"),
                ComponentId::new("@query"),
                ComponentId::new(CONTENT_DIGEST),
            ])
            .build()?;
        Ok(Self {
            http,
            door,
            origin,
            signer,
            token: std::sync::RwLock::new(None),
        })
    }

    /// Presents `token` (an agent token from the site's issuer, or a cloud's) in
    /// `Authorization: Bearer` on every call from now on, beside the signature; `None` stops.
    pub fn set_token(&self, token: Option<String>) {
        *self.token.write().unwrap_or_else(|e| e.into_inner()) = token;
    }

    /// The identity the door sees: `wba:{origin}#{keyid}`.
    pub fn id(&self) -> String {
        format!("wba:{}#{}", self.origin, self.signer.keyid())
    }

    /// Sends a JSON-RPC call, signed.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::now_v7().to_string(),
            "method": method,
            "params": params,
        }))
        .map_err(anyhow::Error::from)?;
        let url = self
            .door
            .join(A2A_PATH.trim_start_matches('/'))
            .map_err(anyhow::Error::from)?;
        let mut request = self
            .http
            .post(url)
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(CONTENT_DIGEST, content_digest(&body));
        if let Some(token) = self
            .token
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_deref()
        {
            request = request.bearer_auth(token);
        }
        let mut request = request.body(body).build()?;
        self.signer
            .sign_reqwest(&mut request)
            .map_err(anyhow::Error::from)?;
        let response = self.http.execute(request).await?;
        let status = response.status().as_u16();
        let value: Value = response.json().await.unwrap_or(Value::Null);
        if let Some(error) = value.get("error") {
            return Err(ClientError::Refused {
                status,
                code: error["data"]["code"].as_str().unwrap_or("error").to_owned(),
                message: error["message"].as_str().unwrap_or("no message").to_owned(),
                receipt: serde_json::from_value(error["data"]["receipt"].clone())
                    .ok()
                    .map(Box::new),
            });
        }
        Ok(value["result"].clone())
    }

    /// Sends one settlement message and reads the door's answer.
    pub async fn send(&self, message: &SettleMessage) -> Result<DoorReply, ClientError> {
        let mut part = Part::data(serde_json::to_value(message).map_err(anyhow::Error::from)?);
        part.media_type = Some(MESSAGE_MEDIA_TYPE.into());
        let message = Message {
            id: MessageId::new(uuid::Uuid::now_v7().to_string()),
            role: MessageRole::User,
            parts: vec![part],
            task_id: None,
            context_id: None,
            reference_task_ids: None,
            extensions: None,
            metadata: None,
        };
        let result = self
            .call("SendMessage", json!({ "message": message }))
            .await?;
        let task: Task = serde_json::from_value(result["task"].clone())
            .map_err(|e| anyhow::anyhow!("the door's answer isn't an A2A task: {e}"))?;
        Ok(reply_of(&task))
    }

    /// Reads one of this agent's tasks back.
    pub async fn task(&self, task_id: &str) -> Result<DoorReply, ClientError> {
        let result = self.call("GetTask", json!({ "id": task_id })).await?;
        let task: Task = serde_json::from_value(result["task"].clone())
            .map_err(|e| anyhow::anyhow!("the door's answer isn't an A2A task: {e}"))?;
        Ok(reply_of(&task))
    }
}

/// Reads a task's answer or refusal.
pub fn reply_of(task: &Task) -> DoorReply {
    let data = |refusal: bool| {
        task.artifacts
            .iter()
            .flatten()
            .filter(move |a| (a.name.as_deref() == Some(REFUSAL_ARTIFACT)) == refusal)
            .flat_map(|a| a.parts.iter())
            .filter_map(|p| match &p.content {
                PartContent::Data(data) => Some(data.clone()),
                _ => None,
            })
    };
    let answer = data(false).find_map(|d| serde_json::from_value::<DoorAnswer>(d).ok());
    let refusal_receipt = data(true).find_map(|d| serde_json::from_value::<Sealed>(d).ok());
    let refusal = task
        .status
        .message
        .as_ref()
        .and_then(|m| m.text().map(str::to_owned));
    DoorReply {
        task_id: task.id.0.clone(),
        state: task.status.state,
        answer,
        refusal,
        refusal_receipt,
    }
}

impl From<Refusal> for ClientError {
    fn from(r: Refusal) -> Self {
        Self::Refused {
            status: r.status.as_u16(),
            code: r.code.into(),
            message: r.message,
            receipt: None,
        }
    }
}
