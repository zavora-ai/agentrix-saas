//! The LaunchGate: go-live, the release pointer and rollback (R7.3, R7.4, R8.1; design §4.5).
//!
//! Lane: `move` (specs/first-release/design.md §3.0). Task S2-01.
//!
//! **Go-live** ([`LaunchGate::go_live`]), only after `decisions/` has verified the owner's
//! approval of the go-live card's fingerprint (release digest + charter v1):
//! 1. the card's release must still be the site's candidate, its manifest must match its
//!    digest, and charter v1 must be the one the card names;
//! 2. the release is appended to the site log and the checkpoint signed with
//!    `adk_site::signer::Ed25519ReleaseSigner` (key name `{domain}/.well-known/awp/log`);
//! 3. one transaction marks it live, supersedes the previous release and moves
//!    `site_pointer` (atomic);
//! 4. a probe loads the release exactly as the edge does (`site::release::ReleaseCache`:
//!    checkpoint, inclusion proof, manifest digest, file digests). If it fails, the pointer
//!    goes straight back and the owner is told; later probes run in the background and, on a
//!    failure, the Guardian rolls back (its charter v1 level, "Undo only");
//! 5. a `release.signed` receipt links the decision's fingerprint to the manifest digest.
//!
//! Once the first probe passes, [`after_live::AfterLive`] runs in the background with the new
//! release's pages: billing's go-live day and pictures for pages that had none (handover-wiring
//! 1.3). Go-live never waits on it.
//!
//! **Rollback** ([`LaunchGate::roll_back`]): one atomic pointer switch to the previous signed
//! release, or off the platform back to the old site, for 30 days, with a `release.rollback`
//! receipt.
//!
//! **Serving**: [`LaunchReleases`] is the edge's `site::release::ReleaseSource` over
//! `site_pointer`, `releases`, the CAS and the site log.
//!
//! The site log is rebuilt from the signed releases in `releases` (deterministic: the same
//! entries and key give the same tree and the same signed checkpoint), so it needs no tables
//! of its own. Each site's release key is derived from the node's release master key.

pub mod after_live;
pub mod ports;
#[cfg(test)]
pub(crate) mod tests;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use adk_core::audit_log::InMemoryAuditLogStore;
use adk_site::log::{LOG_PREFIX, SiteLog};
use adk_site::signer::{Ed25519ReleaseSigner, ReleaseSigner};
use agentrix_move::candidate::{load_manifest, to_awp};
use agentrix_move::golive::GoLiveArgs;
use agentrix_move::ports::Events;
use agentrix_move::store::{ReleaseRecord, ReleaseState};
use agentrix_move::{MoveState, MoveStore};
use agentrix_types::ui::{DoneEvent, DoneOutcome, StepEvent, StepName, StepState};
use agentrix_types::{DecisionCard, Digest, ReceiptBody, ReceiptKind, SiteId, Subject, UiEvent};
use anyhow::{Context as _, bail, ensure};
use async_trait::async_trait;
use awp_types::LogEntry;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use sha2::{Digest as _, Sha256};
use tokio::sync::Mutex;

use crate::db::cas::Cas;
use crate::db::receipts::ReceiptStore;
use crate::site::release::{LivePointer, ReleaseCache, ReleaseSource};

/// How long the owner can undo a go-live (R7.4).
pub const UNDO_DAYS: i64 = 30;

/// Each site's release key, derived from the node's release master key: the master never
/// signs anything itself, and a site's key is the same after a restart.
#[derive(Clone)]
pub struct ReleaseKeys {
    master: [u8; 32],
}

impl std::fmt::Debug for ReleaseKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReleaseKeys(..)")
    }
}

impl ReleaseKeys {
    /// Keys under `master` (32 random bytes kept in the node's key directory).
    pub fn new(master: [u8; 32]) -> Self {
        Self { master }
    }

    /// The release signer for `domain`.
    pub fn signer(&self, domain: &str) -> anyhow::Result<Ed25519ReleaseSigner> {
        let mut hash = Sha256::new();
        hash.update(b"agentrix/release-key/v1\0");
        hash.update(self.master);
        hash.update(domain.to_ascii_lowercase().as_bytes());
        let seed: [u8; 32] = hash.finalize().into();
        Ed25519ReleaseSigner::from_seed(&domain.to_ascii_lowercase(), seed)
            .map_err(|e| anyhow::anyhow!("release key for {domain}: {e}"))
    }

    /// The verifier key the site publishes, `<name>+<hash>+<base64 key>`.
    pub fn verifier_key(&self, domain: &str) -> anyhow::Result<String> {
        Ok(self.signer(domain)?.verifier_key())
    }
}

/// One site's log, rebuilt: its published files and its latest checkpoint.
#[derive(Debug, Clone, Default)]
pub struct LogFiles {
    /// Releases in the log.
    pub size: u64,
    /// Tiles and entry bundles, by path under the log prefix.
    pub tiles: HashMap<String, Bytes>,
    /// The signed checkpoint note.
    pub checkpoint: String,
}

/// Site logs rebuilt from the `releases` table, cached by site until the next go-live.
#[derive(Debug)]
pub struct SiteLogs {
    keys: ReleaseKeys,
    cache: Mutex<HashMap<String, Arc<LogFiles>>>,
}

impl SiteLogs {
    /// Logs signed with `keys`.
    pub fn new(keys: ReleaseKeys) -> Self {
        Self {
            keys,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The release keys.
    pub fn keys(&self) -> &ReleaseKeys {
        &self.keys
    }

    async fn open(&self, domain: &str, entries: &[(u64, Digest)]) -> anyhow::Result<SiteLog> {
        let signer: Arc<dyn ReleaseSigner> = Arc::new(self.keys.signer(domain)?);
        let log = SiteLog::open(
            Arc::new(InMemoryAuditLogStore::new()),
            &domain.to_ascii_lowercase(),
            signer,
        )
        .await
        .map_err(|e| anyhow::anyhow!("opening the site log: {e}"))?;
        for (seq, digest) in entries {
            log.append(&LogEntry {
                seq: *seq,
                manifest: to_awp(digest),
            })
            .await
            .map_err(|e| anyhow::anyhow!("rebuilding the site log at release {seq}: {e}"))?;
        }
        Ok(log)
    }

    async fn files_of(log: &SiteLog) -> anyhow::Result<LogFiles> {
        let publication = log
            .publication(0)
            .await
            .map_err(|e| anyhow::anyhow!("site log: {e}"))?;
        let prefix = format!("{LOG_PREFIX}/");
        let mut files = LogFiles {
            size: publication.size,
            ..Default::default()
        };
        for (path, bytes) in publication.files() {
            let Some(rel) = path.strip_prefix(prefix.as_str()) else {
                continue;
            };
            if rel == "checkpoint" {
                files.checkpoint = String::from_utf8(bytes.to_vec())?;
            } else {
                files.tiles.insert(rel.to_owned(), bytes.clone());
            }
        }
        Ok(files)
    }

    /// The site's log files, rebuilt from its signed releases when the cache is stale.
    pub async fn files(
        &self,
        store: &MoveStore,
        site: &SiteId,
        domain: &str,
    ) -> anyhow::Result<Arc<LogFiles>> {
        let signed = store.signed_releases(site).await?;
        if let Some(cached) = self.cache.lock().await.get(site.as_str())
            && cached.size == signed.len() as u64
        {
            return Ok(cached.clone());
        }
        let entries: Vec<(u64, Digest)> =
            signed.iter().map(|r| (r.seq, r.manifest_digest)).collect();
        let log = self.open(domain, &entries).await?;
        let files = Arc::new(if entries.is_empty() {
            LogFiles::default()
        } else {
            Self::files_of(&log).await?
        });
        if let Some(stored) = signed.last().and_then(|r| r.checkpoint.as_deref())
            && stored != files.checkpoint
        {
            tracing::error!(site = %site, "the rebuilt site log does not match the stored checkpoint");
            bail!("the site log does not match its signed checkpoint; releases are paused");
        }
        self.cache
            .lock()
            .await
            .insert(site.as_str().to_owned(), files.clone());
        Ok(files)
    }

    /// Appends release `seq` and signs the new checkpoint. Returns the log's files.
    pub async fn append(
        &self,
        store: &MoveStore,
        site: &SiteId,
        domain: &str,
        seq: u64,
        manifest: &Digest,
    ) -> anyhow::Result<LogFiles> {
        let signed = store.signed_releases(site).await?;
        let entries: Vec<(u64, Digest)> =
            signed.iter().map(|r| (r.seq, r.manifest_digest)).collect();
        let log = self.open(domain, &entries).await?;
        ensure!(
            log.size().await + 1 == seq,
            "release {seq} is not next in the site log (it holds {})",
            log.size().await
        );
        log.append(&LogEntry {
            seq,
            manifest: to_awp(manifest),
        })
        .await
        .map_err(|e| anyhow::anyhow!("signing the site log: {e}"))?;
        let files = Self::files_of(&log).await?;
        self.cache.lock().await.remove(site.as_str());
        Ok(files)
    }
}

/// The edge's view of releases (`site::release::ReleaseSource`) over the move lane's tables.
#[derive(Clone)]
pub struct LaunchReleases {
    store: MoveStore,
    cas: Arc<Cas>,
    logs: Arc<SiteLogs>,
    platform: Option<String>,
}

pub use agentrix_move::golive::{platform_host_from_env, public_address, site_alias};

impl LaunchReleases {
    /// Releases from `store`, blobs from `cas`, logs from `logs`; platform addresses under
    /// the host in `AGENTRIX_ORIGIN`.
    pub fn new(store: MoveStore, cas: Arc<Cas>, logs: Arc<SiteLogs>) -> Self {
        Self {
            store,
            cas,
            logs,
            platform: platform_host_from_env(),
        }
    }

    /// Serves platform addresses under `platform` (or none).
    pub fn with_platform_host(mut self, platform: Option<&str>) -> Self {
        self.platform = platform.map(str::to_ascii_lowercase);
        self
    }

    /// The site a request host names: its own domain, or its platform address. Only labels
    /// directly under the platform host count, so on-demand TLS is never asked for others.
    async fn site_for(
        &self,
        host: &str,
    ) -> anyhow::Result<Option<agentrix_move::store::SiteRecord>> {
        if let Some(site) = self.store.site_by_host(host).await? {
            return Ok(Some(site));
        }
        let Some(platform) = &self.platform else {
            return Ok(None);
        };
        let Some(label) = host.strip_suffix(&format!(".{platform}")) else {
            return Ok(None);
        };
        if label.is_empty() || label.contains('.') {
            return Ok(None);
        }
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT site_id FROM sites WHERE replace(lower(domain), '.', '-') = ? LIMIT 1",
        )
        .bind(label)
        .fetch_optional(self.store.pool())
        .await?;
        match row {
            Some((id,)) => self.store.site(&SiteId::new(id)).await,
            None => Ok(None),
        }
    }
}

#[async_trait]
impl ReleaseSource for LaunchReleases {
    async fn live(&self, host: &str) -> anyhow::Result<Option<LivePointer>> {
        let Some(site) = self.site_for(host).await? else {
            return Ok(None);
        };
        let Some((seq, manifest)) = self.store.pointer(&site.id).await? else {
            return Ok(None);
        };
        let Some(release) = self.store.release(&site.id, seq).await? else {
            return Ok(None);
        };
        let Some(checkpoint) = release.checkpoint else {
            return Ok(None);
        };
        Ok(Some(LivePointer {
            site_id: site.id.to_string(),
            domain: site.domain.to_ascii_lowercase(),
            seq,
            manifest_digest: manifest.to_string(),
            checkpoint,
            release_keys: vec![self.logs.keys().verifier_key(&site.domain)?],
        }))
    }

    async fn blob(&self, digest: &str) -> anyhow::Result<Option<Bytes>> {
        let Ok(digest) = Digest::from_str(digest) else {
            return Ok(None);
        };
        Ok(self.cas.get(&digest).await?.map(Bytes::from))
    }

    async fn log_file(&self, site_id: &str, path: &str) -> anyhow::Result<Option<Bytes>> {
        let site = SiteId::new(site_id);
        let Some(record) = self.store.site(&site).await? else {
            return Ok(None);
        };
        let files = self.logs.files(&self.store, &site, &record.domain).await?;
        Ok(files.tiles.get(path).cloned())
    }
}

/// Where an undo goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackTo {
    /// The signed release before the live one, or the old site when there is none.
    Previous,
    /// Off the platform: the domain shows the old site, which was kept (R7.4).
    OldSite,
}

/// A go-live that happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launched {
    /// The release number.
    pub seq: u64,
    /// Its manifest digest.
    pub manifest: Digest,
    /// The `release.signed` receipt.
    pub receipt: Digest,
    /// The site's host.
    pub host: String,
    /// Time from approval to the pointer switch and a passing probe.
    pub took_ms: u64,
}

/// An undo that happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolledBack {
    /// The release now live, or `None` for the old site.
    pub to: Option<u64>,
    /// The `release.rollback` receipt.
    pub receipt: Digest,
}

/// The LaunchGate.
#[derive(Clone)]
pub struct LaunchGate {
    /// The move lane's tables.
    pub store: MoveStore,
    /// The content store.
    pub cas: Arc<Cas>,
    /// The receipts store.
    pub receipts: Arc<ReceiptStore>,
    /// The Move's live stream.
    pub events: Arc<dyn Events>,
    /// Site logs and release keys.
    pub logs: Arc<SiteLogs>,
    /// Background probes after go-live: how many, and how far apart.
    pub probes: (u32, Duration),
    /// What follows a go-live in the background (billing, pictures), when wired.
    pub after_live: Option<Arc<dyn after_live::AfterLive>>,
}

impl LaunchGate {
    /// The edge's release source over the same tables.
    pub fn releases(&self) -> LaunchReleases {
        LaunchReleases::new(self.store.clone(), self.cas.clone(), self.logs.clone())
    }

    /// Loads the live release of `host` exactly as the edge will, and reads its home page.
    pub async fn probe(&self, host: &str, seq: u64) -> anyhow::Result<()> {
        let cache = ReleaseCache::new(Arc::new(self.releases()));
        let release = cache
            .current(host)
            .await?
            .context("the edge would serve nothing for this host")?;
        ensure!(
            release.pointer.seq == seq,
            "the edge would serve release {}, not {seq}",
            release.pointer.seq
        );
        let (_, entry) = release
            .resolve("/")
            .context("the release has no home page")?;
        cache
            .file(entry)
            .await?
            .context("the home page does not match its digest")?;
        Ok(())
    }

    /// Signs and switches to the release a verified go-live approval names (R7.3).
    /// `decisions/` calls this only after the approval verified over the card's fingerprint.
    pub async fn go_live(
        &self,
        card: &DecisionCard,
        decision_receipt: &Digest,
        approver: &str,
    ) -> anyhow::Result<Launched> {
        let started = Utc::now();
        ensure!(
            card.fingerprint_matches()?,
            "this card was changed after it was raised"
        );
        let args = GoLiveArgs::of(card)?;
        let site = self
            .store
            .site(&card.site)
            .await?
            .context("the card's site is unknown")?;
        let release = self
            .store
            .release(&card.site, args.seq)
            .await?
            .context("the release on this card is no longer waiting to go live")?;
        ensure!(
            release.state == ReleaseState::Candidate && release.manifest_digest == args.release,
            "the version of your site on this card is no longer the one waiting to go live"
        );
        let (_, charter, _) = self
            .store
            .charter(&card.site, 1)
            .await?
            .context("the starting levels for your WebAgents are missing")?;
        ensure!(
            charter == args.charter,
            "the starting levels changed after this card was raised"
        );
        let manifest =
            load_manifest(&CasBlobs(self.cas.clone()), &card.site, &args.release).await?;
        for (path, entry) in &manifest.files {
            let digest = Digest::from_str(entry.sha256.as_str())?;
            ensure!(
                self.cas.contains(&digest).await,
                "the new site's file {path} is missing"
            );
        }

        // Sign: append to the site log and sign the checkpoint.
        let files = self
            .logs
            .append(
                &self.store,
                &card.site,
                &site.domain,
                args.seq,
                &args.release,
            )
            .await?;
        let previous = self.store.pointer(&card.site).await?;
        // Switch: one transaction.
        self.store
            .go_live(
                &card.site,
                args.seq,
                &files.checkpoint,
                &card.decision_id,
                Utc::now(),
            )
            .await?;
        let host = site.domain.to_ascii_lowercase();
        if let Err(e) = self.probe(&host, args.seq).await {
            tracing::error!(error = %format!("{e:#}"), %host, "go-live probe failed; switching back");
            let back = previous.map(|(seq, _)| seq);
            self.switch_back(
                &card.site,
                &host,
                back,
                "the first check of the new release failed",
                "LaunchGate",
            )
            .await?;
            bail!(
                "The new release didn't pass its first check, so we switched your site straight back. Nothing changed for visitors."
            );
        }
        // Live and serving: billing and pictures follow in the background (1.3).
        if let Some(after) = self.after_live.clone() {
            let (site_id, cas, files, at) = (
                card.site.clone(),
                self.cas.clone(),
                manifest.files.clone(),
                Utc::now(),
            );
            tokio::spawn(async move {
                let pages = after_live::release_pages(&cas, &files).await;
                after.went_live(&site_id, at, pages).await;
            });
        }
        self.store
            .sign_charter(&card.site, 1, approver, &decision_receipt.to_string())
            .await?;

        let checkpoint_digest = self.cas.put(files.checkpoint.as_bytes()).await?;
        self.cas
            .record(
                self.store.pool(),
                &card.site,
                &checkpoint_digest,
                "checkpoint",
                "text/plain",
                files.checkpoint.len() as u64,
            )
            .await?;
        let ended = Utc::now();
        let mut body = ReceiptBody::new(
            ReceiptKind::ReleaseSigned,
            card.site.clone(),
            Subject::Decision(card.decision_id.clone()),
            started,
            ended,
        );
        body.inputs = vec![args.release, *decision_receipt];
        body.outputs = vec![checkpoint_digest];
        body.detail = Some(serde_json::json!({
            "seq": args.seq,
            "domain": host,
            "decision": card.decision_id,
            "fingerprint": card.fingerprint,
            "charter": args.charter,
            "charter_version": 1,
            "release_key": self.logs.keys().verifier_key(&site.domain)?,
            "files": manifest.files.len(),
        }));
        let receipt = self
            .receipts
            .issue(body.with_title(format!("Signed release {} of {host}; it is live", args.seq)))
            .await?
            .digest;

        self.store
            .set_state(&args.move_id, MoveState::Live, ended)
            .await?;
        let summary = format!("Live: release {} of {host} is signed and served", args.seq);
        self.store
            .record_step(
                &args.move_id,
                &card.site,
                StepName::GoLive,
                StepState::Done,
                &summary,
                Some(&receipt),
                None,
                started,
                Some(ended),
            )
            .await?;
        let took_ms = u64::try_from((ended - started).num_milliseconds()).unwrap_or(0);
        let mut step = StepEvent::new(StepName::GoLive, StepState::Done, took_ms, &summary, ended);
        step.receipt = Some(receipt);
        let _ = self
            .events
            .emit(&card.site, &args.move_id, UiEvent::Step(step))
            .await;
        let _ = self
            .events
            .emit(
                &card.site,
                &args.move_id,
                UiEvent::Done(DoneEvent {
                    outcome: DoneOutcome::Live,
                    href: Some(public_address(&host, platform_host_from_env().as_deref())),
                    at: ended,
                }),
            )
            .await;
        self.spawn_probes(card.site.clone(), host.clone(), args.seq);
        Ok(Launched {
            seq: args.seq,
            manifest: args.release,
            receipt,
            host,
            took_ms,
        })
    }

    /// Background probes after go-live. A failure is undone by the Guardian, whose charter v1
    /// level is "Undo only" (ADR-012).
    fn spawn_probes(&self, site: SiteId, host: String, seq: u64) {
        let (count, every) = self.probes;
        if count == 0 {
            return;
        }
        let gate = self.clone();
        tokio::spawn(async move {
            for _ in 0..count {
                tokio::time::sleep(every).await;
                if gate
                    .store
                    .pointer(&site)
                    .await
                    .ok()
                    .flatten()
                    .map(|(s, _)| s)
                    != Some(seq)
                {
                    return;
                }
                if let Err(e) = gate.probe(&host, seq).await {
                    tracing::error!(error = %format!("{e:#}"), %host, "probe failed; the Guardian rolls back");
                    if let Err(e) = gate
                        .roll_back(&site, RollbackTo::Previous, "Guardian")
                        .await
                    {
                        tracing::error!(error = %format!("{e:#}"), %host, "the Guardian's rollback failed");
                    }
                    return;
                }
            }
        });
    }

    async fn switch_back(
        &self,
        site: &SiteId,
        host: &str,
        to: Option<u64>,
        reason: &str,
        by: &str,
    ) -> anyhow::Result<RolledBack> {
        let started = Utc::now();
        let from = self.store.pointer(site).await?;
        match to {
            Some(seq) => self.store.roll_back_to(site, seq, Utc::now()).await?,
            None => self.store.roll_back_to_old_site(site).await?,
        }
        let to_release = match to {
            Some(seq) => self.store.release(site, seq).await?,
            None => None,
        };
        let mut body = ReceiptBody::new(
            ReceiptKind::ReleaseRollback,
            site.clone(),
            Subject::Site(site.clone()),
            started,
            Utc::now(),
        );
        body.inputs = from.iter().map(|(_, d)| *d).collect();
        body.outputs = to_release.iter().map(|r| r.manifest_digest).collect();
        body.detail = Some(serde_json::json!({
            "from_seq": from.map(|(s, _)| s),
            "to_seq": to,
            "to": if to.is_some() { "previous-release" } else { "old-site" },
            "by": by,
            "reason": reason,
        }));
        let title = match to {
            Some(seq) => format!("Rolled {host} back to release {seq}"),
            None => format!("Took {host} back to your old site"),
        };
        let receipt = self.receipts.issue(body.with_title(title)).await?.digest;
        Ok(RolledBack { to, receipt })
    }

    /// Undoes the live release in one step (R7.4): back to the previous signed release, or off
    /// the platform to the old site. Allowed for 30 days after the live release was signed.
    pub async fn roll_back(
        &self,
        site: &SiteId,
        to: RollbackTo,
        by: &str,
    ) -> anyhow::Result<RolledBack> {
        let record = self.store.site(site).await?.context("unknown site")?;
        let (live_seq, _) = self
            .store
            .pointer(site)
            .await?
            .context("this site isn't live, so there is nothing to undo")?;
        let releases = self.store.releases(site).await?;
        let live = releases
            .iter()
            .find(|r| r.seq == live_seq)
            .context("the live release is missing")?;
        ensure!(
            within_undo(live, Utc::now()),
            "Undo works for {UNDO_DAYS} days after going live, and this release went live more than {UNDO_DAYS} days ago."
        );
        let target = match to {
            RollbackTo::OldSite => None,
            RollbackTo::Previous => releases
                .iter()
                .filter(|r| r.seq < live_seq && r.checkpoint.is_some())
                .map(|r| r.seq)
                .max(),
        };
        let reason = if by == "Guardian" {
            "a check of the live site failed"
        } else {
            "you asked to undo"
        };
        let done = self
            .switch_back(
                site,
                &record.domain.to_ascii_lowercase(),
                target,
                reason,
                by,
            )
            .await?;
        if let Some(decision) = &live.decision
            && let Some(card) = self.store.card(decision).await?
            && let Ok(args) = GoLiveArgs::of(&card.card)
        {
            self.store
                .set_state(&args.move_id, MoveState::RolledBack, Utc::now())
                .await?;
        }
        Ok(done)
    }
}

/// Whether the release is still inside its undo window.
pub fn within_undo(release: &ReleaseRecord, now: DateTime<Utc>) -> bool {
    release
        .signed_at
        .is_some_and(|at| now - at <= chrono::Duration::days(UNDO_DAYS))
}

/// The CAS as the Move crate's blob port, for reading manifests back.
struct CasBlobs(Arc<Cas>);

#[async_trait]
impl agentrix_move::ports::Blobs for CasBlobs {
    async fn put(
        &self,
        _site: &SiteId,
        _kind: &str,
        _ct: &str,
        bytes: &[u8],
    ) -> anyhow::Result<Digest> {
        Ok(self.0.put(bytes).await?)
    }
    async fn get(&self, _site: &SiteId, digest: &Digest) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self.0.get(digest).await?)
    }
}
