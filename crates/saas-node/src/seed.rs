//! What the node needs to know about the books it starts on: the vendors the founders'
//! default mandates may pay, and Kilele as Agentrix's customer.
//!
//! With `--books fake` the books are saas-web's story (`saas_web::seed`), posted into
//! saas-core's FakeBooks through the Books contract; with `--books era` they are Zavora ERA's.
//! Either way [`Story::read`] only reads them; [`ensure_cast`] adds the story's one partner
//! if it is missing.

use saas_core::books::{BillId, CustomerId, DocumentFilter, InvoiceId, VendorId};
use saas_core::{Books, CompanyId};

/// What the books hold that the node starts from.
#[derive(Debug, Clone, Default)]
pub struct Story {
    /// Agentrix's vendors (the default mandate's payees).
    pub agentrix_payees: Vec<VendorId>,
    /// Kilele's vendors.
    pub kilele_payees: Vec<VendorId>,
    /// Kilele, as Agentrix's customer.
    pub kilele_customer: Option<CustomerId>,
    /// Agentrix's oldest open invoice to Kilele.
    pub kilele_invoice: Option<InvoiceId>,
    /// Kilele's oldest open bill from Agentrix.
    pub kilele_bill: Option<BillId>,
}

impl Story {
    /// Reads both companies' books.
    pub async fn read(agentrix: &dyn Books, kilele: &dyn Books) -> anyhow::Result<Self> {
        let open = DocumentFilter {
            party: None,
            open_only: true,
            reference: None,
        };
        let agentrix_payees = agentrix
            .vendors()
            .await?
            .into_iter()
            .map(|v| v.id)
            .collect();
        let kilele_vendors = kilele.vendors().await?;
        let agentrix_as_vendor = kilele_vendors
            .iter()
            .find(|v| v.counterparty.company == Some(CompanyId::agentrix()))
            .map(|v| v.id.clone());
        let kilele_customer = agentrix
            .customers()
            .await?
            .into_iter()
            .find(|c| c.counterparty.company == Some(CompanyId::kilele()))
            .map(|c| c.id);
        let kilele_invoice = match &kilele_customer {
            Some(c) => agentrix
                .invoices(&open)
                .await?
                .into_iter()
                .filter(|i| &i.customer == c)
                .min_by_key(|i| i.due_date)
                .map(|i| i.id),
            None => None,
        };
        let kilele_bill = match &agentrix_as_vendor {
            Some(v) => kilele
                .bills(&open)
                .await?
                .into_iter()
                .filter(|b| &b.vendor == v)
                .min_by_key(|b| b.due_date)
                .map(|b| b.id),
            None => None,
        };
        Ok(Self {
            agentrix_payees,
            kilele_payees: kilele_vendors.into_iter().map(|v| v.id).collect(),
            kilele_customer,
            kilele_invoice,
            kilele_bill,
        })
    }
}

/// The partner Payables pays by PayPal Payouts in the story's scene (d) and the attacks.
pub const PARTNER: &str = "Savannah Growth Partners (referral)";

/// Makes sure Agentrix's books hold the story's partner, with `partner_email` (a PayPal sandbox
/// account, `PAYPAL_PARTNER_EMAIL`) as its PayPal payee. Run before the mandates are drawn up,
/// so the partner is one of the founder's payees.
pub async fn ensure_cast(agentrix: &dyn Books, partner_email: Option<&str>) -> anyhow::Result<()> {
    let email = partner_email.unwrap_or("payouts@savannah-growth.example");
    let mut who = saas_core::books::Counterparty::usd(PARTNER, Some(email));
    who.paypal_email = Some(email.to_owned());
    let vendor = agentrix.ensure_vendor(&who).await?;
    // The model providers Agentrix's spend bills name, with their Payout emails
    // (`SAAS_PAYEE_ANTHROPIC`, `SAAS_PAYEE_VULTR`): in the books before the founder's mandate is
    // drawn up, so its payees include them.
    let payees = crate::hourly::payees_from_env();
    for provider in ["anthropic", "vultr"] {
        agentrix
            .ensure_vendor(&saas_core::spend::vendor_for(
                provider,
                payees.get(provider).cloned(),
            ))
            .await?;
    }
    if vendor.counterparty.paypal_email.is_none() {
        tracing::warn!(
            "{PARTNER} is in the books without a PayPal payee; scene (d) has nobody to pay"
        );
    }
    Ok(())
}

/// Makes sure each company's books hold its payroll vendor, with the payroll account as its
/// PayPal payee ([`crate::billing::payroll_vendor`]). Run before the mandates are drawn up, so
/// payroll is one of each founder's payees and a payroll run inside the mandate needs no one.
pub async fn ensure_payroll(
    agentrix: &dyn Books,
    kilele: &dyn Books,
    env: &crate::config::EnvFile,
) -> anyhow::Result<()> {
    for (books, company) in [
        (agentrix, CompanyId::agentrix()),
        (kilele, CompanyId::kilele()),
    ] {
        let vendor = books
            .ensure_vendor(&crate::billing::payroll_vendor(env, &company))
            .await?;
        if vendor.counterparty.paypal_email.is_none() {
            tracing::warn!(company = %company, "the payroll vendor has no PayPal payee; its payroll bills can't be paid");
        }
    }
    Ok(())
}
