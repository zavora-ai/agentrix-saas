//! The bots node may run, read from a credential file only node's user can read.
//!
//! `AGENTRIX_TELEGRAM_FILE` names a JSON file:
//!
//! ```json
//! {"bots": [{"site": "kilele-bakery-sample", "token": "123456:ABC…"}]}
//! ```
//!
//! A bot may also name `host` (the site's host, when it isn't the usual one) and `api` (a
//! self-hosted Telegram Bot API server instead of `https://api.telegram.org`).
//!
//! The token is held in a [`Token`], whose `Debug` never prints it and which has no `Display`,
//! so it can't reach a log line, an error, the outbox or a receipt by accident. A file that other
//! users can read is refused, and the refusal names the path, never the content.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A bot token. Never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// Wraps a token (tests and the file loader).
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// The secret itself, for building a Bot API URL. Nothing else may call this.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    /// A value derived from the token that Telegram echoes on each webhook call
    /// (`X-Telegram-Bot-Api-Secret-Token`), so the webhook knows the call came from Telegram.
    /// Hex of SHA-256 over a fixed label and the token: never the token itself.
    pub fn webhook_secret(&self) -> String {
        use sha2::{Digest as _, Sha256};
        let mut h = Sha256::new();
        h.update(b"agentrix-channels/webhook/v1\0");
        h.update(self.0.as_bytes());
        hex::encode(h.finalize())
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(hidden)")
    }
}

/// One bot and the site it speaks for.
#[derive(Debug, Clone)]
pub struct BotConfig {
    /// The site's id (`kilele-bakery-sample`): the bot answers for this site and links its owners.
    pub site: String,
    /// The host the site is served on, when it isn't `<site>.<platform host>` for a sample or
    /// the site's domain in `sites`.
    pub host: Option<String>,
    /// The Bot API token.
    pub token: Token,
    /// A self-hosted Telegram Bot API server, when not `https://api.telegram.org`.
    pub api: Option<String>,
}

/// How node receives updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Long polling with `getUpdates` (local runs: there's no public HTTPS).
    Poll,
    /// Telegram calls `POST /channels/telegram/{site}/webhook` (deploys).
    Webhook,
}

/// Every bot node runs.
#[derive(Debug, Clone)]
pub struct Config {
    /// The bots.
    pub bots: Vec<BotConfig>,
    /// How updates arrive.
    pub mode: Mode,
}

impl Config {
    /// The bot for `site`.
    pub fn bot(&self, site: &str) -> Option<&BotConfig> {
        self.bots.iter().find(|b| b.site == site)
    }
}

#[derive(Deserialize)]
struct FileBot {
    site: String,
    #[serde(default)]
    host: Option<String>,
    token: String,
    #[serde(default)]
    api: Option<String>,
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    bots: Vec<FileBot>,
}

/// Why the file wasn't used, in words for the log (never its content).
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Others can read it.
    #[error("{0} can be read by other users; run chmod 600 on it. Telegram stays off.")]
    Readable(PathBuf),
    /// It isn't valid.
    #[error("{0} isn't the expected JSON ({1}). Telegram stays off.")]
    Invalid(PathBuf, String),
    /// It couldn't be read.
    #[error("{0} couldn't be read ({1}). Telegram stays off.")]
    Unreadable(PathBuf, String),
}

/// Where the file is: `AGENTRIX_TELEGRAM_FILE`, else `<data dir>/agentrix-channels/telegram.json`
/// if it exists.
pub fn path(data_dir: &Path) -> Option<PathBuf> {
    match std::env::var_os("AGENTRIX_TELEGRAM_FILE") {
        Some(p) if !p.is_empty() => Some(expand_home(PathBuf::from(p))),
        _ => {
            let p = data_dir.join("agentrix-channels").join("telegram.json");
            p.exists().then_some(p)
        }
    }
}

fn expand_home(p: PathBuf) -> PathBuf {
    match (p.strip_prefix("~"), std::env::var_os("HOME")) {
        (Ok(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => p,
    }
}

/// Whether `path` is in systemd's credentials folder for this service (`$CREDENTIALS_DIRECTORY`,
/// where the node unit's `LoadCredential=telegram.json` puts the file).
fn in_credentials_dir(path: &Path) -> bool {
    std::env::var_os("CREDENTIALS_DIRECTORY")
        .filter(|d| !d.is_empty())
        .is_some_and(|d| path.starts_with(PathBuf::from(d)))
}

/// Whether a file with `mode` is readable by more than its owner. In systemd's credentials
/// folder, which only the service can open, systemd mounts files 0440, so group read is fine
/// there; anywhere else the file must be 0600.
fn too_open(mode: u32, in_credentials: bool) -> bool {
    let others = if in_credentials { 0o007 } else { 0o077 };
    mode & others != 0
}

/// Reads the bots from `path`. `Ok(None)` when the file lists no bot.
///
/// # Errors
///
/// When the file is readable by others, unreadable or not the expected JSON.
pub fn load(path: &Path) -> Result<Option<Config>, ConfigError> {
    let meta = std::fs::metadata(path)
        .map_err(|e| ConfigError::Unreadable(path.into(), e.kind().to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if too_open(meta.permissions().mode(), in_credentials_dir(path)) {
            return Err(ConfigError::Readable(path.into()));
        }
    }
    let _ = meta;
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError::Unreadable(path.into(), e.kind().to_string()))?;
    // serde_json's message can quote the input, so only its line and column are kept.
    let file: File = serde_json::from_str(&text).map_err(|e| {
        ConfigError::Invalid(
            path.into(),
            format!("line {}, column {}", e.line(), e.column()),
        )
    })?;
    let bots: Vec<BotConfig> = file
        .bots
        .into_iter()
        .filter(|b| !b.site.trim().is_empty() && b.token.contains(':'))
        .map(|b| BotConfig {
            site: b.site.trim().to_owned(),
            host: b
                .host
                .map(|h| h.trim().to_ascii_lowercase())
                .filter(|h| !h.is_empty()),
            token: Token::new(b.token.trim()),
            api: b
                .api
                .map(|a| a.trim().trim_end_matches('/').to_owned())
                .filter(|a| a.starts_with("http://") || a.starts_with("https://")),
        })
        .collect();
    if bots.is_empty() {
        return Ok(None);
    }
    let mode = match std::env::var("AGENTRIX_TELEGRAM_MODE").as_deref() {
        Ok("webhook") => Mode::Webhook,
        _ => Mode::Poll,
    };
    Ok(Some(Config { bots, mode }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, body: &str, mode: u32) -> PathBuf {
        let p = dir.join("telegram.json");
        std::fs::write(&p, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        p
    }

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("agx-ch-{}", uuid::Uuid::now_v7().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_private_file_loads_and_the_token_never_prints() {
        let d = tmp();
        let p = write(
            &d,
            r#"{"bots":[{"site":"kilele-bakery-sample","token":"123:SECRET-abc"}]}"#,
            0o600,
        );
        let cfg = load(&p).unwrap().unwrap();
        assert_eq!(cfg.bots[0].site, "kilele-bakery-sample");
        let printed = format!("{cfg:?}");
        assert!(!printed.contains("SECRET"), "{printed}");
        assert_ne!(cfg.bots[0].token.webhook_secret(), "123:SECRET-abc");
        assert!(!cfg.bots[0].token.webhook_secret().contains("SECRET"));
    }

    #[cfg(unix)]
    #[test]
    fn a_file_others_can_read_is_refused_without_quoting_it() {
        let d = tmp();
        let p = write(&d, r#"{"bots":[{"site":"s","token":"1:SECRET"}]}"#, 0o644);
        let err = load(&p).unwrap_err().to_string();
        assert!(err.contains("chmod 600"), "{err}");
        assert!(!err.contains("SECRET"));
    }

    #[test]
    fn systemd_credentials_may_be_group_readable_nothing_else_may() {
        // LoadCredential= mounts the file 0440 in a folder only the service can open.
        assert!(!too_open(0o440, true));
        assert!(!too_open(0o400, true));
        assert!(too_open(0o444, true));
        // Anywhere else, 0600 or tighter.
        assert!(too_open(0o440, false));
        assert!(too_open(0o640, false));
        assert!(!too_open(0o600, false));
        assert!(!too_open(0o400, false));
    }

    #[test]
    fn broken_json_is_refused_without_quoting_it() {
        let d = tmp();
        let p = write(&d, r#"{"bots":[{"site":"s","token":"1:SECRET"]}"#, 0o600);
        let err = load(&p).unwrap_err().to_string();
        assert!(!err.contains("SECRET"), "{err}");
    }

    #[test]
    fn a_file_with_no_usable_bot_is_none() {
        let d = tmp();
        let p = write(&d, r#"{"bots":[{"site":"s","token":"no-colon"}]}"#, 0o600);
        assert!(load(&p).unwrap().is_none());
    }
}
