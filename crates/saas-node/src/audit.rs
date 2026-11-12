//! `agentrix-saas audit`: the signed audit logs, exported and verified.
//!
//! [`verify`] runs adk-auth's signed-audit verifier (`AuditExport::verify` over trusted
//! `NoteVerifier` keys), the same code `adk-rust audit verify` runs at the pinned adk-rust rev,
//! with the same arguments and exit status: 0 when the log verifies, 1 when it doesn't.

use std::path::{Path, PathBuf};

use adk_auth::signed_audit::{AuditExport, FileCheckpointAnchor, NoteVerifier};
use anyhow::Context;

/// Verifies the export at `export_path` against the trusted `keys` (and those in `key_file`,
/// one per line), with any extra checkpoint files. Prints the report; returns whether it is
/// valid.
pub fn verify(
    export_path: &Path,
    mut keys: Vec<String>,
    key_file: Option<&Path>,
    checkpoint_files: &[PathBuf],
    json: bool,
) -> anyhow::Result<bool> {
    if let Some(path) = key_file {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        keys.extend(
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(String::from),
        );
    }
    anyhow::ensure!(!keys.is_empty(), "give --key or --key-file");
    let trusted = keys
        .iter()
        .map(|key| {
            key.parse::<NoteVerifier>()
                .map_err(|e| anyhow::anyhow!("invalid verifier key '{key}': {e:?}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let text = std::fs::read_to_string(export_path)
        .with_context(|| format!("reading {}", export_path.display()))?;
    let mut export = AuditExport::from_jsonl(&text)
        .map_err(|(line, error)| anyhow::anyhow!("{}:{line}: {error}", export_path.display()))?;
    for path in checkpoint_files {
        export.checkpoints.extend(
            FileCheckpointAnchor::read(path)
                .with_context(|| format!("reading checkpoints {}", path.display()))?,
        );
    }
    let report = export.verify(&trusted);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{report}");
    }
    Ok(report.is_valid())
}

/// Writes `<out>/<company>.jsonl` and `<out>/<company>.vkey` from the company's database under
/// `data_dir` (`<data_dir>/<company>/<company>.db`).
pub async fn export(
    data_dir: &Path,
    company: &str,
    out: &Path,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    let audit = saas_company::stores::open_audit(company, &data_dir.join(company)).await?;
    audit.write_export_as(out, company).await
}
