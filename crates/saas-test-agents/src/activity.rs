//! What the test agents did, kept in memory for the control page and appended to
//! `activity.jsonl` in the data directory.

use std::collections::VecDeque;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One thing an agent did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Activity {
    /// When.
    pub at: DateTime<Utc>,
    /// Which agent (`amani`).
    pub agent: String,
    /// Which site (an origin).
    pub site: String,
    /// `browse`, `quote`, `checkout`, `ask`, `custom`, `impostor`, `pay`, `error`.
    pub kind: String,
    /// Whether it went as expected (an impostor's refusal is `true`).
    pub ok: bool,
    /// What happened, in a line.
    pub detail: String,
}

/// The log.
#[derive(Debug)]
pub struct Log {
    recent: Mutex<VecDeque<Activity>>,
    file: PathBuf,
}

impl Log {
    /// Appends to `file`.
    pub fn new(file: PathBuf) -> Self {
        Self {
            recent: Mutex::new(VecDeque::new()),
            file,
        }
    }

    /// Records one activity.
    pub fn record(&self, agent: &str, site: &str, kind: &str, ok: bool, detail: impl Into<String>) {
        let entry = Activity {
            at: Utc::now(),
            agent: agent.into(),
            site: site.into(),
            kind: kind.into(),
            ok,
            detail: detail.into(),
        };
        tracing::info!(agent, site, kind, ok, detail = %entry.detail, "test agents");
        if let Ok(line) = serde_json::to_string(&entry) {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.file)
            {
                let _ = writeln!(f, "{line}");
            }
        }
        let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        recent.push_front(entry);
        recent.truncate(200);
    }

    /// The most recent, newest first.
    pub fn recent(&self, n: usize) -> Vec<Activity> {
        self.recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .take(n)
            .cloned()
            .collect()
    }
}
