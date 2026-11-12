//! Job specs, events and results (design §4.2): the shared types from `agentrix-types`, plus
//! the rules the runner adds before it lets a spec near Docker.

use chrono::{DateTime, Utc};

pub use agentrix_types::{
    ArtifactRef, Digest, JobEvent, JobKind, JobResult, JobSpec, KillReason, Peaks, Refusal,
    SignedJobSpec, VerifyingKey,
};

/// Largest CPU share a room may ask for.
pub const MAX_CPUS: f64 = 4.0;
/// Largest memory cap, in MiB.
pub const MAX_MEMORY_MB: u32 = 4096;
/// Largest process cap.
pub const MAX_PIDS: u32 = 2048;
/// Longest wall-clock time, in seconds.
pub const MAX_SECONDS: u32 = 1800;
/// Largest `/out`, in MiB.
pub const MAX_OUT_MB: u32 = 2048;
/// Most input blobs per job. A 50-page site's render job carries every page plus its shared
/// stylesheets and images (107 files for the demo fixture), so 64 was too few; each input is
/// still uploaded and checked by digest and size.
pub const MAX_INPUTS: usize = 512;
/// Most allowlist entries per job.
pub const MAX_ALLOW: usize = 32;

/// Why a spec was refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SpecError {
    /// The signature does not verify against control's key, or another key signed it.
    #[error("the job spec's signature does not verify: {0}")]
    BadSignature(String),
    /// A field breaks a rule.
    #[error("invalid job spec: {0}")]
    Invalid(String),
}

fn invalid(msg: impl Into<String>) -> SpecError {
    SpecError::Invalid(msg.into())
}

/// Checks control's signature (`agentrix-types` canonicalises and verifies), then the rules.
pub fn verify<'a>(
    signed: &'a SignedJobSpec,
    key: &VerifyingKey,
    now: DateTime<Utc>,
) -> Result<&'a JobSpec, SpecError> {
    let spec = signed
        .verify(key)
        .map_err(|e| SpecError::BadSignature(e.to_string()))?;
    validate(spec, now)?;
    Ok(spec)
}

/// Whether a kind of room gets a proxy on the rooms network. A replay gets one with an empty
/// allowlist, so every attempt to leave is refused and recorded.
pub fn proxied(kind: JobKind) -> bool {
    matches!(kind, JobKind::Fetch | JobKind::Replay)
}

/// The rules a spec must meet beyond its signature. The job id and the image become Docker
/// names and command-line arguments, so their characters are checked here.
pub fn validate(spec: &JobSpec, now: DateTime<Utc>) -> Result<(), SpecError> {
    let id = spec.job_id.as_str();
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(invalid("job_id must be 1-64 of [A-Za-z0-9_-]"));
    }
    let name = spec.image.name();
    if name.is_empty()
        || name.starts_with('-')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"./_-:".contains(&b))
    {
        return Err(invalid(
            "the image name has characters Docker does not allow",
        ));
    }
    let c = spec.caps;
    if !(c.cpus > 0.0 && c.cpus <= MAX_CPUS) {
        return Err(invalid(format!("caps.cpus must be in (0, {MAX_CPUS}]")));
    }
    for (name, v, max) in [
        ("memory_mb", c.memory_mb, MAX_MEMORY_MB),
        ("pids", c.pids, MAX_PIDS),
        ("seconds", c.seconds, MAX_SECONDS),
        ("out_mb", c.out_mb, MAX_OUT_MB),
    ] {
        if v == 0 || v > max {
            return Err(invalid(format!("caps.{name} must be in 1..={max}")));
        }
    }
    if c.memory_mb < 64 {
        return Err(invalid("caps.memory_mb must be at least 64"));
    }
    if spec.is_expired(now) {
        return Err(invalid("the deadline has passed"));
    }
    if spec.inputs.len() > MAX_INPUTS {
        return Err(invalid(format!("at most {MAX_INPUTS} inputs")));
    }
    if spec.allowlist.len() > MAX_ALLOW {
        return Err(invalid(format!("at most {MAX_ALLOW} allowlist entries")));
    }
    match spec.kind {
        JobKind::Fetch if spec.allowlist.is_empty() => {
            return Err(invalid("a fetch job needs an allowlist"));
        }
        JobKind::Replay | JobKind::Render | JobKind::Build if !spec.allowlist.is_empty() => {
            return Err(invalid("only fetch jobs may have an allowlist"));
        }
        _ => {}
    }
    for origin in &spec.allowlist {
        if !origin_ok(origin) {
            return Err(invalid(format!(
                "allowlist entry {origin:?} is not a host, host:port or *.suffix"
            )));
        }
    }
    Ok(())
}

/// A conservative check of agentrix-egress's `Origin` syntax: `host`, `host:port`, `*.suffix`,
/// optionally with `:port`. The proxy parses it again when it starts.
fn origin_ok(s: &str) -> bool {
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (s, None),
    };
    if let Some(p) = port
        && !p.parse::<u16>().is_ok_and(|n| n > 0)
    {
        return false;
    }
    let host = host.strip_prefix("*.").unwrap_or(host);
    !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentrix_types::{Caps, ImageRef, Signer};

    fn spec() -> JobSpec {
        JobSpec {
            job_id: "job-1".into(),
            kind: JobKind::Fetch,
            image: ImageRef::parse(&format!(
                "ghcr.io/zavora-ai/room-fetch@sha256:{}",
                "a".repeat(64)
            ))
            .unwrap(),
            inputs: vec![],
            allowlist: vec!["kilele.example".into(), "*.kilele.example:443".into()],
            caps: Caps {
                cpus: 2.0,
                memory_mb: 2048,
                pids: 256,
                seconds: 600,
                out_mb: 256,
            },
            deadline: Utc::now() + chrono::Duration::minutes(10),
            args: serde_json::json!({"url": "https://kilele.example/"}),
        }
    }

    #[test]
    fn a_signed_spec_verifies_and_a_tampered_one_does_not() {
        let control = Signer::from_secret_bytes(&[7; 32]);
        let signed = SignedJobSpec::sign(spec(), &control).unwrap();
        assert!(verify(&signed, &control.verifying_key(), Utc::now()).is_ok());
        let mut tampered = signed.clone();
        tampered.spec.allowlist.push("evil.example".into());
        assert!(matches!(
            verify(&tampered, &control.verifying_key(), Utc::now()),
            Err(SpecError::BadSignature(_))
        ));
        let other = Signer::from_secret_bytes(&[8; 32]);
        assert!(matches!(
            verify(&signed, &other.verifying_key(), Utc::now()),
            Err(SpecError::BadSignature(_))
        ));
    }

    #[test]
    fn the_wire_form_is_agentrix_types_own() {
        let control = Signer::from_secret_bytes(&[7; 32]);
        let signed = SignedJobSpec::sign(spec(), &control).unwrap();
        let wire = serde_json::to_string(&signed).unwrap();
        let back: SignedJobSpec = serde_json::from_str(&wire).unwrap();
        assert!(verify(&back, &control.verifying_key(), Utc::now()).is_ok());
    }

    #[test]
    fn rules_refuse_bad_ids_names_caps_and_misplaced_allowlists() {
        let now = Utc::now();
        let mut s = spec();
        s.job_id = "../etc".into();
        assert!(validate(&s, now).is_err());
        let mut s = spec();
        s.image = ImageRef::parse(&format!("--privileged@sha256:{}", "a".repeat(64))).unwrap();
        assert!(
            validate(&s, now).is_err(),
            "an image name must not read as a flag"
        );
        let mut s = spec();
        s.kind = JobKind::Render;
        assert!(validate(&s, now).is_err(), "render has no network");
        let mut s = spec();
        s.allowlist = vec![];
        assert!(validate(&s, now).is_err(), "fetch needs an allowlist");
        let mut s = spec();
        s.allowlist = vec!["http://x".into()];
        assert!(validate(&s, now).is_err());
        let mut s = spec();
        s.caps.memory_mb = 8192;
        assert!(validate(&s, now).is_err());
        let mut s = spec();
        s.deadline = now - chrono::Duration::seconds(1);
        assert!(validate(&s, now).is_err());
        assert!(validate(&spec(), now).is_ok());
    }

    #[test]
    fn a_fifty_page_render_fits_and_the_cap_still_holds() {
        let mut render = spec();
        render.kind = JobKind::Render;
        render.allowlist.clear();
        // The demo fixture's candidate: 50 pages and 57 shared assets.
        render.inputs = (0..107)
            .map(|i| agentrix_types::Digest::of(format!("file {i}").as_bytes()))
            .collect();
        assert!(validate(&render, Utc::now()).is_ok());
        render.inputs = (0..=MAX_INPUTS)
            .map(|i| agentrix_types::Digest::of(format!("file {i}").as_bytes()))
            .collect();
        assert!(
            validate(&render, Utc::now()).is_err(),
            "more than MAX_INPUTS is still refused"
        );
    }

    #[test]
    fn events_serialise_with_an_event_tag() {
        let line = serde_json::to_string(&JobEvent::Killed {
            reason: KillReason::Pids,
        })
        .unwrap();
        assert_eq!(line, r#"{"event":"killed","reason":"pids"}"#);
    }
}
