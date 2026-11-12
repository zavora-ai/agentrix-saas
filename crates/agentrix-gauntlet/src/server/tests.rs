//! The two routes over an in-memory backend: both need the token; a posted run is rescored from
//! the backend's own receipts, and only verified receipts of the posting site count.

use std::sync::Mutex;

use agentrix_types::{Digest, KeySet, Receipt, ReceiptKind, Signer, SiteId};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt as _;

use super::*;
use crate::PostedAttack;
use crate::tests::{full_set, job_body, signed};

const TOKEN: &str = "Bearer test-token-not-a-secret";

struct Mem {
    receipts: Vec<Receipt>,
    keys: KeySet,
    recorded: Mutex<Vec<(String, u32)>>,
}

#[async_trait]
impl RunBackend for Mem {
    async fn authorized(&self, headers: &HeaderMap, _site: &str) -> bool {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            == Some(TOKEN)
    }

    fn keys(&self) -> KeySet {
        self.keys.clone()
    }

    async fn receipt(&self, digest: &Digest) -> anyhow::Result<Option<Receipt>> {
        Ok(self
            .receipts
            .iter()
            .find(|r| r.digest().ok().as_ref() == Some(digest))
            .cloned())
    }

    async fn list(
        &self,
        site: &str,
        _since: Option<&str>,
        kind: Option<&str>,
        limit: u32,
    ) -> anyhow::Result<Vec<Listed>> {
        Ok(self
            .receipts
            .iter()
            .filter(|r| r.body.site.as_str() == site)
            .filter(|r| kind.is_none_or(|k| r.body.kind.as_str() == k))
            .take(limit as usize)
            .map(|r| Listed {
                digest: r.digest().unwrap(),
                kind: r.body.kind.as_str().to_owned(),
                ended_at: r.body.ended_at.to_rfc3339(),
            })
            .collect())
    }

    async fn record(&self, site: &str, _run: &PostedRun, score: &Score) -> anyhow::Result<Digest> {
        self.recorded
            .lock()
            .unwrap()
            .push((site.to_owned(), score.score));
        Ok(Digest::of(b"gauntlet.run"))
    }
}

fn backend(signer: &Signer, receipts: Vec<Receipt>) -> Arc<Mem> {
    Arc::new(Mem {
        receipts,
        keys: KeySet {
            keys: vec![signer.published_key()],
        },
        recorded: Mutex::new(Vec::new()),
    })
}

fn posted(site: &str, cited: &[Digest]) -> PostedRun {
    PostedRun {
        site: site.to_owned(),
        started_at: "2026-11-08T08:00:00Z".into(),
        ended_at: "2026-11-08T08:05:00Z".into(),
        runner: Some("test".into()),
        // Every receipt cited for every attack: the node decides which proves what.
        attacks: (1..=10)
            .map(|id| PostedAttack {
                id,
                receipts: cited.to_vec(),
            })
            .collect(),
    }
}

async fn send(b: Arc<Mem>, req: Request<Body>) -> (StatusCode, Value) {
    let resp = router(b).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn post(run: &PostedRun, auth: Option<&str>) -> Request<Body> {
    let mut req = Request::post("/gauntlet/runs").header(header::CONTENT_TYPE, "application/json");
    if let Some(a) = auth {
        req = req.header(header::AUTHORIZATION, a);
    }
    req.body(Body::from(serde_json::to_vec(run).unwrap()))
        .unwrap()
}

#[tokio::test]
async fn both_routes_need_the_token() {
    let signer = Signer::generate();
    let b = backend(&signer, Vec::new());
    let (status, _) = send(b.clone(), post(&posted("st_fixture", &[]), None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(
        b.clone(),
        post(&posted("st_fixture", &[]), Some("Bearer wrong")),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let req = Request::get("/receipts?site=st_fixture")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(b.clone(), req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(b.recorded.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_posted_run_is_rescored_from_the_nodes_own_receipts() {
    let signer = Signer::generate();
    let set = full_set(&signer);
    let cited: Vec<Digest> = set.iter().map(|v| v.digest).collect();
    let b = backend(&signer, set.into_iter().map(|v| v.receipt).collect());
    let (status, body) = send(b.clone(), post(&posted("st_fixture", &cited), Some(TOKEN))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["score"], 10);
    assert_eq!(body["out_of"], 10);
    assert_eq!(body["receipt"], json!(Digest::of(b"gauntlet.run")));
    assert_eq!(
        b.recorded.lock().unwrap().as_slice(),
        &[("st_fixture".to_owned(), 10)]
    );
}

#[tokio::test]
async fn unknown_forged_or_other_site_receipts_do_not_count() {
    let ours = Signer::generate();
    let theirs = Signer::generate();
    let other_site = {
        let mut body = job_body(ReceiptKind::JobFetch, &["169.254.169.254"], 0);
        body.site = SiteId::new("st_someone_else");
        Receipt::sign(body, &ours).unwrap()
    };
    let forged =
        Receipt::sign(job_body(ReceiptKind::JobFetch, &["127.0.0.1"], 0), &theirs).unwrap();
    let missing = signed(
        &ours,
        job_body(ReceiptKind::JobReplay, &["api.wp-update-cdn.test"], 0),
    );
    let cited = vec![
        other_site.digest().unwrap(),
        forged.digest().unwrap(),
        missing.digest,
    ];
    let b = backend(&ours, vec![other_site, forged]);
    let (status, body) = send(b.clone(), post(&posted("st_fixture", &cited), Some(TOKEN))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["score"], 0, "{body}");
}

#[tokio::test]
async fn the_listing_returns_the_sites_receipts_and_refuses_a_bad_since() {
    let signer = Signer::generate();
    let set = full_set(&signer);
    let n = set.len();
    let b = backend(&signer, set.into_iter().map(|v| v.receipt).collect());
    let req = Request::get("/receipts?site=st_fixture&since=2026-11-07T00:00:00Z")
        .header(header::AUTHORIZATION, TOKEN)
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(b.clone(), req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), n);
    assert!(body[0]["digest"].as_str().unwrap().starts_with("sha256:"));

    let req = Request::get("/receipts?site=st_fixture&since=yesterday';--")
        .header(header::AUTHORIZATION, TOKEN)
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(b, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
