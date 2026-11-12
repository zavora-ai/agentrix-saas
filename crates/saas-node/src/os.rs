//! Face's three Agentrix OS apps (contract C9), Passkey, Badges and Wallet, served in one
//! binary: [`console_apps`] goes into node's shell, and the apps read the money layer in
//! process through the adapters here, which [`Shared`] fills once the money layer has started.
//!
//! - **Badges:** each site's agents' badges and the verifier ([`crate::badges::Badges`]).
//! - **Wallet:** the Cashier's wallet feed (saas-wallet).
//! - **Passkey:** the owner's moments, run by saas-badge's ceremonies (the PayPal link is its
//!   `BindingDesk`), sealed with the evidence the console's passkey produced.
//!
//! The shell's site id (`kilele-bakery-sample`) maps to face's site id (`kilele`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use async_trait::async_trait;
use saas_badge::ceremony::{CeremonyError, PasskeyEvidence};
use saas_core::identity::Badge;
use saas_core::wallet::WalletFeed;
use saas_web::Access;
use saas_web::apps::{
    self, AppError, Badges as AppBadges, Ceremonies as AppCeremonies, Moment, Sealed, SiteApps,
    Subject, Verification, Wallets as AppWallets,
};
use saas_web::passkeys::Passkeys;

/// What the apps read, filled when the money layer has started.
#[derive(Default)]
pub struct Shared {
    badges: OnceLock<Arc<crate::badges::Badges>>,
    wallets: RwLock<HashMap<String, Arc<saas_wallet::Wallets>>>,
    ceremonies: OnceLock<saas_badge::ceremony::Ceremonies>,
}

/// The one [`Shared`] of this process.
pub fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(Shared::default)
}

impl Shared {
    /// The badges and the verifier.
    pub fn set_badges(&self, badges: Arc<crate::badges::Badges>) {
        let _ = self.badges.set(badges);
    }

    /// `site`'s wallets (face's site id: `kilele`).
    pub fn add_wallets(&self, site: &str, wallets: Arc<saas_wallet::Wallets>) {
        self.wallets
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(site.to_owned(), wallets);
    }

    /// `site`'s wallets (face's site id: `kilele`), once the money layer has started them.
    pub fn wallets(&self, site: &str) -> Option<Arc<saas_wallet::Wallets>> {
        self.wallets
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(site)
            .cloned()
    }

    /// The owners' moments.
    pub fn set_ceremonies(&self, ceremonies: saas_badge::ceremony::Ceremonies) {
        let _ = self.ceremonies.set(ceremonies);
    }
}

fn failed(e: impl std::fmt::Display) -> AppError {
    AppError::Refused(e.to_string())
}

/// Same JSON, face's type.
fn retype<A: serde::Serialize, B: serde::de::DeserializeOwned>(a: &A) -> Result<B, AppError> {
    serde_json::to_value(a)
        .and_then(serde_json::from_value)
        .map_err(failed)
}

struct NodeBadges;

#[async_trait]
impl AppBadges for NodeBadges {
    async fn badges(&self, site: &str) -> Result<Vec<Badge>, AppError> {
        let badges = shared().badges.get().ok_or(AppError::NotConnected)?;
        let state = match site {
            "agentrix" => &badges.agentrix,
            "kilele" => &badges.kilele,
            _ => return Err(AppError::NotFound),
        };
        Ok(state
            .identity
            .agents
            .iter()
            .filter_map(|a| state.badge(&a.role))
            .collect())
    }

    async fn verify(&self, did: &str) -> Result<Verification, AppError> {
        let badges = shared().badges.get().ok_or(AppError::NotConnected)?;
        let report = badges.verifier.verify(did).await;
        // saas-badge writes both names of a field (`verified` and `ok`, `check` and `name`);
        // face's type reads either, but not both at once.
        let mut value = serde_json::to_value(&report).map_err(failed)?;
        if let Some(map) = value.as_object_mut() {
            map.remove("ok");
            if let Some(checks) = map.get_mut("checks").and_then(|c| c.as_array_mut()) {
                for check in checks {
                    if let Some(c) = check.as_object_mut()
                        && c.contains_key("check")
                    {
                        c.remove("name");
                    }
                }
            }
        }
        let verification: Verification = serde_json::from_value(value).map_err(failed)?;
        Ok(verification.settled())
    }
}

/// The wallets as face's apps and activity page read them, by face's site id (`agentrix`,
/// `kilele`): not connected until the money layer has started them.
pub fn wallets() -> Arc<dyn AppWallets> {
    Arc::new(NodeWallets)
}

struct NodeWallets;

#[async_trait]
impl AppWallets for NodeWallets {
    async fn wallet(&self, site: &str) -> Result<WalletFeed, AppError> {
        let wallets = shared()
            .wallets
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(site)
            .cloned()
            .ok_or(AppError::NotConnected)?;
        let cashier = crate::samples::cashier(wallets.company()).map_err(failed)?;
        wallets.feed(&cashier).await.map_err(failed)
    }
}

struct NodeCeremonies;

fn ceremony_error(e: CeremonyError) -> AppError {
    match e {
        CeremonyError::NotFound => AppError::NotFound,
        CeremonyError::Changed => AppError::Changed,
        CeremonyError::Expired => AppError::Expired,
        other => AppError::Refused(other.to_string()),
    }
}

#[async_trait]
impl AppCeremonies for NodeCeremonies {
    async fn subject(&self, site: &str, moment: Moment, id: &str) -> Result<Subject, AppError> {
        let ceremony = shared()
            .ceremonies
            .get()
            .ok_or(AppError::NotConnected)?
            .get(site, moment.as_str())
            .ok_or(AppError::NotFound)?;
        retype(&ceremony.subject(id).await.map_err(ceremony_error)?)
    }

    async fn seal(
        &self,
        site: &str,
        moment: Moment,
        id: &str,
        evidence: serde_json::Value,
    ) -> Result<Sealed, AppError> {
        let ceremony = shared()
            .ceremonies
            .get()
            .ok_or(AppError::NotConnected)?
            .get(site, moment.as_str())
            .ok_or(AppError::NotFound)?;
        let evidence: PasskeyEvidence = serde_json::from_value(evidence).map_err(failed)?;
        retype(&ceremony.seal(id, evidence).await.map_err(ceremony_error)?)
    }
}

/// Kilele Bakery's apps (face's site `kilele`), over the adapters.
pub fn kilele_apps(owner: &str, passkeys_file: PathBuf) -> anyhow::Result<SiteApps> {
    Ok(SiteApps {
        site: "kilele".into(),
        name: "Kilele Bakery".into(),
        owner: owner.into(),
        base: String::new(),
        ceremonies: Arc::new(NodeCeremonies),
        badges: Arc::new(NodeBadges),
        wallet: Arc::new(NodeWallets),
        paypal_link: crate::platform::PAYPAL_LINK_KILELE.into(),
        passkeys: Arc::new(Passkeys::at(passkeys_file)?),
        // The shell checks the owner before the apps run.
        access: Access {
            owner_token: None,
            open: false,
        },
        fragment: false,
    })
}

/// The three apps for node's shell: Kilele Bakery's sample site (`kilele-bakery-sample`) has
/// them; `console_host` is the console's host as browsers see it (the passkeys' RP ID).
pub fn console_apps(
    apps: SiteApps,
    console_host: &str,
) -> Vec<Arc<dyn agentrix_shell::apps::ConsoleApp>> {
    saas_web::os::console_apps(
        HashMap::from([(crate::platform::KILELE_SAMPLE.to_owned(), apps)]),
        console_host,
    )
}

/// Face's public badge pages (`/agents`, `/agents/check`, `/agents/check-token`).
pub fn public_router(apps: SiteApps) -> axum::Router {
    apps::public_router(apps)
}

/// A company's signed audit log as saas-badge's audit sink: the PayPal link's records go there.
pub struct CompanyAuditSink(pub Arc<saas_company::audit::SignedAudit>);

#[async_trait]
impl saas_badge::binding::AuditSink for CompanyAuditSink {
    async fn append(&self, kind: &str, record: &serde_json::Value) -> anyhow::Result<u64> {
        let written = self
            .0
            .record(
                adk_core::audit_log::AuditRecordDraft::new(
                    adk_core::audit_log::AuditRecordKind::Other(kind.to_owned()),
                    kind,
                )
                .args_digest(self.0.digest(record))
                .ext("record", record.clone()),
            )
            .await?;
        Ok(written.seq)
    }
}
