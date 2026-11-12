//! Test kit: a real signed site (adk-site's log, an Ed25519 release signer, the manifest in
//! CAS), a scripted model and a receipts sink, shared by the site and door tests.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use adk_core::audit_log::InMemoryAuditLogStore;
use adk_site::log::{LOG_PREFIX, SiteLog};
use adk_site::release::{file_digest, manifest_digest};
use adk_site::signer::{Ed25519ReleaseSigner, ReleaseSigner};
use async_trait::async_trait;
use awp_types::{FileEntry, GeneratorRef, LogEntry, ReleaseManifest};
use bytes::Bytes;
use futures::stream::{self, BoxStream, StreamExt};

use super::{AskModel, LivePointer, ReceiptDraft, ReceiptSink, ReleaseSource, SiteState};

pub(crate) const DOMAIN: &str = "kilele-bakery.example";

/// Releases, blobs and log tiles in memory.
#[derive(Default)]
pub(crate) struct Memory {
    pub(crate) pointers: Mutex<HashMap<String, LivePointer>>,
    pub(crate) blobs: Mutex<HashMap<String, Bytes>>,
    pub(crate) tiles: Mutex<HashMap<String, Bytes>>,
}

#[async_trait]
impl ReleaseSource for Memory {
    async fn live(&self, host: &str) -> anyhow::Result<Option<LivePointer>> {
        Ok(self.pointers.lock().unwrap().get(host).cloned())
    }
    async fn blob(&self, digest: &str) -> anyhow::Result<Option<Bytes>> {
        Ok(self.blobs.lock().unwrap().get(digest).cloned())
    }
    async fn log_file(&self, _site: &str, path: &str) -> anyhow::Result<Option<Bytes>> {
        Ok(self.tiles.lock().unwrap().get(path).cloned())
    }
}

/// Screens everything as safe, has no embeddings, and answers from a script.
pub(crate) struct Scripted;

#[async_trait]
impl AskModel for Scripted {
    async fn screen(&self, _site: &str, text: &str) -> anyhow::Result<bool> {
        Ok(!text.contains("UNSAFE"))
    }
    async fn embed(&self, _site: &str, _texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        anyhow::bail!("no embeddings in tests")
    }
    async fn answer(
        &self,
        _site: &str,
        _system: &str,
        user: &str,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>> {
        let reply = if user.contains("Question: Do you deliver") {
            vec![
                "Yes. Saturday deliveries ",
                "to Kilimani run 10:00-12:00 ",
                "and cost KSh 300 [1].",
            ]
        } else if user.contains("Question: Is there an uncited") {
            vec!["We are open every day."]
        } else {
            vec!["DONT_KNOW"]
        };
        Ok(stream::iter(reply.into_iter().map(|s| Ok(s.to_owned()))).boxed())
    }
}

#[derive(Default)]
pub(crate) struct Receipts(pub(crate) Mutex<Vec<ReceiptDraft>>);

#[async_trait]
impl ReceiptSink for Receipts {
    async fn write(&self, draft: ReceiptDraft) -> anyhow::Result<String> {
        let mut all = self.0.lock().unwrap();
        all.push(draft);
        Ok(format!("r-{}", all.len()))
    }
}

pub(crate) const INDEX_HTML: &str = r#"<!doctype html><html><head><title>Kilele Bakery</title></head><body><main><h1>Kilele Bakery</h1><p>Sourdough and cakes, baked at dawn in Kilimani.</p></main></body></html>"#;
pub(crate) const DELIVERY_HTML: &str = r#"<!doctype html><html><head><title>Delivery</title></head><body><main><h1>Delivery</h1><h2 id="kilimani">Kilimani</h2><p>Saturday deliveries to Kilimani run 10:00-12:00 and cost KSh 300.</p></main></body></html>"#;
pub(crate) const DELIVERY_MD: &str = "# Delivery\n\n## Kilimani\n\nSaturday deliveries to Kilimani run 10:00-12:00 and cost KSh 300.\n";
pub(crate) const CONTACT_HTML: &str = r#"<!doctype html><html><head><title>Contact</title></head><body><main><h1>Contact</h1><p>Write to Wanjiru.</p></main></body></html>"#;

pub(crate) struct Site {
    pub(crate) memory: Arc<Memory>,
    log: SiteLog,
    signer: Arc<Ed25519ReleaseSigner>,
}

fn file(
    memory: &Memory,
    files: &mut BTreeMap<String, FileEntry>,
    path: &str,
    body: &str,
    kind: &str,
) {
    let bytes = Bytes::from(body.to_owned());
    let digest = file_digest(&bytes);
    memory
        .blobs
        .lock()
        .unwrap()
        .insert(digest.as_str().to_owned(), bytes.clone());
    files.insert(
        path.into(),
        FileEntry {
            sha256: digest,
            size: bytes.len() as u64,
            content_type: kind.into(),
        },
    );
}

impl Site {
    pub(crate) async fn new() -> Self {
        let signer = Arc::new(Ed25519ReleaseSigner::from_seed(DOMAIN, [7; 32]).unwrap());
        let log = SiteLog::open(
            Arc::new(InMemoryAuditLogStore::new()),
            DOMAIN,
            signer.clone(),
        )
        .await
        .unwrap();
        Self {
            memory: Arc::new(Memory::default()),
            log,
            signer,
        }
    }

    /// Stages, logs and points `host` at release `seq`, whose index page says `headline`.
    pub(crate) async fn release(&self, seq: u64, headline: &str) -> LivePointer {
        let mut files = BTreeMap::new();
        let index = INDEX_HTML.replace("Sourdough and cakes", headline);
        file(
            &self.memory,
            &mut files,
            "index.html",
            &index,
            "text/html; charset=utf-8",
        );
        file(
            &self.memory,
            &mut files,
            "delivery/index.html",
            DELIVERY_HTML,
            "text/html; charset=utf-8",
        );
        file(
            &self.memory,
            &mut files,
            "delivery/index.md",
            DELIVERY_MD,
            "text/markdown; charset=utf-8",
        );
        file(
            &self.memory,
            &mut files,
            "contact/index.html",
            CONTACT_HTML,
            "text/html; charset=utf-8",
        );
        let manifest = ReleaseManifest {
            seq,
            previous: None,
            base: None,
            created_at: chrono::Utc::now(),
            files,
            removed: BTreeSet::new(),
            redirects: vec![],
            source: awp_types::ReleaseSource::Migration,
            generator: GeneratorRef {
                name: "test".into(),
                version: "1".into(),
            },
            charter_version: 1,
        };
        let (digest, canonical) = manifest_digest(&manifest).unwrap();
        self.memory
            .blobs
            .lock()
            .unwrap()
            .insert(digest.as_str().to_owned(), canonical);
        let before = self.log.size().await;
        self.log
            .append(&LogEntry {
                seq,
                manifest: digest.clone(),
            })
            .await
            .unwrap();
        let publication = self.log.publication(before).await.unwrap();
        let mut checkpoint = String::new();
        let prefix = format!("{LOG_PREFIX}/");
        for (path, bytes) in publication.files() {
            if let Some(rel) = path.strip_prefix(prefix.as_str()) {
                if rel == "checkpoint" {
                    checkpoint = String::from_utf8(bytes.to_vec()).unwrap();
                } else {
                    self.memory
                        .tiles
                        .lock()
                        .unwrap()
                        .insert(rel.to_owned(), bytes.clone());
                }
            }
        }
        let pointer = LivePointer {
            site_id: "site-kilele".into(),
            domain: DOMAIN.into(),
            seq,
            manifest_digest: digest.as_str().to_owned(),
            checkpoint,
            release_keys: vec![self.signer.verifier_key()],
        };
        self.memory
            .pointers
            .lock()
            .unwrap()
            .insert(DOMAIN.into(), pointer.clone());
        pointer
    }

    pub(crate) fn state(&self) -> (SiteState, Arc<Receipts>) {
        let receipts = Arc::new(Receipts::default());
        (
            SiteState::new(self.memory.clone(), Arc::new(Scripted), receipts.clone()),
            receipts,
        )
    }
}
