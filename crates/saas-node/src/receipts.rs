//! The node's receipts: what agentrix-effects issues for every PayPal movement, signed with the
//! node's Ed25519 key (agentrix-types' `Receipt::sign`) and kept as JSON lines in the data dir.
//!
//! The key lives at `<data_dir>/keys/receipts.key` (mode 0600, made on first boot). Judge mode
//! keeps receipts in memory with a fresh key.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use agentrix_host::Receipts;
use agentrix_types::{Receipt, ReceiptBody, Signer};
use async_trait::async_trait;

/// Signed receipts.
pub struct NodeReceipts {
    signer: Signer,
    file: Option<PathBuf>,
    issued: Mutex<HashMap<String, String>>,
}

impl std::fmt::Debug for NodeReceipts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeReceipts")
            .field("key", &self.signer.key_id().as_str())
            .finish_non_exhaustive()
    }
}

fn write_secret(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

impl NodeReceipts {
    /// Receipts signed with the key in `data_dir`, kept in `<data_dir>/receipts.jsonl`.
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let key = data_dir.join("keys").join("receipts.key");
        let signer = match std::fs::read_to_string(&key) {
            Ok(text) => {
                let bytes: [u8; 32] = hex::decode(text.trim())?
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("{} isn't a 32-byte key", key.display()))?;
                Signer::from_secret_bytes(&bytes)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let signer = Signer::generate();
                write_secret(&key, hex::encode(signer.secret_bytes()).as_bytes())?;
                signer
            }
            Err(e) => return Err(e.into()),
        };
        let file = data_dir.join("receipts.jsonl");
        let mut issued = HashMap::new();
        if let Ok(text) = std::fs::read_to_string(&file) {
            for line in text.lines() {
                if let Ok(receipt) = serde_json::from_str::<Receipt>(line)
                    && let Ok(digest) = receipt.digest()
                {
                    issued.insert(digest.to_string(), line.to_owned());
                }
            }
        }
        Ok(Self {
            signer,
            file: Some(file),
            issued: Mutex::new(issued),
        })
    }

    /// In memory with a fresh key: judge mode and tests.
    pub fn ephemeral() -> Self {
        Self {
            signer: Signer::generate(),
            file: None,
            issued: Mutex::new(HashMap::new()),
        }
    }

    /// The key receipts are signed with.
    pub fn key_id(&self) -> String {
        self.signer.key_id().as_str().to_owned()
    }
}

#[async_trait]
impl Receipts for NodeReceipts {
    async fn issue(&self, body: ReceiptBody) -> anyhow::Result<String> {
        let receipt = Receipt::sign(body, &self.signer)?;
        let digest = receipt.digest()?.to_string();
        let line = String::from_utf8(agentrix_types::canonical_json(&receipt)?)?;
        if let Some(file) = &self.file {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file)?;
            writeln!(f, "{line}")?;
        }
        self.issued
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(digest.clone(), line);
        Ok(digest)
    }

    async fn get_json(&self, digest: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .issued
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(digest)
            .cloned())
    }
}
