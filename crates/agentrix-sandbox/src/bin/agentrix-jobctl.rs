//! `agentrix-jobctl`: signs job specs for `agentrix-runner` in tests and smoke checks, the way
//! control does in production (design §4.2). It never runs anywhere but a test bench.
//!
//! ```text
//! agentrix-jobctl keygen <dir>                  writes control-job.key (secret) and control-job.pub
//! agentrix-jobctl digest <file>                 prints sha256:<hex>, the path for PUT /blobs
//! agentrix-jobctl sign --key <file> <spec.json> prints a SignedJobSpec for POST /jobs
//! ```
//!
//! The spec is a `JobSpec` as JSON. `job_id` defaults to a fresh id and `deadline` to now plus
//! `caps.seconds` plus a minute.

use std::path::{Path, PathBuf};

use agentrix_types::{Digest, JobId, JobSpec, SignedJobSpec, Signer};
use clap::{Parser, Subcommand};

/// Command-line arguments.
#[derive(Debug, Parser)]
#[command(
    version,
    about = "Signs job specs for agentrix-runner (test bench only)"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Writes a fresh test signing key: `control-job.key` (hex secret) and `control-job.pub`.
    Keygen {
        /// Where to write them.
        dir: PathBuf,
    },
    /// Prints a file's content address.
    Digest {
        /// The file.
        file: PathBuf,
    },
    /// Signs a spec and prints the SignedJobSpec.
    Sign {
        /// The secret key file from `keygen`.
        #[arg(long)]
        key: PathBuf,
        /// The spec, as JSON.
        spec: PathBuf,
    },
}

fn read_secret(path: &Path) -> anyhow::Result<Signer> {
    let bytes = hex::decode(std::fs::read_to_string(path)?.trim())?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("{} must hold 32 bytes of hex", path.display()))?;
    Ok(Signer::from_secret_bytes(&arr))
}

fn main() -> anyhow::Result<()> {
    match Args::parse().command {
        Command::Keygen { dir } => {
            std::fs::create_dir_all(&dir)?;
            let signer = Signer::generate();
            let secret = dir.join("control-job.key");
            std::fs::write(&secret, hex::encode(signer.secret_bytes()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600))?;
            }
            std::fs::write(
                dir.join("control-job.pub"),
                hex::encode(signer.verifying_key().as_bytes()),
            )?;
            println!("{}", signer.key_id().as_str());
        }
        Command::Digest { file } => println!("{}", Digest::of(&std::fs::read(file)?)),
        Command::Sign { key, spec } => {
            let signer = read_secret(&key)?;
            let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(spec)?)?;
            let obj = value
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("the spec must be a JSON object"))?;
            if !obj.contains_key("job_id") {
                obj.insert("job_id".into(), JobId::generate().as_str().into());
            }
            if !obj.contains_key("deadline") {
                let seconds = obj
                    .get("caps")
                    .and_then(|c| c.get("seconds"))
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(600);
                let deadline = chrono::Utc::now() + chrono::Duration::seconds(seconds + 60);
                obj.insert("deadline".into(), deadline.to_rfc3339().into());
            }
            let spec: JobSpec = serde_json::from_value(value)?;
            println!(
                "{}",
                serde_json::to_string(&SignedJobSpec::sign(spec, &signer)?)?
            );
        }
    }
    Ok(())
}
