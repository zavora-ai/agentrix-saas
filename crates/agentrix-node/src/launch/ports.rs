//! The Mover's ports over the node's own stores and clients, so `app.rs` only constructs them
//! ([`mover`]):
//!
//! | Port (`agentrix_move::ports`) | Here | Over |
//! |---|---|---|
//! | `Blobs` | [`NodeBlobs`] | `db::cas` with its per-site index |
//! | `Receipts` | [`NodeReceipts`] | `db::receipts::ReceiptStore` |
//! | `Events` | [`MoveEvents`] | `ui_events` (stream `move:<id>`) and a broadcast for SSE |
//! | `Rooms` | [`BrokerRooms`] | `broker::Broker::run`, parent `Move` |
//! | `Gateway` | `agentrix_move::gateway::HttpGateway` | the Model Gateway on localhost |

use std::sync::Arc;

use agentrix_move::gateway::HttpGateway;
use agentrix_move::ports::{Blobs, Events, Receipts, RoomArtifact, RoomJob, RoomRun, Rooms};
use agentrix_move::{MoveStore, Mover};
use agentrix_types::{Digest, JobKind, MoveId, ReceiptBody, SiteId, UiEnvelope, UiEvent};
use anyhow::Context as _;
use async_trait::async_trait;
use bytes::Bytes;
use sqlx::SqlitePool;
use tokio::sync::broadcast;

use crate::broker::{Broker, BrokerError, JobRequest, Parent};
use crate::db::cas::Cas;
use crate::db::receipts::ReceiptStore;

/// The CAS with its per-site index.
#[derive(Clone)]
pub struct NodeBlobs {
    /// The control database (the `artifacts` index).
    pub pool: SqlitePool,
    /// The content store.
    pub cas: Arc<Cas>,
}

#[async_trait]
impl Blobs for NodeBlobs {
    async fn put(
        &self,
        site: &SiteId,
        kind: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> anyhow::Result<Digest> {
        let digest = self.cas.put(bytes).await?;
        self.cas
            .record(
                &self.pool,
                site,
                &digest,
                kind,
                content_type,
                bytes.len() as u64,
            )
            .await?;
        Ok(digest)
    }

    async fn get(&self, site: &SiteId, digest: &Digest) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self
            .cas
            .get_for_site(&self.pool, site, digest)
            .await?
            .map(|(bytes, _)| bytes))
    }
}

/// The receipts store.
#[derive(Clone)]
pub struct NodeReceipts(pub Arc<ReceiptStore>);

#[async_trait]
impl Receipts for NodeReceipts {
    async fn issue(&self, body: ReceiptBody) -> anyhow::Result<Digest> {
        Ok(self.0.issue(body).await?.digest)
    }
}

/// A Move's events: persisted in `ui_events` (stream `move:<id>`, `seq` monotonic per Move)
/// for `Last-Event-ID` replay, broadcast to live SSE subscribers (design §4.3a), and turned
/// into the Mover's presence (W1's `presence::global().on_move`).
#[derive(Clone)]
pub struct MoveEvents {
    pool: SqlitePool,
    live: broadcast::Sender<(MoveId, UiEnvelope)>,
}

impl MoveEvents {
    /// Events in `pool`.
    pub fn new(pool: SqlitePool) -> Self {
        let (live, _) = broadcast::channel(1024);
        Self { pool, live }
    }

    /// Live events of every Move; filter by id.
    pub fn subscribe(&self) -> broadcast::Receiver<(MoveId, UiEnvelope)> {
        self.live.subscribe()
    }

    /// A Move's stored events after `after` (a `Last-Event-ID`), in order.
    pub async fn replay(&self, move_id: &MoveId, after: u64) -> anyhow::Result<Vec<UiEnvelope>> {
        let rows: Vec<(i64, String, String)> = sqlx::query_as(
            "SELECT seq, name, data FROM ui_events WHERE stream = ? AND seq > ? ORDER BY seq",
        )
        .bind(format!("move:{move_id}"))
        .bind(i64::try_from(after).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(seq, name, data)| {
                let event: UiEvent = serde_json::from_value(serde_json::json!({
                    "event": name,
                    "data": serde_json::from_str::<serde_json::Value>(&data)?,
                }))?;
                Ok(UiEnvelope {
                    seq: u64::try_from(seq)?,
                    event,
                })
            })
            .collect()
    }
}

#[async_trait]
impl Events for MoveEvents {
    async fn emit(&self, site: &SiteId, move_id: &MoveId, event: UiEvent) -> anyhow::Result<()> {
        let (seq,): (i64,) = sqlx::query_as(
            "INSERT INTO ui_events (stream, seq, site_id, name, data, at) \
             VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM ui_events WHERE stream = ?1), ?2, ?3, ?4, ?5) \
             RETURNING seq",
        )
        .bind(format!("move:{move_id}"))
        .bind(site.as_str())
        .bind(event.name())
        .bind(event.data_json()?)
        .bind(agentrix_move::store::time(chrono::Utc::now()))
        .fetch_one(&self.pool)
        .await?;
        // The Mover's living mark follows its steps (W1, handover-wiring 2.1).
        agentrix_kernel::presence::global().on_move(site.as_str(), &event);
        let _ = self.live.send((
            move_id.clone(),
            UiEnvelope {
                seq: u64::try_from(seq)?,
                event,
            },
        ));
        Ok(())
    }
}

/// Sealed rooms through the broker client (design §4.2): inputs read from the CAS, artifacts
/// checked and stored by the broker, the job's receipt written by it.
#[derive(Clone)]
pub struct BrokerRooms {
    /// The broker.
    pub broker: Broker,
    /// Where the Move's inputs are.
    pub blobs: NodeBlobs,
}

#[async_trait]
impl Rooms for BrokerRooms {
    fn supports(&self, kind: JobKind) -> bool {
        self.broker.can_run(kind)
    }

    async fn run(&self, site: &SiteId, move_id: &MoveId, job: RoomJob) -> anyhow::Result<RoomRun> {
        let mut inputs = Vec::with_capacity(job.inputs.len());
        for digest in &job.inputs {
            let bytes = self
                .blobs
                .get(site, digest)
                .await?
                .with_context(|| format!("the room's input {digest} is missing"))?;
            inputs.push(Bytes::from(bytes));
        }
        let title = match job.kind {
            JobKind::Replay => "Replayed the old site in a sealed room with no route out",
            JobKind::Build => "Built the new site in a sealed room with no network",
            JobKind::Render => "Rendered the new pages in a sealed room with no network",
            JobKind::Fetch => "Read public pages in a sealed room",
        };
        let mut request = JobRequest::new(site.clone(), job.kind, job.args.clone(), title);
        request.inputs = inputs;
        request.parent = Parent::Move(move_id.clone());
        request.artifact_kind = "move-artifact";
        // A transport error before the runner answered (a stale pooled connection after the
        // runner sat idle) is tried once more; every failure is logged with its detail, since
        // the owner only sees the plain line.
        let mut tries = 0;
        let outcome = loop {
            tries += 1;
            match self.broker.run(request.clone(), &mut |_| {}).await {
                Ok(outcome) => break outcome,
                Err(e @ BrokerError::Unreachable(_)) if tries == 1 => {
                    tracing::warn!(%move_id, kind = ?job.kind, error = %e, "the runner did not answer; trying the room once more");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => {
                    tracing::warn!(%move_id, kind = ?job.kind, error = %e, "the room did not run");
                    return Err(anyhow::anyhow!("{}", e.plain()));
                }
            }
        };
        Ok(RoomRun {
            artifacts: outcome
                .artifacts
                .iter()
                .map(|a| RoomArtifact {
                    name: a.name.clone(),
                    digest: a.digest,
                })
                .collect(),
            result: outcome.result,
            receipt: outcome.receipt,
        })
    }
}

/// The Mover over the node's stores, the broker and the Model Gateway at `models_url`.
pub fn mover(
    pool: SqlitePool,
    cas: Arc<Cas>,
    receipts: Arc<ReceiptStore>,
    events: Arc<MoveEvents>,
    broker: Broker,
    models_url: &str,
    http: reqwest::Client,
) -> Mover {
    let blobs = NodeBlobs {
        pool: pool.clone(),
        cas,
    };
    Mover {
        store: MoveStore::new(pool),
        blobs: Arc::new(blobs.clone()),
        receipts: Arc::new(NodeReceipts(receipts)),
        events,
        rooms: Arc::new(BrokerRooms { broker, blobs }),
        gateway: Arc::new(HttpGateway::new(models_url, http)),
        policy: Default::default(),
    }
}
