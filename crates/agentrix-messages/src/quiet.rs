//! Quiet hours from W5's shell: one source for Night Time on screen and for held messages.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use agentrix_shell::store::ShellStore;

use crate::directory::Directory;
use crate::send::{QuietHours, quiet_end};

/// Reads each site's quiet hours from the shell (`ShellStore::quiet_hours`) and the site's
/// offset from the directory. A message with no site is never held.
pub struct ShellQuietHours {
    shell: ShellStore,
    directory: Arc<dyn Directory>,
}

impl ShellQuietHours {
    /// Over the shell's store and the directory.
    pub fn new(shell: ShellStore, directory: Arc<dyn Directory>) -> Self {
        Self { shell, directory }
    }
}

#[async_trait]
impl QuietHours for ShellQuietHours {
    async fn quiet_until(
        &self,
        _account: &str,
        site_id: Option<&str>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        let site_id = site_id?;
        let hours = match self.shell.quiet_hours(site_id).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(error = %e, "couldn't read quiet hours; sending now");
                return None;
            }
        };
        let offset = match self.directory.site(site_id).await {
            Ok(Some(site)) => site.utc_offset_minutes,
            _ => 0,
        };
        quiet_end(hours.start, hours.end, offset, now)
    }
}
