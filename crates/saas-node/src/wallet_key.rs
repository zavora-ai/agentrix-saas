//! The key each company's wallet checkpoints are signed under, named for the node's platform
//! host: `wallet.<company>.<platform host>` (`wallet.agentrix.paypal.agentrix.website`).
//!
//! saas-wallet reads its key from `<wallets dir>/keys/<company>-wallet.key` and makes one named
//! for the old host when there is none. [`name_for_host`] runs first: a key already named for
//! this host is used as it is; one named for another host is kept beside it, as
//! `<company>-wallet.<its name>.key`, because the checkpoints it signed verify only against it,
//! and a new key under this host's name signs from then on.

use std::io::Write;
use std::path::Path;

use adk_auth::signed_audit::{AuditSigner, Ed25519Signer};
use saas_core::CompanyId;

/// The name `company`'s wallet checkpoints are signed under on a node at `platform_host`.
pub fn key_name(company: &CompanyId, platform_host: &str) -> String {
    format!(
        "wallet.{company}.{}",
        platform_host.trim().to_ascii_lowercase()
    )
}

/// Makes sure `company`'s wallet key in `wallets_dir` is named for `platform_host`; returns
/// the verifier key of the one it set aside, when it set one aside.
pub fn name_for_host(
    wallets_dir: &Path,
    company: &CompanyId,
    platform_host: &str,
) -> anyhow::Result<Option<String>> {
    let keys = wallets_dir.join("keys");
    std::fs::create_dir_all(&keys)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for d in [wallets_dir, keys.as_path()] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let file = keys.join(format!("{company}-wallet.key"));
    let want = key_name(company, platform_host);
    let mut set_aside = None;
    match std::fs::read_to_string(&file) {
        Ok(text) => {
            let old = Ed25519Signer::from_note_key(text.trim())
                .map_err(|e| anyhow::anyhow!("{}: {e:?}", file.display()))?;
            if old.key_name() == want {
                return Ok(None);
            }
            let kept = keys.join(format!(
                "{company}-wallet.{}.key",
                old.key_name().replace(['/', '\\'], "_")
            ));
            std::fs::rename(&file, &kept)?;
            tracing::info!(company = %company, old = %old.verifier_key(), kept = %kept.display(), "the wallet key was named for another host; it is kept for the checkpoints it signed");
            set_aside = Some(old.verifier_key());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let signer = Ed25519Signer::generate(want).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut out = options.open(&file)?;
    out.write_all(signer.to_note_key().as_bytes())?;
    out.sync_all()?;
    tracing::info!(company = %company, key = %signer.verifier_key(), "wallet checkpoints are signed under this host's name");
    Ok(set_aside)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_named_for_another_host_is_kept_and_a_new_one_signs() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        std::fs::create_dir_all(&keys).unwrap();
        let old = Ed25519Signer::generate("wallet.agentrix.agentrix.website").unwrap();
        std::fs::write(keys.join("agentrix-wallet.key"), old.to_note_key().as_str()).unwrap();

        let agentrix = CompanyId::agentrix();
        let aside = name_for_host(dir.path(), &agentrix, "paypal.agentrix.website").unwrap();
        assert_eq!(aside, Some(old.verifier_key()));
        let now = std::fs::read_to_string(keys.join("agentrix-wallet.key")).unwrap();
        let signer = Ed25519Signer::from_note_key(now.trim()).unwrap();
        assert_eq!(signer.key_name(), "wallet.agentrix.paypal.agentrix.website");
        let kept = keys.join("agentrix-wallet.wallet.agentrix.agentrix.website.key");
        assert!(kept.exists());

        // Named right already: left as it is.
        assert_eq!(
            name_for_host(dir.path(), &agentrix, "paypal.agentrix.website").unwrap(),
            None
        );
        let same = std::fs::read_to_string(keys.join("agentrix-wallet.key")).unwrap();
        assert_eq!(same, now);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(keys.join("agentrix-wallet.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // None yet: made under this host's name.
        let kilele = CompanyId::kilele();
        assert_eq!(
            name_for_host(dir.path(), &kilele, "paypal.agentrix.website").unwrap(),
            None
        );
        let k = std::fs::read_to_string(keys.join("kilele-bakery-wallet.key")).unwrap();
        assert_eq!(
            Ed25519Signer::from_note_key(k.trim()).unwrap().key_name(),
            "wallet.kilele-bakery.paypal.agentrix.website"
        );
    }
}
