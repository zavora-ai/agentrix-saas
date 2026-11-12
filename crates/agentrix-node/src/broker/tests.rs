use std::collections::HashMap;
use std::sync::Arc;

use agentrix_types::{ImageRef, JobKind, ReceiptKind, Signer, SiteId, Subject, UiEvent};
use bytes::Bytes;

use super::fake::{FakePage, FakeRunner, RoomRun, fetch_room};
use super::fetch::{FetchArgs, FetchRequest, parse_doh_txt, parse_note};
use super::memory::{MemoryBlobs, MemoryReceipts};
use super::ui::UiMapper;
use super::*;

fn image(name: &str) -> ImageRef {
    ImageRef::parse(&format!("{name}@{}", Digest::of(name.as_bytes()))).unwrap()
}

fn images() -> Images {
    Images {
        fetch: Some(image("room-fetch")),
        replay: Some(image("room-replay")),
        render: Some(image("room-render")),
        build: Some(image("room-build")),
    }
}

struct Rig {
    broker: Broker,
    runner: FakeRunner,
    receipts: MemoryReceipts,
    blobs: MemoryBlobs,
    signer: Arc<Signer>,
}

fn rig(room: fake::Room) -> Rig {
    let signer = Arc::new(Signer::from_secret_bytes(&[3; 32]));
    let runner = FakeRunner::new(signer.verifying_key(), room);
    let receipts = MemoryReceipts::new(signer.clone());
    let blobs = MemoryBlobs::default();
    let broker = Broker::new(
        Arc::new(runner.clone()),
        signer.clone(),
        Arc::new(receipts.clone()),
        Arc::new(blobs.clone()),
        images(),
    );
    Rig {
        broker,
        runner,
        receipts,
        blobs,
        signer,
    }
}

fn site() -> SiteId {
    SiteId::new("site_test")
}

#[tokio::test]
async fn a_build_job_uploads_inputs_checks_artifacts_and_signs_a_receipt() {
    let room: fake::Room = Arc::new(|spec, inputs| {
        assert_eq!(inputs.len(), 1, "the input was uploaded before the job");
        let mut out = std::collections::BTreeMap::new();
        out.insert("site/index.html".to_string(), b"<h1>built</h1>".to_vec());
        assert_eq!(spec.args["pages"], 3);
        RoomRun {
            events: vec![JobEvent::Note {
                text: "built 3 pages".into(),
            }],
            out,
            ..RoomRun::default()
        }
    });
    let r = rig(room);
    let mut req = JobRequest::new(
        site(),
        JobKind::Build,
        serde_json::json!({"pages": 3}),
        "Built 3 pages",
    );
    req.inputs = vec![Bytes::from_static(b"inventory")];
    req.parent = Parent::Move(MoveId::new("move_1"));
    let mut seen = Vec::new();
    let out = r.broker.run(req, &mut |ev| seen.push(ev)).await.unwrap();

    assert!(out.clean_exit());
    let built = out.artifact("site/index.html").unwrap();
    assert_eq!(built.digest, Digest::of(b"<h1>built</h1>"));
    assert!(matches!(seen[0], BrokerEvent::Admitted { .. }));
    assert!(
        seen.iter()
            .any(|e| matches!(e, BrokerEvent::Job(JobEvent::Note { .. })))
    );

    let receipt = r.receipts.get(&out.receipt).unwrap();
    let body = receipt.verify(&r.signer.verifying_key()).unwrap();
    assert_eq!(body.kind, ReceiptKind::JobBuild);
    assert_eq!(body.subject, Subject::Job(out.spec.job_id.clone()));
    assert_eq!(body.inputs, vec![Digest::of(b"inventory")]);
    assert_eq!(body.outputs, vec![built.digest]);
    assert_eq!(body.destroyed, Some(true));
    assert_eq!(body.title(), Some("Built 3 pages"));
    assert_eq!(body.detail.as_ref().unwrap()["move"], "move_1");
    assert_eq!(body.policy.as_ref().unwrap().runtime, "fake");
    assert!(r.blobs.kind_of(&site(), &built.digest).is_some());
}

#[tokio::test]
async fn a_spec_signed_by_another_key_is_refused() {
    let r = rig(Arc::new(|_, _| RoomRun::default()));
    let other = Arc::new(Signer::from_secret_bytes(&[9; 32]));
    let broker = Broker::new(
        Arc::new(r.runner.clone()),
        other.clone(),
        Arc::new(MemoryReceipts::new(other)),
        Arc::new(MemoryBlobs::default()),
        images(),
    );
    let req = JobRequest::new(site(), JobKind::Render, serde_json::json!({}), "x");
    let err = broker.run(req, &mut |_| {}).await.unwrap_err();
    assert!(
        matches!(err, BrokerError::Refused { status: 403, .. }),
        "{err}"
    );
}

#[tokio::test]
async fn an_artifact_that_does_not_match_its_digest_is_rejected() {
    // A runner that serves different bytes than it reports.
    struct Liar(FakeRunner);
    #[async_trait]
    impl RunnerApi for Liar {
        async fn put_blob(&self, d: &Digest, b: Bytes) -> Result<(), BrokerError> {
            self.0.put_blob(d, b).await
        }
        async fn post_job(&self, s: &SignedJobSpec) -> Result<EventStream, BrokerError> {
            self.0.post_job(s).await
        }
        async fn get_artifact(&self, _: &Digest) -> Result<Bytes, BrokerError> {
            Ok(Bytes::from_static(b"swapped"))
        }
    }
    let room: fake::Room = Arc::new(|_, _| {
        let mut out = std::collections::BTreeMap::new();
        out.insert("a".to_string(), b"original".to_vec());
        RoomRun {
            out,
            ..RoomRun::default()
        }
    });
    let r = rig(room);
    let broker = Broker::new(
        Arc::new(Liar(r.runner.clone())),
        r.signer.clone(),
        Arc::new(r.receipts.clone()),
        Arc::new(r.blobs.clone()),
        images(),
    );
    let req = JobRequest::new(site(), JobKind::Build, serde_json::json!({}), "x");
    let err = broker.run(req, &mut |_| {}).await.unwrap_err();
    assert!(matches!(err, BrokerError::Artifact(_)), "{err}");
    assert!(
        r.receipts.all().is_empty(),
        "no receipt for output that didn't check out"
    );
}

#[tokio::test]
async fn no_image_means_no_job() {
    let r = rig(Arc::new(|_, _| RoomRun::default()));
    let broker = Broker::new(
        Arc::new(r.runner.clone()),
        r.signer.clone(),
        Arc::new(r.receipts.clone()),
        Arc::new(r.blobs.clone()),
        Images::default(),
    );
    let req = JobRequest::new(site(), JobKind::Fetch, serde_json::json!({}), "x");
    assert_eq!(
        broker.run(req, &mut |_| {}).await.unwrap_err(),
        BrokerError::NoImage(JobKind::Fetch)
    );
}

#[tokio::test]
async fn an_unreachable_runner_says_so_in_plain_words() {
    let r = rig(Arc::new(|_, _| RoomRun::default()));
    r.runner.set_down(true);
    let req = JobRequest::new(site(), JobKind::Render, serde_json::json!({}), "x");
    let err = r.broker.run(req, &mut |_| {}).await.unwrap_err();
    assert_eq!(err.plain(), "Our sealed rooms aren't reachable right now.");
}

fn web() -> HashMap<String, FakePage> {
    let mut web = HashMap::new();
    web.insert(
        "https://kilele.example/".to_string(),
        FakePage::html("<html><body><a href='/about'>About</a></body></html>"),
    );
    web.insert(
        "https://kilele.example/old".to_string(),
        FakePage::redirect("https://evil.example/"),
    );
    web
}

#[tokio::test]
async fn a_fetch_reads_the_manifest_maps_ticks_pages_and_frames() {
    let r = rig(fetch_room(web()));
    let urls = vec![
        "https://kilele.example/".to_string(),
        "https://kilele.example/missing".to_string(),
        "https://kilele.example/old".to_string(),
    ];
    let mut mapper = UiMapper::new();
    mapper.fetch_job(
        &urls
            .iter()
            .enumerate()
            .map(|(i, u)| (u.clone(), Some(i as u32 + 1)))
            .collect::<Vec<_>>(),
        3,
    );
    let mut ui = Vec::new();
    let out = r
        .broker
        .fetch(
            FetchRequest {
                site: site(),
                allowlist: vec!["kilele.example".into()],
                args: FetchArgs::new(urls.clone(), fetch::DEFAULT_USER_AGENT),
                parent: Parent::Assessment(AssessmentId::new("asmt_1")),
                title: "Read 3 pages of kilele.example in a sealed room".into(),
            },
            &mut |ev| ui.extend(mapper.on(&ev, Utc::now())),
        )
        .await
        .unwrap();

    assert_eq!(out.problem, None);
    assert_eq!(out.pages.len(), 3);
    assert_eq!(out.pages[0].seen.status, Some(200));
    assert!(out.pages[0].text().contains("About"));
    assert_eq!(out.pages[1].seen.status, Some(404));
    assert!(
        out.pages[2]
            .seen
            .error
            .as_deref()
            .unwrap()
            .contains("refused")
    );

    let pages: Vec<_> = ui
        .iter()
        .filter(|e| matches!(e, UiEvent::Page(_)))
        .collect();
    assert_eq!(pages.len(), 3);
    let frames: Vec<_> = ui
        .iter()
        .filter_map(|e| match e {
            UiEvent::Frame(f) => Some(f),
            _ => None,
        })
        .collect();
    assert!(!frames.is_empty());
    assert_eq!(frames[0].page_url, "https://kilele.example/");
    assert_eq!(frames[0].src, format!("/frames/{}", frames[0].digest));
    assert_eq!((frames[0].w, frames[0].h), (1280, 800));
    assert_eq!(
        r.blobs.kind_of(&site(), &frames[0].digest).unwrap(),
        ("frame".to_string(), "image/png".to_string())
    );
    let last_tick = ui
        .iter()
        .rev()
        .find_map(|e| match e {
            UiEvent::ReceiptTick(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        last_tick.refusals, 1,
        "the redirect off the allowlist was refused"
    );
    assert_eq!(last_tick.hosts, 1);
    assert!(last_tick.requests >= 3);

    let receipt = r.receipts.get(&out.job.receipt).unwrap();
    let observed = receipt.body.observed.as_ref().unwrap();
    assert_eq!(observed.hosts, vec!["kilele.example".to_string()]);
    assert_eq!(observed.refusals.len(), 1);
    assert_eq!(
        receipt.body.detail.as_ref().unwrap()["assessment"],
        "asmt_1"
    );
    assert_eq!(receipt.body.kind, ReceiptKind::JobFetch);
    let spec = &r.runner.jobs()[0];
    assert_eq!(spec.allowlist, vec!["kilele.example".to_string()]);
    assert_eq!(spec.args["user_agent"], fetch::DEFAULT_USER_AGENT);
}

#[tokio::test]
async fn claim_probes_run_from_the_sandbox() {
    let mut web = HashMap::new();
    web.insert(
        "https://dns.google/resolve?name=_agentrix.kilele.example&type=TXT".to_string(),
        FakePage::text(
            r#"{"Status":0,"Answer":[{"name":"_agentrix.kilele.example.","type":16,"data":"\"agentrix-site-verification=\" \"abc123\""}]}"#,
        ),
    );
    web.insert(
        "https://kilele.example/.well-known/agentrix-verify.txt".to_string(),
        FakePage::text("agentrix-site-verification=abc123\n"),
    );
    let r = rig(fetch_room(web));
    let txt = r
        .broker
        .probe_txt(&site(), "_agentrix.kilele.example")
        .await
        .unwrap();
    assert_eq!(txt, vec!["agentrix-site-verification=abc123".to_string()]);
    let url =
        reqwest::Url::parse("https://kilele.example/.well-known/agentrix-verify.txt").unwrap();
    let (status, body) = r.broker.probe_file(&site(), &url).await.unwrap();
    assert_eq!(status, 200);
    assert!(body.contains("abc123"));
    let missing = reqwest::Url::parse("https://kilele.example/nope.txt").unwrap();
    assert_eq!(r.broker.probe_file(&site(), &missing).await.unwrap().0, 404);
    let jobs = r.runner.jobs();
    assert_eq!(jobs[0].allowlist, vec!["dns.google".to_string()]);
    assert_eq!(jobs[1].allowlist, vec!["kilele.example".to_string()]);
    assert_eq!(
        r.receipts.all().len(),
        3,
        "each probe is a job with a receipt"
    );
    assert!(r.broker.probe_txt(&site(), "bad name; rm").await.is_err());
}

#[tokio::test]
async fn the_proxys_own_403_is_a_refusal_not_the_sites_answer() {
    let mut web = HashMap::new();
    web.insert(
        "http://kilele.example/plain".to_string(),
        FakePage {
            status: 403,
            content_type: "text/plain".into(),
            body: b"agentrix-egress refused 169.254.169.254:80: metadata address\n".to_vec(),
            ms: 1,
            location: None,
            rendered: None,
        },
    );
    web.insert(
        "http://kilele.example/real".to_string(),
        FakePage::status(403),
    );
    let r = rig(fetch_room(web));
    let urls = vec![
        "http://kilele.example/plain".to_string(),
        "http://kilele.example/real".to_string(),
    ];
    let out = r
        .broker
        .fetch(
            FetchRequest {
                site: site(),
                allowlist: vec!["kilele.example".into()],
                args: FetchArgs::new(urls, fetch::DEFAULT_USER_AGENT),
                parent: Parent::None,
                title: "x".into(),
            },
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_eq!(out.pages[0].seen.status, None);
    assert_eq!(
        out.pages[0].seen.error.as_deref(),
        Some("refused by the egress proxy (169.254.169.254:80: metadata address)")
    );
    assert!(out.pages[0].body.is_none());
    assert_eq!(
        out.pages[1].seen.status,
        Some(403),
        "a site's own 403 stays a 403"
    );
}

#[test]
fn notes_and_doh_answers_parse() {
    assert_eq!(parse_note("fetched 3 200").unwrap().status, Some(200));
    assert_eq!(parse_note("fetched 4 error").unwrap().status, None);
    assert!(parse_note("built 3 pages").is_none());
    assert_eq!(
        parse_doh_txt(r#"{"Status":3}"#).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        parse_doh_txt(r#"{"Status":0}"#).unwrap(),
        Vec::<String>::new()
    );
    assert!(parse_doh_txt(r#"{"Status":2}"#).is_err());
}

#[test]
fn image_headers_are_read_and_other_formats_refused() {
    assert_eq!(image_info(fake::TINY_PNG), Some(("image/png", 1280, 800)));
    let jpeg = [
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x02, 0x58,
        0x03, 0x20, 0x03,
    ];
    assert_eq!(image_info(&jpeg), Some(("image/jpeg", 800, 600)));
    assert_eq!(image_info(b"<svg onload=alert(1)>"), None);
}
