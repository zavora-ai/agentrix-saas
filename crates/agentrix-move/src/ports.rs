//! What the Move needs from the rest of the node, as traits, so each lane plugs in its own
//! implementation and tests use fakes (design §3.0, D10a).
//!
//! | Port | Implemented by | Over |
//! |---|---|---|
//! | [`Blobs`] | data (`db::cas`) | the content-addressed store |
//! | [`Receipts`] | data (`db::receipts`) | the receipts store, signed with control's key |
//! | [`Events`] | coordinator | `ui_events` and the `/moves/{id}/events` stream |
//! | [`Rooms`] | coordinator (`broker/`) | sealed rooms on the sandbox VM |
//! | [`Gateway`] | this crate ([`crate::gateway::HttpGateway`]) | the Model Gateway, by purpose |

use agentrix_types::{Digest, JobKind, JobResult, MoveId, ReceiptBody, Risk, SiteId, UiEvent};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The content-addressed store.
#[async_trait]
pub trait Blobs: Send + Sync {
    /// Stores `bytes` for `site` and records what they are (`kind`, for example `page`,
    /// `evidence`, `manifest`, `move-state`) and their media type. Returns their digest.
    async fn put(
        &self,
        site: &SiteId,
        kind: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> anyhow::Result<Digest>;

    /// The bytes named `digest`, if `site` holds them. Same shape as the broker's
    /// `BlobStore::get`, so one adapter over `db::cas` serves both.
    async fn get(&self, site: &SiteId, digest: &Digest) -> anyhow::Result<Option<Vec<u8>>>;
}

/// The receipts store: signs a body with control's key and stores it.
#[async_trait]
pub trait Receipts: Send + Sync {
    /// Signs and stores `body`; returns the receipt's digest.
    async fn issue(&self, body: ReceiptBody) -> anyhow::Result<Digest>;
}

/// The Move's live stream (design §4.3a). The implementation numbers events per Move,
/// persists them for `Last-Event-ID` replay and fans them out to SSE subscribers.
#[async_trait]
pub trait Events: Send + Sync {
    /// Emits one event on `/moves/{move_id}/events`.
    async fn emit(&self, site: &SiteId, move_id: &MoveId, event: UiEvent) -> anyhow::Result<()>;
}

/// A job for a sealed room, before the broker signs it (design §4.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomJob {
    /// What the room does. The Move uses `replay`, `build` and `render`, all with no network.
    pub kind: JobKind,
    /// Input blobs in the CAS; the broker uploads them and the room sees `/in/<digest>`.
    pub inputs: Vec<Digest>,
    /// Non-secret arguments, written to `/in/job.json`.
    pub args: Value,
}

/// One artifact a room produced, already fetched into the CAS and checked by the broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomArtifact {
    /// Its name inside the room's output, for example `shots/about.png`.
    pub name: String,
    /// Its digest in the CAS.
    pub digest: Digest,
}

/// What a room run gave back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomRun {
    /// The runner's result: artifacts, peaks, refusals, kill reason, destruction.
    pub result: JobResult,
    /// The artifacts by name.
    pub artifacts: Vec<RoomArtifact>,
    /// The `job.*` receipt the broker wrote for the run.
    pub receipt: Digest,
}

impl RoomRun {
    /// The artifact called `name`.
    pub fn artifact(&self, name: &str) -> Option<&Digest> {
        self.artifacts
            .iter()
            .find(|a| a.name == name)
            .map(|a| &a.digest)
    }

    /// Whether the room ran to a clean exit (not killed, exit code 0).
    pub fn succeeded(&self) -> bool {
        self.result.killed.is_none() && self.result.exit_code == Some(0)
    }
}

/// Sealed rooms, through the broker client.
#[async_trait]
pub trait Rooms: Send + Sync {
    /// Whether this deployment can run `kind` rooms. When it cannot, the Move takes the
    /// documented fallback and says so on the Move screen (R4.4).
    fn supports(&self, kind: JobKind) -> bool;

    /// Runs one job to completion: signs the spec, uploads the inputs, streams the job's
    /// events to the Move's stream, fetches the artifacts and writes the job's receipt.
    /// A room that was killed (caps) is a [`RoomRun`] with `result.killed` set, not an error.
    async fn run(&self, site: &SiteId, move_id: &MoveId, job: RoomJob) -> anyhow::Result<RoomRun>;
}

/// Who a model call is for, so the gateway routes, meters and budgets it (R11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallContext {
    /// The site (`x-agentrix-site`).
    pub site: SiteId,
    /// The Move, for the usage meter.
    pub move_id: MoveId,
    /// Which try this is, from 1. The gateway goes one tier up on each retry (design §4.3).
    pub attempt: u32,
}

/// A page for the builder model to write. The page's content is **data**: it is clean text
/// from the old site, passed in a field the instructions name as untrusted, never as
/// instructions (R5.3, R5.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildRequest {
    /// The site's name.
    pub site_name: String,
    /// The page's path, which the new page keeps.
    pub path: String,
    /// The page's title.
    pub title: String,
    /// The page's clean content, as Markdown.
    pub markdown: String,
    /// Links the page may carry: its own links and the site's pages.
    pub allowed_links: Vec<String>,
    /// Images the page may show, by path.
    pub allowed_images: Vec<String>,
    /// What went wrong last time, when this is a retry (R6.3).
    pub feedback: Option<String>,
    /// Page features the gateway's tier rules read: sections and forms (design §4.3).
    pub sections: u32,
}

/// The builder's page: the body of the page, as HTML, before the site theme wraps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildReply {
    /// The `<article>` content.
    pub body_html: String,
    /// The model that wrote it, as the gateway reported it, for the receipt.
    pub model: Option<String>,
}

/// The safety screen's verdict on a piece of untrusted text (R5.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screen {
    /// Whether the text is safe to keep as content.
    pub safe: bool,
    /// The screen's raw answer or reason, kept on the evidence.
    pub reason: String,
}

/// A request for a vision parity verdict on one rebuilt page (R6.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisionRequest {
    /// The page's path.
    pub path: String,
    /// A screenshot of the rebuilt page (PNG).
    pub new_png: Vec<u8>,
    /// A screenshot of the old page, when the crawl took one (PNG).
    pub old_png: Option<Vec<u8>>,
    /// The clean text the page should show.
    pub expected_text: String,
}

/// The vision model's verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisionVerdict {
    /// From 0 (nothing alike) to 1 (the same content and structure).
    pub score: f64,
    /// One plain line.
    pub verdict: String,
}

/// The Model Gateway, called by purpose only (design §4.3). No method names a model or a
/// provider.
#[async_trait]
pub trait Gateway: Send + Sync {
    /// Purpose `build`, tier `auto`: writes one page.
    async fn build_page(
        &self,
        ctx: &CallContext,
        request: &BuildRequest,
    ) -> anyhow::Result<BuildReply>;

    /// Purpose `screen`: content safety on untrusted text.
    async fn screen(&self, ctx: &CallContext, text: &str) -> anyhow::Result<Screen>;

    /// Purpose `vision-parity`: does the rebuilt page show what it should?
    async fn vision_parity(
        &self,
        ctx: &CallContext,
        request: &VisionRequest,
    ) -> anyhow::Result<VisionVerdict>;

    /// Purpose `decide`: a risk score for a card, when the gateway offers one.
    async fn decide(&self, _ctx: &CallContext, _state: &str) -> anyhow::Result<Option<Risk>> {
        Ok(None)
    }
}
