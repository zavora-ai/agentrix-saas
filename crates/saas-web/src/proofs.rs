//! `money.agentrix.website/proofs`: what anyone can check for themselves, read from files and
//! the feed, never typed in here.
//!
//! From the proofs directory ([`crate::Face::with_proofs_dir`], `SAAS_PROOFS_DIR` on the node,
//! the repo's `specs/saas/` locally):
//!
//! - `PROOFS.md`: the attack suite and how it is scored, rendered as Markdown (raw HTML in it is
//!   shown as text).
//! - `proofs/attacks.json`: the suite's results. Read tolerantly: `{suite, committed, ranAt,
//!   scoredFrom, cases: [...]}` or a bare array of cases, each with `id`, `name` or `title`,
//!   `attack` or `description`, `expected`, `outcome` or `result`, `passed` or `pass`, and
//!   `evidence` or `receipt`.
//! - `proofs/audit/<company>.jsonl` and `<company>.vkey`: the signed audit log's export and the
//!   key that verifies it, offered for download with the `agentrix-saas audit verify` line
//!   (adk-auth's signed-audit verifier, the same code as `adk-rust audit verify` at the pin).
//! - `attacks.json`: the case list, committed by its SHA-256 before the run; the page hashes it
//!   and says whether the run's `committed` names it.
//!
//! From the feed: the audit log's last check. From the agent door: the agents' public keys.
//! A file that isn't there yet says so; nothing is filled in.

use std::path::{Path as FsPath, PathBuf};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pulldown_cmark::{Event, Options, Parser, html};
use serde_json::{Value, json};
use sha2::Digest;

use crate::{Ctx, pages, site_links};

/// One case of the attack suite, as the page shows it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Case {
    /// The case's id.
    pub id: String,
    /// What it is called.
    pub name: String,
    /// What was tried.
    pub attack: String,
    /// What should happen.
    pub expected: String,
    /// What happened.
    pub outcome: String,
    /// True when it held.
    pub passed: bool,
    /// What shows it (a receipt, a PayPal id, a log line).
    pub evidence: String,
}

/// The suite's results.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Suite {
    /// Its name.
    pub suite: String,
    /// The case list's hash, committed before the run.
    pub committed: String,
    /// When it ran.
    pub ran_at: String,
    /// What it was scored from.
    pub scored_from: String,
    /// The cases.
    pub cases: Vec<Case>,
}

fn text(v: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| v.get(*k))
        .map(|x| match x {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

/// Reads the suite's results, as tolerantly as the module says.
pub fn parse_suite(v: &Value) -> Option<Suite> {
    let (meta, cases) = match v {
        Value::Array(cases) => (&Value::Null, cases),
        Value::Object(_) => (v, v.get("cases")?.as_array()?),
        _ => return None,
    };
    let cases = cases
        .iter()
        .map(|c| {
            let outcome = text(c, &["outcome", "result", "actual"]);
            let expected = text(c, &["expected", "want"]);
            let passed = c
                .get("passed")
                .or_else(|| c.get("pass"))
                .and_then(Value::as_bool)
                .unwrap_or(!outcome.is_empty() && outcome == expected);
            Case {
                id: text(c, &["id"]),
                name: text(c, &["name", "title", "id"]),
                attack: text(c, &["attack", "description", "input"]),
                expected,
                outcome,
                passed,
                evidence: text(c, &["evidence", "receipt", "proof"]),
            }
        })
        .collect();
    Some(Suite {
        suite: text(meta, &["suite", "name"]),
        committed: text(meta, &["committed", "caseListHash", "hash"]),
        ran_at: text(meta, &["ranAt", "ran_at", "at"]),
        scored_from: text(meta, &["scoredFrom", "scored_from"]),
        cases,
    })
}

/// Markdown to HTML, with any raw HTML in it shown as text.
pub fn markdown(src: &str) -> String {
    let events = Parser::new_ext(src, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH).map(
        |e| match e {
            Event::Html(h) | Event::InlineHtml(h) => Event::Text(h),
            other => other,
        },
    );
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}

/// The audit exports in `dir/proofs/audit`: `(company, has key)` for each `<company>.jsonl`.
fn exports(dir: &FsPath) -> Vec<(String, bool)> {
    let audit = dir.join("proofs/audit");
    let Ok(entries) = std::fs::read_dir(&audit) else {
        return Vec::new();
    };
    let mut out: Vec<(String, bool)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter_map(|n| n.strip_suffix(".jsonl").map(str::to_owned))
        .filter(|n| safe_name(n))
        .map(|n| {
            let key = audit.join(format!("{n}.vkey")).is_file();
            (n, key)
        })
        .collect();
    out.sort();
    out
}

fn safe_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `/proofs`.
pub(crate) async fn page(State(ctx): State<Ctx>, headers: HeaderMap) -> Response {
    let feed = match ctx.shared.feed.feed((ctx.shared.today)()).await {
        Ok(f) => f,
        Err(e) => return crate::failed(e),
    };
    let dir = ctx.shared.proofs_dir.clone();
    let read = |rel: &str| {
        dir.as_ref()
            .and_then(|d| std::fs::read_to_string(d.join(rel)).ok())
    };

    let proofs_md = read("PROOFS.md").map(|md| markdown(&md));
    let suite = read("proofs/attacks.json")
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| parse_suite(&v));
    let held = suite
        .as_ref()
        .map_or(0, |s| s.cases.iter().filter(|c| c.passed).count());
    // The case list was committed by its SHA-256 before the run: hash the committed file and
    // say whether the run's `committed` names it.
    let commitment = dir
        .as_ref()
        .and_then(|d| std::fs::read(d.join("attacks.json")).ok())
        .map(|bytes| {
            let hash = hex::encode(sha2::Sha256::digest(&bytes));
            let claimed = suite
                .as_ref()
                .map(|s| s.committed.trim_start_matches("sha256:").to_ascii_lowercase());
            json!({"hash": format!("sha256:{hash}"), "matches": claimed.as_deref() == Some(hash.as_str())})
        });
    let exports: Vec<Value> = dir
        .as_deref()
        .map(exports)
        .unwrap_or_default()
        .into_iter()
        .map(|(company, key)| {
            json!({
                "company": company,
                "log": format!("/proofs/files/proofs/audit/{company}.jsonl"),
                "key": key.then(|| format!("/proofs/files/proofs/audit/{company}.vkey")),
                "command": format!("agentrix-saas audit verify {company}.jsonl --key \"$(cat {company}.vkey)\""),
            })
        })
        .collect();

    let keys: Vec<Value> = ctx
        .shared
        .keys
        .iter()
        .map(|k| json!({"kid": k.kid, "company": k.company, "agent": k.agent, "x": k.x}))
        .collect();
    let signers: Vec<Value> = feed
        .companies
        .iter()
        .flat_map(|c| {
            c.agents.iter().filter_map(move |a| {
                a.key_id
                    .as_ref()
                    .map(|kid| json!({"company": c.name, "agent": a.name, "kid": kid}))
            })
        })
        .collect();
    let links = site_links(&headers, &ctx.shared.hosts);

    let view = json!({
        "audit": feed.audit,
        "source": feed.source,
        "as_of": feed.as_of.to_string(),
        // Rendered from the repo's own file with raw HTML turned to text; the template marks it safe.
        "proofs_md": proofs_md,
        "suite": suite,
        "held": held,
        "commitment": commitment,
        "exports": exports,
        "keys": keys,
        "signers": signers,
        "links": links,
    });
    pages::render("saas/proofs.html", view)
}

/// `/proofs/files/{path}`: `PROOFS.md`, `proofs/attacks.json`, and the audit exports, as
/// they are on disk.
pub(crate) async fn file(State(ctx): State<Ctx>, Path(path): Path<String>) -> Response {
    let Some(dir) = ctx.shared.proofs_dir.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some((rel, content_type)) = allowed(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match std::fs::read(dir.join(&rel)) {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, content_type),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::CACHE_CONTROL, "no-cache"),
                (
                    header::CONTENT_DISPOSITION,
                    if rel.extension().is_some_and(|e| e == "md") {
                        "inline"
                    } else {
                        "attachment"
                    },
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The files `/proofs/files/` serves, and nothing else.
fn allowed(path: &str) -> Option<(PathBuf, &'static str)> {
    match path {
        "PROOFS.md" => return Some((PathBuf::from("PROOFS.md"), "text/markdown; charset=utf-8")),
        "proofs/attacks.json" => {
            return Some((PathBuf::from("proofs/attacks.json"), "application/json"));
        }
        "attacks.json" => return Some((PathBuf::from("attacks.json"), "application/json")),
        _ => {}
    }
    let name = path.strip_prefix("proofs/audit/")?;
    let (stem, ext) = name.rsplit_once('.')?;
    let content_type = match ext {
        "jsonl" => "application/x-ndjson",
        "vkey" => "text/plain; charset=utf-8",
        _ => return None,
    };
    safe_name(stem).then(|| (PathBuf::from("proofs/audit").join(name), content_type))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_suite_is_read_in_either_shape() {
        let v = json!({"suite": "attacks", "committed": "sha256:ab", "ranAt": "2026-11-11T09:00:00Z", "cases": [
            {"id": "pdf", "name": "Bank details changed", "attack": "a PDF", "expected": "refused", "outcome": "refused", "passed": true, "evidence": "receipt 412"},
            {"id": "webhook", "title": "Forged webhook", "description": "bad signature", "expected": "rejected", "result": "accepted"}
        ]});
        let s = parse_suite(&v).unwrap();
        assert_eq!(s.committed, "sha256:ab");
        assert!(s.cases[0].passed);
        assert_eq!(s.cases[1].name, "Forged webhook");
        assert!(
            !s.cases[1].passed,
            "an outcome that isn't the expected one doesn't hold"
        );
        let bare = parse_suite(&json!([{"id": "x", "pass": true}])).unwrap();
        assert!(bare.cases[0].passed && bare.suite.is_empty());
        assert!(parse_suite(&json!("nope")).is_none());
    }

    #[test]
    fn markdown_renders_and_raw_html_stays_text() {
        let h = markdown("# Proofs\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n<script>x</script>");
        assert!(h.contains("<h1>Proofs</h1>") && h.contains("<table>"));
        assert!(!h.contains("<script>"));
    }

    #[test]
    fn only_the_proof_files_are_served() {
        assert!(allowed("PROOFS.md").is_some());
        assert!(allowed("proofs/attacks.json").is_some());
        assert_eq!(
            allowed("proofs/audit/agentrix.jsonl").unwrap().1,
            "application/x-ndjson"
        );
        assert!(allowed("proofs/audit/agentrix.vkey").is_some());
        for bad in [
            "../../.env",
            "proofs/audit/../x.jsonl",
            "proofs/audit/a.key",
            "README.md",
            "proofs/audit/.jsonl",
        ] {
            assert!(allowed(bad).is_none(), "{bad}");
        }
    }
}
