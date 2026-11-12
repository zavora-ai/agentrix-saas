//! Running one agent: the [`AgentRunner`] seam, and [`ZavoraRunner`], which runs Zavora CLI.
//!
//! Each agent works in a room folder holding only the reference files it is given. Zavora's
//! headless mode refuses tools that need approval (`execute_bash`), and `fs_read` and `fs_write`
//! stop at the room's edge (Zavora's workspace path policy; W0 checked both on the real build,
//! 8 Nov). A designer gets exactly one tool approved, `--approve-tool fs_write`, so it can write
//! `index.html` in its room; the runner never passes `--always-approve`. The child gets a
//! cleared environment with only the Vultr key, `PATH`, `HOME` set to the room, an in-memory
//! session store and the prompt cap. A designer's room is kept for checks and the owner's
//! changes until the owner chooses; the planner's is deleted straight away. After every run the
//! runner reads each file in the room to confirm the key was never written there.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

use crate::stream::{StreamEvent, parse_line};

/// One agent's job.
#[derive(Debug, Clone, Default)]
pub struct AgentJob {
    /// The model by id, e.g. `deepseek-v4.1-flash-1017`.
    pub model: String,
    /// The whole prompt.
    pub prompt: String,
    /// Files placed in the room for the agent to read, by name (skipped when already there).
    pub files: Vec<(String, String)>,
    /// The room to work in, kept afterwards; `None` for a fresh room deleted after the run.
    pub room: Option<PathBuf>,
    /// Whether the agent may write files in its room (`--approve-tool fs_write`).
    pub write: bool,
    /// This turn's time cap, instead of the runner's.
    pub timeout: Option<Duration>,
}

/// What an agent wrote.
#[derive(Debug, Clone, Default)]
pub struct AgentOutput {
    /// The final answer (the result's `response`, or the joined text when there was no result).
    pub text: String,
    /// How long it took.
    pub duration_ms: u64,
    /// The model that answered, as Zavora reported it.
    pub model: Option<String>,
    /// The provider that answered, as Zavora reported it (`vultr`).
    pub provider: Option<String>,
    /// Files in the room read after the run to confirm the key isn't in any of them.
    pub key_checked_files: u64,
}

/// Runs one agent and streams what it does to `events`.
#[async_trait]
pub trait AgentRunner: Send + Sync {
    /// Runs `job` to the end. An error means the agent produced nothing usable.
    async fn run(
        &self,
        job: AgentJob,
        events: UnboundedSender<StreamEvent>,
    ) -> anyhow::Result<AgentOutput>;
}

/// Zavora CLI with the Vultr provider.
#[derive(Debug, Clone)]
pub struct ZavoraRunner {
    /// The binary (`AGENTRIX_ZAVORA_BIN`).
    pub bin: PathBuf,
    /// `VULTR_INFERENCE_API_KEY`, passed only to the child.
    pub api_key: String,
    /// Where fresh room folders are made.
    pub rooms: PathBuf,
    /// The longest one agent may run before it is stopped.
    pub timeout: Duration,
    /// Zavora's prompt cap (`ZAVORA_MAX_PROMPT_CHARS`).
    pub max_prompt_chars: usize,
}

impl ZavoraRunner {
    /// The command line for `job`, without the program (W0's working command).
    pub fn args(job: &AgentJob) -> Vec<String> {
        let mut args: Vec<String> = [
            "--worker-provider",
            "vultr",
            "--worker-model",
            &job.model,
            "--no-stdin",
            "--tool-confirmation-mode",
            "always",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if job.write {
            // Exactly one tool approved; everything else that needs approval stays refused.
            args.extend(["--approve-tool".to_owned(), "fs_write".to_owned()]);
        }
        args.extend(["-o".to_owned(), "stream-json".to_owned()]);
        args.push("ask".into());
        args.push(job.prompt.clone());
        args
    }
}

/// A room folder, deleted when dropped unless it is kept.
struct Room {
    path: PathBuf,
    keep: bool,
}

impl Drop for Room {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Every regular file under `dir` (not following links), up to a few thousand.
fn files_in(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() > 5000 {
            return;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            files_in(&entry.path(), out);
        } else if kind.is_file() {
            out.push(entry.path());
        }
    }
}

/// How many files in `room` were read, or an error naming the first one holding `key`.
pub fn key_absent(room: &Path, key: &str) -> anyhow::Result<u64> {
    let mut files = Vec::new();
    files_in(room, &mut files);
    for file in &files {
        if key.len() >= 8
            && let Ok(bytes) = std::fs::read(file)
            && bytes.windows(key.len()).any(|w| w == key.as_bytes())
        {
            anyhow::bail!(
                "the inference key was found in the room at {}",
                file.display()
            );
        }
    }
    Ok(u64::try_from(files.len()).unwrap_or(u64::MAX))
}

#[async_trait]
impl AgentRunner for ZavoraRunner {
    async fn run(
        &self,
        job: AgentJob,
        events: UnboundedSender<StreamEvent>,
    ) -> anyhow::Result<AgentOutput> {
        let room = match &job.room {
            Some(path) => Room {
                path: path.clone(),
                keep: true,
            },
            None => Room {
                path: self
                    .rooms
                    .join(format!("room-{}", uuid::Uuid::new_v4().simple())),
                keep: false,
            },
        };
        tokio::fs::create_dir_all(&room.path).await?;
        for (name, body) in &job.files {
            let file = room.path.join(name.replace(['/', '\\'], "_"));
            if !file.exists() {
                tokio::fs::write(file, body).await?;
            }
        }
        let started = Instant::now();
        let mut child = Command::new(&self.bin)
            .args(Self::args(&job))
            .current_dir(&room.path)
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", &room.path)
            .env("ZAVORA_SESSION_BACKEND", "memory")
            .env("ZAVORA_MAX_PROMPT_CHARS", self.max_prompt_chars.to_string())
            .env("VULTR_INFERENCE_API_KEY", &self.api_key)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow::anyhow!("starting Zavora CLI at {}: {e}", self.bin.display()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Zavora CLI gave no output"))?;
        let mut lines = BufReader::new(stdout).lines();
        let mut joined = String::new();
        let mut finished: Option<(String, Option<u64>, Option<String>, Option<String>)> = None;
        let mut fatal: Option<String> = None;
        let read = async {
            while let Some(line) = lines.next_line().await? {
                let Some(event) = parse_line(&line) else {
                    continue;
                };
                match &event {
                    StreamEvent::Text(t) => joined.push_str(t),
                    StreamEvent::Finished {
                        response,
                        duration_ms,
                        model,
                        provider,
                    } => {
                        finished = Some((
                            response.clone(),
                            *duration_ms,
                            model.clone(),
                            provider.clone(),
                        ));
                    }
                    StreamEvent::Problem {
                        fatal: true,
                        message,
                    } => fatal = Some(message.clone()),
                    _ => {}
                }
                let _ = events.send(event);
            }
            anyhow::Ok(())
        };
        let cap = job.timeout.unwrap_or(self.timeout);
        let timed_out = match tokio::time::timeout(cap, read).await {
            Ok(r) => {
                r?;
                false
            }
            Err(_) => {
                let _ = child.kill().await;
                true
            }
        };
        let key_checked_files = key_absent(&room.path, &self.api_key)?;
        if timed_out {
            anyhow::bail!(
                "the agent ran past {} seconds and was stopped",
                cap.as_secs()
            );
        }
        let status = child.wait().await?;
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match finished {
            Some((response, duration_ms, model, provider)) => Ok(AgentOutput {
                text: if response.trim().is_empty() {
                    joined
                } else {
                    response
                },
                duration_ms: duration_ms.unwrap_or(elapsed),
                model,
                provider,
                key_checked_files,
            }),
            None if !joined.trim().is_empty() && status.success() => Ok(AgentOutput {
                text: joined,
                duration_ms: elapsed,
                key_checked_files,
                ..AgentOutput::default()
            }),
            None => Err(anyhow::anyhow!(
                "{}",
                fatal.unwrap_or_else(|| format!("Zavora CLI ended without an answer ({status})"))
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_fs_write_is_ever_approved() {
        let mut job = AgentJob {
            model: "deepseek-v4.1-flash-1017".into(),
            prompt: "Write the page.".into(),
            ..AgentJob::default()
        };
        let args = ZavoraRunner::args(&job);
        assert!(!args.iter().any(|a| a.contains("approve")));
        assert_eq!(args[args.len() - 2], "ask");
        assert!(args.windows(2).any(|w| w == ["-o", "stream-json"]));
        assert!(
            args.windows(2)
                .any(|w| w == ["--tool-confirmation-mode", "always"])
        );
        job.write = true;
        let args = ZavoraRunner::args(&job);
        assert!(args.windows(2).any(|w| w == ["--approve-tool", "fs_write"]));
        assert_eq!(args.iter().filter(|a| a.contains("approve")).count(), 1);
        assert!(!args.iter().any(|a| a == "--always-approve"));
    }

    #[test]
    fn a_key_left_in_the_room_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".zavora")).unwrap();
        std::fs::write(dir.path().join("index.html"), "<p>ok</p>").unwrap();
        std::fs::write(dir.path().join(".zavora/log"), "nothing").unwrap();
        assert_eq!(key_absent(dir.path(), "vk-secret-123456").unwrap(), 2);
        std::fs::write(dir.path().join("leak.txt"), "key=vk-secret-123456").unwrap();
        assert!(key_absent(dir.path(), "vk-secret-123456").is_err());
    }
}
