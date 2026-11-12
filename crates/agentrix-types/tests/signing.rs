//! Sign and verify round trips (S0-08 done-when), and the wire shapes other lanes code against.

use agentrix_types::ui::{FrameKind, ParityEvent, ParityState, StepEvent, StepName, StepState};
use agentrix_types::*;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use ed25519_dalek::Verifier as _;
use serde_json::json;

fn t(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn spec() -> JobSpec {
    JobSpec {
        job_id: JobId::new("job_1"),
        kind: JobKind::Fetch,
        image: ImageRef::parse(&format!("agentrix/room-fetch@{}", Digest::of(b"img"))).unwrap(),
        inputs: vec![Digest::of(b"input")],
        allowlist: vec!["fixture.example".into(), "*.fixture.example:443".into()],
        caps: JobKind::Fetch.default_caps(),
        deadline: t("2026-11-07T20:00:00Z"),
        args: json!({"start": "https://fixture.example/", "max_pages": 40}),
    }
}

#[test]
fn canonical_json_is_rfc_8785() {
    let a = json!({"b": 2, "a": [1, "x", true, null], "c": {"z": 0.5, "y": "é"}});
    let b = json!({"c": {"y": "é", "z": 0.5}, "a": [1, "x", true, null], "b": 2});
    let expected = r#"{"a":[1,"x",true,null],"b":2,"c":{"y":"é","z":0.5}}"#;
    assert_eq!(canonical_json(&a).unwrap(), expected.as_bytes());
    assert_eq!(canonical_json(&b).unwrap(), expected.as_bytes());
}

#[test]
fn signed_job_spec_round_trips_through_json() {
    let control = Signer::generate();
    let signed = SignedJobSpec::sign(spec(), &control).unwrap();
    let wire = serde_json::to_string(&signed).unwrap();
    let received: SignedJobSpec = serde_json::from_str(&wire).unwrap();
    assert_eq!(received.verify(&control.verifying_key()).unwrap(), &spec());
}

#[test]
fn a_tampered_or_foreign_spec_is_refused() {
    let control = Signer::generate();
    let mut signed = SignedJobSpec::sign(spec(), &control).unwrap();
    signed.spec.allowlist.push("169.254.169.254".into());
    assert!(matches!(
        signed.verify(&control.verifying_key()),
        Err(Error::BadSignature)
    ));

    let attacker = Signer::generate();
    let forged = SignedJobSpec::sign(spec(), &attacker).unwrap();
    assert!(matches!(
        forged.verify(&control.verifying_key()),
        Err(Error::WrongKey { .. })
    ));
}

fn job_receipt(control: &Signer) -> Receipt {
    let mut body = ReceiptBody::new(
        ReceiptKind::JobFetch,
        SiteId::new("site_1"),
        Subject::Job(JobId::new("job_1")),
        t("2026-11-07T19:00:00Z"),
        t("2026-11-07T19:03:12.250Z"),
    );
    body.inputs = vec![Digest::of(b"input")];
    body.policy = Some(Policy {
        allowlist: vec!["fixture.example".into()],
        caps: JobKind::Fetch.default_caps(),
        runtime: "runsc".into(),
    });
    body.observed = Some(Observed {
        hosts: vec!["fixture.example".into()],
        requests: 41,
        bytes: 3_145_728,
        refusals: vec![Refusal {
            host: "169.254.169.254".into(),
            port: 80,
            reason: "metadata".into(),
        }],
        peaks: Peaks {
            cpu_pct: 87.5,
            memory_mb: 612.0,
            pids: 41,
        },
    });
    body.outputs = vec![Digest::of(b"page-1"), Digest::of(b"page-2")];
    body.destroyed = Some(true);
    let body = body.with_title("Read 2 pages of fixture.example in a sealed room");
    Receipt::sign(body, control).unwrap()
}

#[test]
fn receipt_round_trips_and_verifies_against_the_published_keys() {
    let control = Signer::generate();
    let receipt = job_receipt(&control);
    let wire = serde_json::to_string(&receipt).unwrap();
    let received: Receipt = serde_json::from_str(&wire).unwrap();
    assert_eq!(received, receipt);

    let keys = KeySet {
        keys: vec![control.published_key()],
    };
    let keys: KeySet = serde_json::from_str(&serde_json::to_string(&keys).unwrap()).unwrap();
    let body = received.verify_with(&keys).unwrap();
    assert_eq!(body.kind, ReceiptKind::JobFetch);
    assert_eq!(
        body.title(),
        Some("Read 2 pages of fixture.example in a sealed room")
    );
    assert_eq!(received.digest().unwrap(), receipt.digest().unwrap());
}

#[test]
fn anyone_can_verify_a_receipt_without_this_crate() {
    let control = Signer::generate();
    let receipt = job_receipt(&control);
    let mut object: serde_json::Value = serde_json::to_value(&receipt).unwrap();
    let signature = object.as_object_mut().unwrap().remove("signature").unwrap();
    let bytes = serde_jcs::to_vec(&object).unwrap();
    let sig = URL_SAFE_NO_PAD.decode(signature.as_str().unwrap()).unwrap();
    let sig = ed25519_dalek::Signature::from_slice(&sig).unwrap();
    let key = KeySet {
        keys: vec![control.published_key()],
    }
    .find(&KeyId::of(&control.verifying_key()))
    .unwrap();
    key.verify(&bytes, &sig).unwrap();
}

#[test]
fn a_tampered_receipt_is_refused() {
    let control = Signer::generate();
    let mut receipt = job_receipt(&control);
    receipt.body.destroyed = Some(false);
    assert!(matches!(
        receipt.verify(&control.verifying_key()),
        Err(Error::BadSignature)
    ));
}

#[test]
fn receipt_kinds_serialise_as_design_names_them() {
    for kind in ReceiptKind::ALL {
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, format!("\"{}\"", kind.as_str()));
        assert_eq!(ReceiptKind::parse(kind.as_str()), Some(kind));
    }
}

#[test]
fn fingerprint_binds_every_part_of_the_effect() {
    let effect = EffectRequest {
        site: SiteId::new("site_1"),
        webagent: "mover".into(),
        tool: "release.go_live".into(),
        args: json!({"release": 3, "pointer": "live"}),
        amount: None,
        expiry: t("2026-11-07T21:00:00Z"),
    };
    let reordered = EffectRequest {
        args: json!({"pointer": "live", "release": 3}),
        ..effect.clone()
    };
    assert_eq!(
        effect.fingerprint().unwrap(),
        reordered.fingerprint().unwrap()
    );

    let priced = EffectRequest {
        amount: Some(Money {
            currency: "KES".into(),
            minor: 150_000,
        }),
        ..effect.clone()
    };
    assert_ne!(effect.fingerprint().unwrap(), priced.fingerprint().unwrap());

    let card = DecisionCard::new(
        DecisionId::new("dec_1"),
        CardKind::GoLive,
        "Publish the new site",
        "Release 3 goes live; undo puts release 2 back.",
        effect,
        t("2026-11-07T20:30:00Z"),
    )
    .unwrap();
    assert!(card.fingerprint_matches().unwrap());
}

#[test]
fn job_stream_lines_match_the_runners_wire_shapes() {
    let killed = serde_json::to_string(&JobEvent::Killed {
        reason: KillReason::OutCap,
    })
    .unwrap();
    assert_eq!(killed, r#"{"event":"killed","reason":"out_cap"}"#);

    let refused = serde_json::to_string(&JobEvent::Refused(Refusal {
        host: "169.254.169.254".into(),
        port: 80,
        reason: "metadata".into(),
    }))
    .unwrap();
    assert_eq!(
        refused,
        r#"{"event":"refused","host":"169.254.169.254","port":80,"reason":"metadata"}"#
    );

    let request = JobEvent::parse(
        r#"{"event":"request","method":"CONNECT","host":"fixture.example","port":443,"bytes_up":512,"bytes_down":9000}"#,
    )
    .unwrap();
    assert!(matches!(request, JobEvent::Request { port: 443, .. }));

    let result = JobResult {
        job_id: JobId::new("job_1"),
        runtime: "runsc".into(),
        exit_code: Some(0),
        killed: None,
        artifacts: vec![ArtifactRef {
            digest: Digest::of(b"a"),
            name: "pages/index.html".into(),
            bytes: 1,
        }],
        requests: 12,
        refusals: vec![],
        peaks: Peaks::default(),
        destroyed: true,
        started_at: t("2026-11-07T19:00:00Z"),
        ended_at: t("2026-11-07T19:01:00Z"),
    };
    let line = serde_json::to_string(&JobEvent::Result(Box::new(result.clone()))).unwrap();
    assert!(line.starts_with(r#"{"event":"result","job_id":"job_1""#));
    assert_eq!(
        JobEvent::parse(&line).unwrap(),
        JobEvent::Result(Box::new(result))
    );
}

#[test]
fn approval_methods_are_written_as_the_approve_route_names_them() {
    assert_eq!(
        serde_json::to_string(&ApprovalMethod::Passkey).unwrap(),
        r#""passkey""#
    );
    assert_eq!(
        serde_json::to_string(&ApprovalMethod::StepUp).unwrap(),
        r#""totp""#
    );
}

#[test]
fn image_refs_must_be_pinned_by_digest() {
    assert!(ImageRef::parse("agentrix/room-fetch:latest").is_err());
    assert!(ImageRef::parse("agentrix/room-fetch@sha256:abc").is_err());
    let pinned = format!("ghcr.io/zavora/room-fetch@{}", Digest::of(b"x"));
    let image = ImageRef::parse(&pinned).unwrap();
    assert_eq!(image.name(), "ghcr.io/zavora/room-fetch");
    assert_eq!(image.digest(), Digest::of(b"x"));
}

#[test]
fn ui_events_use_kebab_case_names_seq_and_at() {
    let at = t("2026-11-07T19:00:00+03:00");
    let step = UiEnvelope {
        seq: 7,
        event: UiEvent::Step(StepEvent::new(
            StepName::GoLive,
            StepState::Waiting,
            1200,
            &"x".repeat(200),
            at,
        )),
    };
    let json = serde_json::to_value(&step).unwrap();
    assert_eq!(json["seq"], 7);
    assert_eq!(json["event"], "step");
    assert_eq!(json["data"]["step"], "golive");
    assert_eq!(json["data"]["at"], "2026-11-07T16:00:00Z");
    assert_eq!(
        json["data"]["summary"].as_str().unwrap().chars().count(),
        140
    );
    let back: UiEnvelope = serde_json::from_value(json).unwrap();
    assert_eq!(back, step);

    let parity = UiEvent::Parity(ParityEvent {
        page: None,
        score: 0.97,
        verdict: "Matches".into(),
        state: ParityState::Pass,
        attempt: 1,
        at,
    });
    assert!(parity.data_json().unwrap().contains(r#""page":null"#));

    let frame = UiEnvelope {
        seq: 8,
        event: UiEvent::Frame(ui::FrameEvent::new(
            FrameKind::SealedScreenshot,
            "https://fixture.example/",
            Digest::of(b"frame"),
            1280,
            800,
            at,
        )),
    };
    let sse = frame.to_sse().unwrap();
    assert!(sse.starts_with("id: 8\nevent: frame\ndata: {"));
    assert!(sse.contains(r#""kind":"sealed-screenshot""#));
    assert!(sse.ends_with("}\n\n"));

    let data = frame.event.data_json().unwrap();
    assert_eq!(UiEvent::from_parts("frame", &data).unwrap(), frame.event);
    assert_eq!(
        UiEvent::ReceiptTick(ui::TickEvent {
            job: None,
            hosts: 1,
            requests: 2,
            bytes: 3,
            refusals: 0,
            at
        })
        .name(),
        "receipt-tick"
    );
}

#[test]
fn a_job_receipt_records_what_the_gauntlet_scores() {
    let spec = spec();
    let result = JobResult {
        job_id: spec.job_id.clone(),
        runtime: "runsc".into(),
        exit_code: None,
        killed: Some(KillReason::Pids),
        artifacts: vec![ArtifactRef {
            digest: Digest::of(b"page"),
            name: "pages/index.html".into(),
            bytes: 4,
        }],
        requests: 2,
        refusals: vec![Refusal {
            host: "169.254.169.254".into(),
            port: 80,
            reason: "metadata".into(),
        }],
        peaks: Peaks::default(),
        destroyed: true,
        started_at: t("2026-11-07T19:00:00Z"),
        ended_at: t("2026-11-07T19:00:30Z"),
    };
    let events = vec![
        JobEvent::Started {
            job_id: spec.job_id.clone(),
            runtime: "runsc".into(),
            at: t("2026-11-07T19:00:00Z"),
        },
        JobEvent::Request {
            method: "GET".into(),
            host: "fixture.example".into(),
            port: 443,
            bytes_up: 100,
            bytes_down: 2_000,
        },
        JobEvent::Request {
            method: "GET".into(),
            host: "fixture.example".into(),
            port: 443,
            bytes_up: 100,
            bytes_down: 3_000,
        },
        JobEvent::Killed {
            reason: KillReason::Pids,
        },
        JobEvent::Destroyed { confirmed: true },
    ];
    let body = ReceiptBody::for_job(SiteId::new("site_1"), &spec, &result, &events)
        .with_title("Stopped a build that forked too many processes");
    assert_eq!(body.kind, ReceiptKind::JobFetch);
    assert_eq!(body.subject, Subject::Job(spec.job_id.clone()));
    let observed = body.observed.as_ref().unwrap();
    assert_eq!(observed.hosts, ["fixture.example"]);
    assert_eq!(observed.bytes, 5_000);
    assert_eq!(observed.refusals[0].host, "169.254.169.254");
    assert_eq!(body.destroyed, Some(true));
    let detail = body.detail.as_ref().unwrap();
    assert_eq!(detail["killed"], "pids");
    assert_eq!(
        detail["events"],
        json!(["started", "request", "request", "killed", "destroyed"])
    );
    assert_eq!(
        body.title(),
        Some("Stopped a build that forked too many processes")
    );
    let signer = Signer::generate();
    Receipt::sign(body, &signer)
        .unwrap()
        .verify(&signer.verifying_key())
        .unwrap();
}
