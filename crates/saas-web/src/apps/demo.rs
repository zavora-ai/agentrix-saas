//! Stand-ins for the apps' seams on a node where settle, the badge service or the wallet
//! aren't connected. Each says so on the page: nothing here claims a seal, a verification or a
//! reconciliation it didn't do.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saas_core::Books;
use saas_core::books::{DocumentFilter, DocumentRef, PaymentDirection, PaymentFilter};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use chrono::{Duration, Utc};
use saas_core::Money;
use saas_core::identity::{Badge, BadgeSite, BadgeStatus, Sponsor, site_did};
use saas_core::wallet::{
    Authority, BooksLink, MatchState, ThreeWayMatch, WalletCause, WalletCounterparty, WalletEntry,
    WalletEntryKind, WalletFeed, WalletSource,
};
use saas_core::{AgentId, CompanyId, PayPalRef};

use super::{
    AppError, Badges, Ceremonies, Check, Failure, Moment, Sealed, Subject, Verification, Wallets,
};

/// Nothing behind the app on this node: every call says so.
pub struct NotConnected;

#[async_trait]
impl Ceremonies for NotConnected {
    async fn subject(&self, _site: &str, _moment: Moment, _id: &str) -> Result<Subject, AppError> {
        Err(AppError::NotConnected)
    }
    async fn seal(
        &self,
        _site: &str,
        _moment: Moment,
        _id: &str,
        _evidence: Value,
    ) -> Result<Sealed, AppError> {
        Err(AppError::NotConnected)
    }
    async fn pending(&self, _site: &str) -> Result<Vec<Subject>, AppError> {
        Err(AppError::NotConnected)
    }
}

#[async_trait]
impl Wallets for NotConnected {
    async fn wallet(&self, _site: &str) -> Result<WalletFeed, AppError> {
        Err(AppError::NotConnected)
    }
}

/// The line every demo subject ends with.
pub const DEMO_LINE: &str = "Demo subject: settle isn't connected on this node, so this is sealed in memory only, not with an owner key or in the audit log.";

fn fingerprint(v: &Value) -> String {
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(serde_json::to_vec(v).unwrap_or_default()))
    )
}

fn subject(
    site: &str,
    moment: Moment,
    id: &str,
    title: String,
    lines: Vec<String>,
    reason: Option<&str>,
) -> Subject {
    let mut lines = lines;
    lines.push(DEMO_LINE.into());
    let body = json!({"site": site, "moment": moment, "id": id, "title": title, "lines": lines, "reason": reason});
    Subject {
        moment,
        id: id.to_owned(),
        site: site.to_owned(),
        state: "pending".into(),
        title,
        lines,
        reason: reason.map(str::to_owned),
        fingerprint: fingerprint(&body),
        expires_at: None,
    }
}

/// The three moments for each site, in memory.
#[derive(Default)]
pub struct DemoCeremonies {
    subjects: Mutex<Vec<(Subject, Option<Value>)>>,
}

impl DemoCeremonies {
    /// One of each moment for every `(site, name, owner)`.
    pub fn story(sites: &[(&str, &str, &str)]) -> Self {
        let mut all = Vec::new();
        for (site, name, owner) in sites {
            let site_up = site.to_ascii_uppercase();
            all.push(subject(
                site,
                Moment::LinkPaypal,
                &format!("bind-{site}"),
                format!("Link {name}'s PayPal to its Cashier"),
                vec![
                    format!("PayPal merchant id DEMO-MERCHANT-{site_up}, as PayPal returned it"),
                    format!("Vault id DEMO-VAULT-{site_up}"),
                    "Agents allowed: Cashier (Receivables and Payables desks)".into(),
                ],
                None,
            ));
            all.push(subject(
                site,
                Moment::Mandate,
                &format!("mandate-{site}-cashier"),
                format!("Sign {owner}'s mandate for the Cashier"),
                vec![
                    "Up to $200.00 a payment and $200.00 a day".into(),
                    "Payees on file only; a new payee or changed details needs your passkey".into(),
                    "Good for 60 days".into(),
                ],
                None,
            ));
            all.push(subject(
                site,
                Moment::StepUp,
                &format!("stepup-{site}-new-payee"),
                "Pay a new payee: Unga Millers, $420.00".into(),
                vec![
                    "Unga Millers isn't a payee on file".into(),
                    "PayPal Payouts to the address on the bill".into(),
                ],
                Some("new-payee"),
            ));
        }
        Self {
            subjects: Mutex::new(all.into_iter().map(|s| (s, None)).collect()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(Subject, Option<Value>)>> {
        self.subjects.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl Ceremonies for DemoCeremonies {
    async fn subject(&self, site: &str, moment: Moment, id: &str) -> Result<Subject, AppError> {
        self.lock()
            .iter()
            .find(|(s, _)| s.site == site && s.moment == moment && s.id == id)
            .map(|(s, _)| s.clone())
            .ok_or(AppError::NotFound)
    }

    async fn seal(
        &self,
        site: &str,
        moment: Moment,
        id: &str,
        evidence: Value,
    ) -> Result<Sealed, AppError> {
        let mut all = self.lock();
        let (s, record) = all
            .iter_mut()
            .find(|(s, _)| s.site == site && s.moment == moment && s.id == id)
            .ok_or(AppError::NotFound)?;
        if s.state != "pending" {
            return Err(AppError::Refused(format!("Already {}.", s.state)));
        }
        if evidence.get("fingerprint").and_then(Value::as_str) != Some(s.fingerprint.as_str()) {
            return Err(AppError::Changed);
        }
        s.state = "sealed".into();
        let sealed = json!({"subject": s.clone(), "evidence": evidence, "sealedWith": "nothing: demo store on this node"});
        *record = Some(sealed.clone());
        Ok(Sealed {
            sealed: true,
            record: sealed,
            audit_seq: None,
        })
    }

    async fn pending(&self, site: &str) -> Result<Vec<Subject>, AppError> {
        Ok(self
            .lock()
            .iter()
            .filter(|(s, _)| s.site == site)
            .map(|(s, _)| s.clone())
            .collect())
    }
}

/// A demo site: its host, name, owner and agents.
#[derive(Debug, Clone)]
pub struct DemoSite {
    /// Its host (`kilele.money.agentrix.website`).
    pub host: String,
    /// "Kilele Bakery".
    pub name: String,
    /// "Wanjiru".
    pub owner: String,
    /// Its agents: (role, name), `("cashier", "Cashier")`.
    pub agents: Vec<(String, String)>,
}

/// Badges built on this node from the story (unsigned, unpublished, and said so), and a
/// verifier that says nothing is connected.
pub struct DemoBadges {
    sites: HashMap<String, DemoSite>,
}

/// The company id of a site id: `agentrix`, `kilele` (Kilele Bakery), or the id itself.
pub fn company_of(site: &str) -> CompanyId {
    match site {
        "kilele" => CompanyId::kilele(),
        "agentrix" => CompanyId::agentrix(),
        other => CompanyId::new(other).unwrap_or_else(|_| CompanyId::agentrix()),
    }
}

impl DemoBadges {
    /// `sites`: site id → its host, name, owner and roles.
    pub fn new(sites: HashMap<String, DemoSite>) -> Self {
        Self { sites }
    }

    /// `did:web:<host>:agents:<role>` (saas-core's form: a port is percent-encoded).
    pub fn did(host: &str, role: &str) -> String {
        Badge::agent_did(host, role)
    }
}

#[async_trait]
impl Badges for DemoBadges {
    async fn badges(&self, site: &str) -> Result<Vec<Badge>, AppError> {
        let s = self.sites.get(site).ok_or(AppError::NotFound)?;
        let company = company_of(site);
        let now = Utc::now();
        let mut out = Vec::new();
        for (role, name) in &s.agents {
            let agent = AgentId::new(company.clone(), role.clone())
                .map_err(|e| AppError::Refused(e.to_string()))?;
            // No keys, binding, mandate, wallet checkpoint or voucher: none exist on this node,
            // and the badge says so by leaving them out.
            out.push(Badge {
                id: Self::did(&s.host, role),
                also_known_as: vec![Badge::urn(&agent)],
                role: role.clone(),
                name: name.clone(),
                site: BadgeSite {
                    did: site_did(&s.host),
                    host: s.host.clone(),
                    name: s.name.clone(),
                    company: company.clone(),
                    sample: site != "agentrix",
                },
                sponsor: Sponsor {
                    owner: format!("{site}-owner"),
                    name: Some(s.owner.clone()),
                    passkey: None,
                    approved_at: None,
                },
                keys: Vec::new(),
                paypal: None,
                mandate: None,
                wallet: None,
                voucher: None,
                credentials: None,
                status: BadgeStatus::Active,
                issued_at: now,
                expires_at: now + Duration::days(30),
            });
        }
        Ok(out)
    }

    async fn verify(&self, did: &str) -> Result<Verification, AppError> {
        let known = self.sites.values().any(|s| {
            s.agents
                .iter()
                .any(|(role, _)| Self::did(&s.host, role) == did)
        });
        let first = Check {
            check: "is one of this node's agents".into(),
            ok: known,
            detail: if known {
                "listed among the site's agents".into()
            } else {
                "no agent here has this DID".into()
            },
        };
        let (checks, failure) = if known {
            let check = "did.json resolves from the site's own domain";
            (
                vec![
                    first,
                    Check {
                        check: check.into(),
                        ok: false,
                        detail: "no badge service on this node".into(),
                    },
                ],
                Failure {
                    check: check.into(),
                    message: "No badge service is connected on this node, so nothing is verified."
                        .into(),
                },
            )
        } else {
            (
                vec![first],
                Failure {
                    check: "is one of this node's agents".into(),
                    message: "This DID isn't one of this node's agents.".into(),
                },
            )
        };
        Ok(Verification {
            did: did.to_owned(),
            verified: false,
            checks,
            failure: Some(failure),
            badge: Value::Null,
            checked_at: None,
        })
    }

    fn note(&self) -> Option<String> {
        Some("Demo badges: built on this node from the story, not signed with the site's owner key and not published at its domain.".into())
    }
}

/// A wallet read from a site's books: every payment with its PayPal id. Not the Cashier's
/// signed wallet log, and not read back from PayPal (no PayPal status, no checkpoint).
pub struct BooksWallet {
    books: HashMap<String, Arc<dyn Books>>,
}

impl BooksWallet {
    /// `books`: site id → its books.
    pub fn new(books: HashMap<String, Arc<dyn Books>>) -> Self {
        Self { books }
    }
}

#[async_trait]
impl Wallets for BooksWallet {
    async fn wallet(&self, site: &str) -> Result<WalletFeed, AppError> {
        let books = self.books.get(site).ok_or(AppError::NotFound)?;
        let fail =
            |e: saas_core::BooksError| AppError::Refused(format!("The books didn't answer: {e}"));
        let bad = |e: saas_core::IdError| AppError::Refused(e.to_string());
        let company = company_of(site);
        let cashier = AgentId::new(company.clone(), "cashier").map_err(bad)?;
        let receivables = AgentId::new(company.clone(), "receivables").map_err(bad)?;
        let payables = AgentId::new(company.clone(), "payables").map_err(bad)?;
        let customers = books.customers().await.map_err(fail)?;
        let vendors = books.vendors().await.map_err(fail)?;
        let invoices = books
            .invoices(&DocumentFilter::default())
            .await
            .map_err(fail)?;
        let bills = books
            .bills(&DocumentFilter::default())
            .await
            .map_err(fail)?;
        let mut payments = books
            .payments(&PaymentFilter::default())
            .await
            .map_err(fail)?;
        payments.sort_by_key(|p| p.date);

        let name_of = |party: &DocumentRef| match party {
            DocumentRef::Customer(id) => customers
                .iter()
                .find(|c| &c.id == id)
                .map(|c| c.counterparty.name.clone()),
            DocumentRef::Vendor(id) => vendors
                .iter()
                .find(|v| &v.id == id)
                .map(|v| v.counterparty.name.clone()),
            _ => None,
        };
        let cause_of = |doc: &DocumentRef| match doc {
            DocumentRef::Invoice(id) => Some(WalletCause::Invoice {
                invoice: id.clone(),
                number: invoices
                    .iter()
                    .find(|i| &i.id == id)
                    .and_then(|i| i.number.clone()),
            }),
            DocumentRef::Bill(id) => Some(WalletCause::Bill {
                bill: id.clone(),
                number: bills
                    .iter()
                    .find(|b| &b.id == id)
                    .and_then(|b| b.vendor_reference.clone().or_else(|| b.number.clone())),
            }),
            _ => None,
        };

        let mut entries = Vec::new();
        let mut matches = Vec::new();
        for p in &payments {
            let Some(paypal) = p.reference.as_deref().and_then(PayPalRef::from_key) else {
                continue;
            };
            let kind = match (&paypal, p.direction) {
                (PayPalRef::Refund(_), _) => WalletEntryKind::Refund,
                (PayPalRef::SubscriptionSale(_), _) => WalletEntryKind::Subscription,
                (_, PaymentDirection::Received) => WalletEntryKind::Capture,
                (_, PaymentDirection::Paid) => WalletEntryKind::Payout,
            };
            let received = p.direction == PaymentDirection::Received;
            // The feed signs amounts: money out of the wallet is negative.
            let amount = if received {
                p.amount
            } else {
                Money {
                    minor: -p.amount.minor,
                    ..p.amount
                }
            };
            let seq = entries.len() as u64;
            entries.push(WalletEntry {
                wallet: cashier.clone(),
                seq,
                at: p.date.and_hms_opt(12, 0, 0).unwrap_or_default().and_utc(),
                kind,
                amount,
                paypal: paypal.clone(),
                paypal_status: "not read back".into(),
                counterparty: name_of(&p.party).map(|name| WalletCounterparty {
                    name,
                    email: None,
                    company: None,
                }),
                cause: p.applied.iter().find_map(|(d, _)| cause_of(d)),
                acting: if received {
                    receivables.clone()
                } else {
                    payables.clone()
                },
                authority: if received {
                    Authority::PayerApproved
                } else {
                    Authority::Mandate {
                        mandate_digest: String::new(),
                        cart_mandate_digest: None,
                    }
                },
                source: WalletSource::ReadBack {
                    call: "the books on this node (demo)".into(),
                },
            });
            matches.push(ThreeWayMatch {
                seq,
                kind,
                amount,
                paypal,
                paypal_status: None,
                paypal_amount: None,
                books: BooksLink {
                    payment: Some(p.id.clone()),
                    ..BooksLink::default()
                },
                books_amount: Some(amount),
                state: MatchState::Matched,
            });
        }
        Ok(WalletFeed {
            wallet: cashier,
            checkpoint: None,
            history: Vec::new(),
            entries,
            matches,
        })
    }

    fn note(&self) -> Option<String> {
        Some("Read from the books on this node: not the Cashier's signed wallet log, and not read back from PayPal.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn demo_moments_seal_only_their_own_fingerprint() {
        let c = DemoCeremonies::story(&[("kilele", "Kilele Bakery", "Wanjiru")]);
        let pending = c.pending("kilele").await.unwrap();
        assert_eq!(pending.len(), 3);
        let s = c
            .subject("kilele", Moment::LinkPaypal, "bind-kilele")
            .await
            .unwrap();
        assert!(s.lines.last().unwrap().starts_with("Demo subject"));
        assert_eq!(
            c.seal(
                "kilele",
                Moment::LinkPaypal,
                "bind-kilele",
                json!({"fingerprint": "sha256:other"})
            )
            .await,
            Err(AppError::Changed)
        );
        let sealed = c
            .seal(
                "kilele",
                Moment::LinkPaypal,
                "bind-kilele",
                json!({"fingerprint": s.fingerprint}),
            )
            .await
            .unwrap();
        assert!(sealed.sealed && sealed.audit_seq.is_none());
        assert!(matches!(
            c.seal(
                "kilele",
                Moment::LinkPaypal,
                "bind-kilele",
                json!({"fingerprint": s.fingerprint})
            )
            .await,
            Err(AppError::Refused(_))
        ));
    }

    #[tokio::test]
    async fn the_demo_verifier_never_says_verified() {
        let p = DemoBadges::new(HashMap::from([(
            "kilele".to_owned(),
            DemoSite {
                host: "kilele.money.localhost:18183".into(),
                name: "Kilele Bakery".into(),
                owner: "Wanjiru".into(),
                agents: vec![("cashier".into(), "Cashier".into())],
            },
        )]));
        let badges = p.badges("kilele").await.unwrap();
        assert_eq!(
            badges[0].id,
            "did:web:kilele.money.localhost%3A18183:agents:cashier"
        );
        assert_eq!(badges[0].site.did, "did:web:kilele.money.localhost%3A18183");
        assert!(badges[0].keys.is_empty() && badges[0].paypal.is_none());
        assert_eq!(
            badges[0].also_known_as,
            vec!["urn:adk:agent:kilele-bakery/cashier".to_owned()]
        );
        assert_eq!(badges[0].sponsor.name.as_deref(), Some("Wanjiru"));
        let v = p.verify(&badges[0].id).await.unwrap();
        assert!(!v.verified);
        assert!(v.failure.unwrap().message.contains("No badge service"));
        assert!(
            !p.verify("did:web:evil.example:agents:cashier")
                .await
                .unwrap()
                .verified
        );
        assert!(p.note().unwrap().contains("not signed"));
    }
}
