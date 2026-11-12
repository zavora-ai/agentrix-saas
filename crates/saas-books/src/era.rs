//! [`EraBooks`]: one company's books in Zavora ERA (hosted).

use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use reqwest::Method;
use saas_core::books::*;
use saas_core::{BooksError, CompanyId, Counterparty, Currency, Money};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::map::{self, R, dec, s};
use crate::rest::Rest;

#[cfg(feature = "mcp")]
use crate::mcp::{ErpMcp, McpConfig};

/// The company's chart, as the seed (`deploy/saas/era/seed.py`) leaves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountMap {
    /// GL of the "PayPal clearing" bank account.
    pub clearing: String,
    /// Trade debtors.
    pub receivables: String,
    /// Trade creditors.
    pub payables: String,
    /// Plan revenue.
    pub revenue: String,
    /// PayPal's fees.
    pub fees: String,
    /// Sales returns: refunds of paid invoices.
    pub sales_returns: String,
}

impl Default for AccountMap {
    fn default() -> Self {
        Self {
            clearing: "1050".into(),
            receivables: "1200".into(),
            payables: "3010".into(),
            revenue: "5100".into(),
            fees: "8075".into(),
            sales_returns: "5400".into(),
        }
    }
}

/// How to reach one company's books. Its `Debug` leaves the password out.
#[derive(Clone)]
pub struct EraConfig {
    /// The company.
    pub company: CompanyId,
    /// ERA's base URL, for example `http://127.0.0.1:18180`.
    pub base_url: String,
    /// The company's ERA login.
    pub email: String,
    /// Its password.
    pub password: String,
    /// The chart.
    pub accounts: AccountMap,
    /// mcp-erp, for ledger writes over MCP. `None` writes over REST.
    #[cfg(feature = "mcp")]
    pub mcp: Option<McpConfig>,
}

impl std::fmt::Debug for EraConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EraConfig")
            .field("company", &self.company)
            .field("base_url", &self.base_url)
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for EraBooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EraBooks")
            .field("company", &self.company)
            .field("via", &self.via())
            .finish()
    }
}

impl EraConfig {
    /// The company's login from the seed's tenants file (`{"agentrix": {"email", "password"}, …}`).
    pub fn from_tenants_file(
        path: &Path,
        company: CompanyId,
        base_url: &str,
    ) -> Result<Self, BooksError> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            BooksError::Unavailable(format!("ERA tenants file {}: {e}", path.display()))
        })?;
        let all: Value = serde_json::from_str(&text)
            .map_err(|e| BooksError::Invalid(format!("ERA tenants file: {e}")))?;
        let t = &all[company.as_str()];
        let (Some(email), Some(password)) = (t["email"].as_str(), t["password"].as_str()) else {
            return Err(BooksError::NotFound {
                what: "ERA tenant",
                id: company.to_string(),
            });
        };
        Ok(Self {
            company,
            base_url: base_url.to_owned(),
            email: email.to_owned(),
            password: password.to_owned(),
            accounts: AccountMap::default(),
            #[cfg(feature = "mcp")]
            mcp: McpConfig::from_env(),
        })
    }

    /// `SAAS_ERA_URL` (default `http://127.0.0.1:18180`) and `SAAS_ERA_TENANTS_FILE` (default
    /// `~/.config/agentrix-saas/era/tenants.json`); mcp-erp from `MCP_ERP_BIN`/`MCP_ERP_DIR`.
    pub fn from_env(company: CompanyId) -> Result<Self, BooksError> {
        let url = std::env::var("SAAS_ERA_URL").unwrap_or_else(|_| "http://127.0.0.1:18180".into());
        let file = std::env::var_os("SAAS_ERA_TENANTS_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default();
                home.join(".config/agentrix-saas/era/tenants.json")
            });
        Self::from_tenants_file(&file, company, &url)
    }
}

/// [`Books`] over Zavora ERA (hosted), as one company.
///
/// - **Ledger writes** (post an invoice, draft and post a bill, record a payment, post a
///   journal) go through the open-source `mcp-erp` MCP server, reached with adk-rust's MCP
///   client, when [`EraConfig::mcp`] is set; over ERA's REST API otherwise.
/// - **Everything else** goes over ERA's REST API: parties, price list, invoice drafts (mcp-erp's
///   tool has no dates, notes or VAT treatment), recurring invoices, CRM, ageing, credit limits,
///   and every read.
///
/// How the contract maps onto ERA:
///
/// | Contract | ERA |
/// |---|---|
/// | a document's `reference` | a `[ref] …` line in its notes; a payment's own `reference`; a journal's unique `reference` |
/// | `submit_invoice` | no-op: ERA invoices have no approval step |
/// | `post_bill` | approve, then post |
/// | `record_refund` | a journal, Dr sales returns, Cr PayPal clearing (ERA gives a paid invoice no credit note) |
/// | `stop_recurring_invoice` | ends the schedule on its last run; deletes one that never ran |
/// | `raise_due_recurring` | posts the drafts ERA's hourly scheduler raised on or before the date |
/// | `open_ticket` | through ERA's customer portal, which serves only the tenant named by ERA's `ENTITY_ID` (Agentrix) |
///
/// Every line is out of scope for VAT; lines without a product use the seeded "General services"
/// (invoices) or "General expense" (bills).
pub struct EraBooks {
    company: CompanyId,
    rest: Rest,
    password_digest: [u8; 32],
    accounts: AccountMap,
    clearing_bank: String,
    service_product: String,
    expense_product: String,
    #[cfg(feature = "mcp")]
    mcp: Option<ErpMcp>,
}

impl EraBooks {
    /// Logs in, finds the seeded PayPal clearing account and default products, and starts
    /// mcp-erp when configured.
    pub async fn connect(cfg: EraConfig) -> Result<Self, BooksError> {
        let rest = Rest::new(&cfg.base_url, cfg.email.clone(), cfg.password.clone());
        rest.login().await?;
        let banks = rest.get("bank-accounts").await?;
        let clearing_bank = banks
            .as_array()
            .into_iter()
            .flatten()
            .find(|b| {
                b["gl_account"].as_str() == Some(cfg.accounts.clearing.as_str())
                    && b["is_active"].as_bool() != Some(false)
            })
            .map(|b| s(&b["id"]))
            .ok_or_else(|| BooksError::NotFound {
                what: "PayPal clearing bank account (run era.sh seed)",
                id: cfg.accounts.clearing.clone(),
            })?;
        let products = rest.get("products").await?;
        let product = |name: &str| {
            products
                .as_array()
                .into_iter()
                .flatten()
                .find(|p| p["name"].as_str() == Some(name))
                .map(|p| s(&p["id"]))
                .ok_or_else(|| BooksError::NotFound {
                    what: "product (run era.sh seed)",
                    id: name.to_owned(),
                })
        };
        let service_product = product("General services")?;
        let expense_product = product("General expense")?;
        #[cfg(feature = "mcp")]
        let mcp = match &cfg.mcp {
            Some(m) => Some(ErpMcp::spawn(m, &cfg.base_url, &cfg.email, &cfg.password).await?),
            None => None,
        };
        Ok(Self {
            company: cfg.company,
            rest,
            password_digest: Sha256::digest(cfg.password.as_bytes()).into(),
            accounts: cfg.accounts,
            clearing_bank,
            service_product,
            expense_product,
            #[cfg(feature = "mcp")]
            mcp,
        })
    }

    /// `"mcp-erp"` when ledger writes go over MCP, `"rest"` otherwise.
    pub fn via(&self) -> &'static str {
        #[cfg(feature = "mcp")]
        if self.mcp.is_some() {
            return "mcp-erp";
        }
        "rest"
    }

    /// The ERA tenant (entity) id these books are.
    pub async fn entity_id(&self) -> Result<String, BooksError> {
        Ok(self.rest.session().await?.entity_id)
    }

    /// A ledger write: the mcp-erp tool when MCP is on, else the REST call.
    async fn write(
        &self,
        tool: &str,
        tool_args: Value,
        method: Method,
        path: &str,
        rest_body: Value,
    ) -> R<Value> {
        #[cfg(feature = "mcp")]
        if let Some(m) = &self.mcp {
            return m.call(tool, tool_args).await;
        }
        let _ = (tool, tool_args);
        self.rest.send(method, path, Some(&rest_body)).await
    }

    async fn invoice_rows(&self) -> R<Vec<Value>> {
        let rows = self.rest.get_all("invoices").await?;
        Ok(rows
            .into_iter()
            .filter(|r| r["invoice_type"].as_str().is_none_or(|t| t == "invoice"))
            .collect())
    }

    /// What each customer owes on open invoices.
    async fn customer_balances(&self) -> R<Vec<(String, Money)>> {
        let mut out: Vec<(String, Money)> = Vec::new();
        for row in self.invoice_rows().await? {
            let inv = map::invoice(&row)?;
            if !inv.status.is_open() {
                continue;
            }
            match out.iter_mut().find(|(c, _)| *c == inv.customer.0) {
                Some((_, m)) => *m = m.checked_add(inv.balance_due)?,
                None => out.push((inv.customer.0.clone(), inv.balance_due)),
            }
        }
        Ok(out)
    }

    async fn vendor_balances(&self) -> R<Vec<(String, Money)>> {
        let mut out: Vec<(String, Money)> = Vec::new();
        for row in self.rest.get_all("bills").await? {
            let b = map::bill(&row)?;
            if !b.status.is_open() {
                continue;
            }
            match out.iter_mut().find(|(v, _)| *v == b.vendor.0) {
                Some((_, m)) => *m = m.checked_add(b.balance_due)?,
                None => out.push((b.vendor.0.clone(), b.balance_due)),
            }
        }
        Ok(out)
    }

    fn balance_of(balances: &[(String, Money)], id: &str, currency: Currency) -> Money {
        balances
            .iter()
            .find(|(p, _)| p == id)
            .map(|(_, m)| *m)
            .unwrap_or(Money::zero(currency))
    }

    async fn payment_by_reference(&self, reference: &str) -> R<Option<Payment>> {
        for row in self.rest.get_all("payments").await? {
            if row["reference"].as_str() == Some(reference) {
                return Ok(Some(map::payment(&row)?));
            }
        }
        Ok(None)
    }

    async fn journal_by_reference(&self, reference: &str) -> R<Option<JournalEntry>> {
        for row in self.rest.get_all("journal-entries").await? {
            if row["reference"].as_str() == Some(reference) {
                let full = self
                    .rest
                    .get(&format!("journal-entries/{}", s(&row["id"])))
                    .await?;
                return Ok(Some(map::journal(&full)?));
            }
        }
        Ok(None)
    }

    async fn find_customer_row(&self, name: &str) -> R<Option<Value>> {
        let rows = self.rest.get("customers").await?;
        Ok(rows
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| same_name(&s(&r["name"]), name))
            .cloned())
    }

    async fn recurring_rows(&self) -> R<Vec<Value>> {
        Ok(self
            .rest
            .get("recurring-invoices")
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    async fn payment_body(
        &self,
        kind: &str,
        party: &str,
        date: NaiveDate,
        amount: Money,
        reference: &str,
        apps: Vec<(String, Money)>,
    ) -> (Value, Value) {
        let tool = json!({
            "payment_type": kind,
            "party_id": party,
            "payment_date": date.to_string(),
            "amount": map::num(amount),
            "currency": amount.currency.code(),
            "fx_rate": 1.0,
            "method": "paypal",
            "reference": reference,
            "bank_account_id": self.clearing_bank,
            "applications": apps.iter().map(|(d, m)| json!({"document_id": d, "amount": map::num(*m)})).collect::<Vec<_>>(),
        });
        let rest = json!({
            "payment_type": kind,
            "party_id": party,
            "payment_date": date.to_string(),
            "amount": dec(amount),
            "currency": amount.currency.code(),
            "fx_rate": "1",
            "method": {"Card": {"processor": "PayPal", "authorization": reference}},
            "reference": reference,
            "bank_account_id": self.clearing_bank,
            "applications": apps.iter().map(|(d, m)| json!({"document_id": d, "amount": dec(*m)})).collect::<Vec<_>>(),
        });
        (tool, rest)
    }

    /// The customer-portal password ERA keeps for one customer's support logins: derived from
    /// the company's own login, so nothing new is stored.
    fn portal_password(&self, customer: &CustomerId) -> String {
        let mut h = Sha256::new();
        h.update(self.password_digest);
        h.update(b"era-portal:");
        h.update(customer.0.as_bytes());
        hex::encode(h.finalize())[..32].to_owned()
    }
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn id_of(v: &Value) -> R<String> {
    v["id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| BooksError::Backend(format!("ERA returned no id: {v}")))
}

#[async_trait::async_trait]
impl Books for EraBooks {
    fn company(&self) -> &CompanyId {
        &self.company
    }

    async fn ensure_customer(
        &self,
        who: &Counterparty,
        credit_limit: Option<Money>,
    ) -> BooksResult<Customer> {
        let id = match self.find_customer_row(&who.name).await? {
            Some(row) => {
                let id = s(&row["id"]);
                let current = map::opt_money(&row["credit_limit"], who.currency)?;
                if credit_limit.is_some() && credit_limit != current {
                    self.set_credit_limit(&CustomerId(id.clone()), credit_limit)
                        .await?;
                }
                id
            }
            None => {
                let mut body = map::party_body(who);
                body["payment_terms"] = json!("Net14");
                if let Some(l) = credit_limit {
                    body["credit_limit"] = dec(l);
                }
                id_of(&self.rest.post("customers", &body).await?)?
            }
        };
        self.customer(&CustomerId(id)).await
    }

    async fn customer(&self, id: &CustomerId) -> BooksResult<Customer> {
        let row = self.rest.get(&format!("customers/{id}")).await?;
        let cur = map::currency(&row["currency"], Currency::USD);
        let balances = self.customer_balances().await?;
        map::customer(&row, Self::balance_of(&balances, &id.0, cur))
    }

    async fn customers(&self) -> BooksResult<Vec<Customer>> {
        let balances = self.customer_balances().await?;
        let rows = self.rest.get("customers").await?;
        rows.as_array()
            .into_iter()
            .flatten()
            .map(|r| {
                let cur = map::currency(&r["currency"], Currency::USD);
                map::customer(r, Self::balance_of(&balances, &s(&r["id"]), cur))
            })
            .collect()
    }

    async fn ensure_vendor(&self, who: &Counterparty) -> BooksResult<Vendor> {
        let rows = self.rest.get("vendors").await?;
        let found = rows
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| same_name(&s(&r["name"]), &who.name))
            .map(|r| s(&r["id"]));
        let id = match found {
            Some(id) => id,
            None => {
                let mut body = map::party_body(who);
                body["payment_terms"] = json!("Net30");
                if let Some(p) = &who.paypal_email {
                    body["bank_details"] = json!({"bank_name": "PayPal", "account_name": who.name, "account_number": p});
                }
                id_of(&self.rest.post("vendors", &body).await?)?
            }
        };
        self.vendor(&VendorId(id)).await
    }

    async fn vendor(&self, id: &VendorId) -> BooksResult<Vendor> {
        let row = self.rest.get(&format!("vendors/{id}")).await?;
        let cur = map::currency(&row["currency"], Currency::USD);
        let balances = self.vendor_balances().await?;
        Ok(map::vendor(&row, Self::balance_of(&balances, &id.0, cur)))
    }

    async fn vendors(&self) -> BooksResult<Vec<Vendor>> {
        let balances = self.vendor_balances().await?;
        let rows = self.rest.get("vendors").await?;
        Ok(rows
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| {
                let cur = map::currency(&r["currency"], Currency::USD);
                map::vendor(r, Self::balance_of(&balances, &s(&r["id"]), cur))
            })
            .collect())
    }

    async fn ensure_product(&self, draft: &ProductDraft) -> BooksResult<Product> {
        let rows = self.rest.get("products").await?;
        let found = rows
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| same_name(&s(&p["name"]), &draft.name))
            .cloned();
        let row = match found {
            Some(r) => r,
            None => {
                let body = json!({
                    "name": draft.name,
                    "description": draft.description,
                    "product_type": "Service",
                    "unit_price": dec(draft.price),
                    "currency": draft.price.currency.code(),
                    "uom": "Each",
                    "sales_account": self.accounts.revenue,
                    "vat_treatment": "OutOfScope",
                    "track_inventory": false,
                });
                let id = id_of(&self.rest.post("products", &body).await?)?;
                self.rest.get(&format!("products/{id}")).await?
            }
        };
        let cur = map::currency(&row["currency"], draft.price.currency);
        Ok(Product {
            id: ProductId(s(&row["id"])),
            name: s(&row["name"]),
            sku: draft.sku.clone(),
            price: map::money(&row["unit_price"], cur)?,
        })
    }

    async fn products(&self) -> BooksResult<Vec<Product>> {
        let rows = self.rest.get("products").await?;
        rows.as_array()
            .into_iter()
            .flatten()
            .filter(|p| map::unquote(&p["product_type"]) != "Expense")
            .map(|p| {
                let cur = map::currency(&p["currency"], Currency::USD);
                Ok(Product {
                    id: ProductId(s(&p["id"])),
                    name: s(&p["name"]),
                    sku: None,
                    price: map::money(&p["unit_price"], cur)?,
                })
            })
            .collect()
    }

    async fn draft_invoice(&self, draft: &InvoiceDraft) -> BooksResult<Invoice> {
        if draft.lines.is_empty() {
            return Err(BooksError::Invalid(
                "an invoice needs at least one line".into(),
            ));
        }
        if let Some(l) = draft
            .lines
            .iter()
            .find(|l| l.unit_price.currency != draft.currency)
        {
            return Err(BooksError::Invalid(format!(
                "line {:?} is not in {}",
                l.description, draft.currency
            )));
        }
        let body = json!({
            "customer_id": draft.customer.0,
            "issue_date": draft.issue_date.to_string(),
            "due_date": draft.due_date.to_string(),
            "currency": draft.currency.code(),
            "fx_rate": "1",
            "lines": map::lines_body(&draft.lines, &self.service_product),
            "notes": map::notes_with_ref(draft.memo.as_deref(), draft.reference.as_deref()),
        });
        let created = self.rest.post("invoices", &body).await?;
        self.invoice(&InvoiceId(id_of(&created)?)).await
    }

    async fn submit_invoice(&self, id: &InvoiceId) -> BooksResult<Invoice> {
        // ERA's invoices go from draft straight to posted; there is no approval step to send to.
        self.invoice(id).await
    }

    async fn post_invoice(&self, id: &InvoiceId) -> BooksResult<Invoice> {
        let current = self.invoice(id).await?;
        if current.status != DocumentStatus::Draft {
            return Err(BooksError::Conflict(format!(
                "invoice {id} is already {:?}",
                current.status
            )));
        }
        self.write(
            "post_invoice",
            json!({"id": id.0}),
            Method::POST,
            &format!("invoices/{id}/post"),
            json!({}),
        )
        .await?;
        self.invoice(id).await
    }

    async fn invoice(&self, id: &InvoiceId) -> BooksResult<Invoice> {
        map::invoice(&self.rest.get(&format!("invoices/{id}")).await?)
    }

    async fn invoices(&self, filter: &DocumentFilter) -> BooksResult<Vec<Invoice>> {
        let mut out = Vec::new();
        for row in self.invoice_rows().await? {
            let inv = map::invoice(&row)?;
            if filter.party.as_deref().is_some_and(|p| inv.customer.0 != p)
                || (filter.open_only && !inv.status.is_open())
                || (filter.reference.is_some() && inv.reference != filter.reference)
            {
                continue;
            }
            out.push(inv);
        }
        Ok(out)
    }

    async fn create_recurring_invoice(
        &self,
        draft: &RecurringInvoiceDraft,
    ) -> BooksResult<RecurringInvoice> {
        if let Some(r) = &draft.reference {
            for row in self.recurring_rows().await? {
                let rec = map::recurring(&row)?;
                if rec.reference.as_ref() == Some(r) {
                    return Ok(rec);
                }
            }
        }
        if draft.lines.is_empty() {
            return Err(BooksError::Invalid(
                "a recurring invoice needs at least one line".into(),
            ));
        }
        let template = json!({
            "customer_id": draft.customer.0,
            "currency": draft.currency.code(),
            "fx_rate": "1",
            "lines": map::lines_body(&draft.lines, &self.service_product),
            "notes": map::notes_with_ref(None, draft.reference.as_deref()),
        });
        let body = json!({
            "customer_id": draft.customer.0,
            "template": template,
            "frequency": map::frequency_name(draft.frequency),
            "start_date": draft.start_date.to_string(),
            "end_date": draft.end_date.map(|d| d.to_string()),
            "auto_send": false,
            "auto_charge": false,
        });
        map::recurring(&self.rest.post("recurring-invoices", &body).await?)
    }

    async fn recurring_invoices(&self) -> BooksResult<Vec<RecurringInvoice>> {
        self.recurring_rows()
            .await?
            .iter()
            .map(map::recurring)
            .collect()
    }

    async fn stop_recurring_invoice(
        &self,
        id: &RecurringInvoiceId,
    ) -> BooksResult<RecurringInvoice> {
        let rows = self.recurring_rows().await?;
        let row =
            rows.iter()
                .find(|r| s(&r["id"]) == id.0)
                .ok_or_else(|| BooksError::NotFound {
                    what: "recurring invoice",
                    id: id.0.clone(),
                })?;
        // ERA runs a schedule only while `today <= end_date`, and refuses an end before the
        // start. A schedule that has run ends on its last run, keeping its history; one that
        // never ran has raised nothing, so it is deleted.
        let start = map::date(&row["start_date"])?;
        let end = match map::opt_date(&row["last_run"])? {
            Some(last) => {
                let template = match &row["template"] {
                    Value::String(t) => serde_json::from_str(t).unwrap_or(Value::Null),
                    other => other.clone(),
                };
                let body = json!({
                    "customer_id": row["customer_id"],
                    "template": template,
                    "frequency": map::unquote(&row["frequency"]),
                    "start_date": start.to_string(),
                    "end_date": last.to_string(),
                    "auto_send": row["auto_send"].as_bool().unwrap_or(false),
                    "auto_charge": false,
                });
                self.rest
                    .put(&format!("recurring-invoices/{id}"), &body)
                    .await?;
                last
            }
            None => {
                self.rest
                    .send(Method::DELETE, &format!("recurring-invoices/{id}"), None)
                    .await?;
                start
            }
        };
        let mut rec = map::recurring(row)?;
        rec.active = false;
        rec.next_date = None;
        rec.end_date = Some(end);
        Ok(rec)
    }

    async fn raise_due_recurring(&self, as_of: NaiveDate) -> BooksResult<Vec<Invoice>> {
        let mut raised = Vec::new();
        for row in self.recurring_rows().await? {
            let id = s(&row["id"]);
            let invoices = self
                .rest
                .get(&format!("recurring-invoices/{id}/invoices"))
                .await?;
            for inv in invoices.as_array().into_iter().flatten() {
                if s(&inv["status"]) == "draft" && map::date(&inv["issue_date"])? <= as_of {
                    raised.push(self.post_invoice(&InvoiceId(s(&inv["id"]))).await?);
                }
            }
        }
        Ok(raised)
    }

    async fn draft_bill(&self, draft: &BillDraft) -> BooksResult<Bill> {
        if draft.lines.is_empty() {
            return Err(BooksError::Invalid("a bill needs at least one line".into()));
        }
        let notes = draft.memo.clone();
        let tool = json!({
            "vendor_id": draft.vendor.0,
            "vendor_invoice_number": draft.vendor_reference,
            "issue_date": draft.issue_date.to_string(),
            "due_date": draft.due_date.to_string(),
            "currency": draft.currency.code(),
            "fx_rate": 1.0,
            "line_items": draft.lines.iter().map(|l| json!({
                "product_id": l.product.as_ref().map(|p| p.0.clone()).unwrap_or_else(|| self.expense_product.clone()),
                "description": l.description,
                "quantity": f64::from(l.quantity),
                "unit_price": map::num(l.unit_price),
            })).collect::<Vec<_>>(),
            "notes": notes,
        });
        let rest = json!({
            "vendor_id": draft.vendor.0,
            "vendor_invoice_number": draft.vendor_reference,
            "issue_date": draft.issue_date.to_string(),
            "due_date": draft.due_date.to_string(),
            "currency": draft.currency.code(),
            "fx_rate": "1",
            "lines": map::lines_body(&draft.lines, &self.expense_product),
            "notes": notes,
        });
        let created = self
            .write("create_bill_draft", tool, Method::POST, "bills", rest)
            .await?;
        self.bill(&BillId(id_of(&created)?)).await
    }

    async fn post_bill(&self, id: &BillId) -> BooksResult<Bill> {
        let current = self.bill(id).await?;
        match current.status {
            DocumentStatus::Draft => {
                self.rest
                    .post(&format!("bills/{id}/approve"), &json!({}))
                    .await?;
            }
            DocumentStatus::Submitted => {}
            other => {
                return Err(BooksError::Conflict(format!(
                    "bill {id} is already {other:?}"
                )));
            }
        }
        self.write(
            "post_bill",
            json!({"id": id.0}),
            Method::POST,
            &format!("bills/{id}/post"),
            json!({}),
        )
        .await?;
        self.bill(id).await
    }

    async fn bill(&self, id: &BillId) -> BooksResult<Bill> {
        map::bill(&self.rest.get(&format!("bills/{id}")).await?)
    }

    async fn bills(&self, filter: &DocumentFilter) -> BooksResult<Vec<Bill>> {
        let mut out = Vec::new();
        for row in self.rest.get_all("bills").await? {
            let b = map::bill(&row)?;
            if filter.party.as_deref().is_some_and(|p| b.vendor.0 != p)
                || (filter.open_only && !b.status.is_open())
                || (filter.reference.is_some() && b.vendor_reference != filter.reference)
            {
                continue;
            }
            out.push(b);
        }
        Ok(out)
    }

    async fn record_receipt(&self, draft: &ReceiptDraft) -> BooksResult<Payment> {
        if let Some(p) = self.payment_by_reference(&draft.reference).await? {
            return Ok(p);
        }
        if !draft.amount.is_positive() {
            return Err(BooksError::Invalid(
                "a receipt needs an amount above zero".into(),
            ));
        }
        let apps = draft
            .applications
            .iter()
            .map(|a| (a.invoice.0.clone(), a.amount))
            .collect();
        let (tool, rest) = self
            .payment_body(
                "customer_payment",
                &draft.customer.0,
                draft.date,
                draft.amount,
                &draft.reference,
                apps,
            )
            .await;
        let created = self
            .write("record_payment", tool, Method::POST, "payments", rest)
            .await?;
        self.payment(&PaymentId(id_of(&created)?)).await
    }

    async fn apply_receipt(
        &self,
        id: &PaymentId,
        applications: &[InvoiceApplication],
    ) -> BooksResult<Payment> {
        for a in applications {
            self.rest.post("payments/apply", &json!({"payment_id": id.0, "document_id": a.invoice.0, "amount": dec(a.amount)})).await?;
        }
        self.payment(id).await
    }

    async fn record_bill_payment(&self, draft: &BillPaymentDraft) -> BooksResult<Payment> {
        if let Some(p) = self.payment_by_reference(&draft.reference).await? {
            return Ok(p);
        }
        if !draft.amount.is_positive() {
            return Err(BooksError::Invalid(
                "a payment needs an amount above zero".into(),
            ));
        }
        let apps = draft
            .applications
            .iter()
            .map(|a| (a.bill.0.clone(), a.amount))
            .collect();
        let (tool, rest) = self
            .payment_body(
                "vendor_payment",
                &draft.vendor.0,
                draft.date,
                draft.amount,
                &draft.reference,
                apps,
            )
            .await;
        let created = self
            .write("record_payment", tool, Method::POST, "payments", rest)
            .await?;
        self.payment(&PaymentId(id_of(&created)?)).await
    }

    async fn record_refund(&self, draft: &RefundDraft) -> BooksResult<Refund> {
        if let Some(r) = self.refund_by_reference(&draft.reference).await? {
            return Ok(r);
        }
        let customer = self
            .rest
            .get(&format!("customers/{}", draft.customer))
            .await?;
        let zero = Money::zero(draft.amount.currency);
        let journal = JournalDraft {
            date: draft.date,
            memo: format!("Refund to {}: {}", s(&customer["name"]), draft.reason),
            reference: Some(draft.reference.clone()),
            lines: vec![
                JournalLine {
                    account: AccountCode(self.accounts.sales_returns.clone()),
                    debit: draft.amount,
                    credit: zero,
                    memo: None,
                },
                JournalLine {
                    account: AccountCode(self.accounts.clearing.clone()),
                    debit: zero,
                    credit: draft.amount,
                    memo: None,
                },
            ],
        };
        let entry = self.post_journal(&journal).await?;
        Ok(Refund {
            credit_note: None,
            payment: None,
            journal: Some(entry.id),
            amount: draft.amount,
            reference: draft.reference.clone(),
        })
    }

    async fn payment(&self, id: &PaymentId) -> BooksResult<Payment> {
        map::payment(&self.rest.get(&format!("payments/{id}")).await?)
    }

    async fn payments(&self, filter: &PaymentFilter) -> BooksResult<Vec<Payment>> {
        let mut out = Vec::new();
        for row in self.rest.get_all("payments").await? {
            if filter
                .reference
                .as_deref()
                .is_some_and(|r| row["reference"].as_str() != Some(r))
            {
                continue;
            }
            let p = map::payment(&row)?;
            if filter.direction.is_some_and(|d| d != p.direction) {
                continue;
            }
            out.push(p);
        }
        Ok(out)
    }

    async fn refund_by_reference(&self, reference: &str) -> BooksResult<Option<Refund>> {
        Ok(self.journal_by_reference(reference).await?.map(|j| {
            let amount = j
                .lines
                .iter()
                .find(|l| l.debit.is_positive())
                .map(|l| l.debit)
                .unwrap_or(Money::zero(Currency::USD));
            Refund {
                credit_note: None,
                payment: None,
                journal: Some(j.id),
                amount,
                reference: reference.to_owned(),
            }
        }))
    }

    async fn account(&self, role: AccountRole) -> BooksResult<Account> {
        let code = match role {
            AccountRole::PayPalClearing => &self.accounts.clearing,
            AccountRole::Receivables => &self.accounts.receivables,
            AccountRole::Payables => &self.accounts.payables,
            AccountRole::Revenue => &self.accounts.revenue,
            AccountRole::PayPalFees => &self.accounts.fees,
        };
        let accounts = self.rest.get("accounts").await?;
        let row = accounts
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| a["code"].as_str() == Some(code.as_str()))
            .ok_or_else(|| BooksError::NotFound {
                what: "account",
                id: code.clone(),
            })?;
        let balance = if role == AccountRole::PayPalClearing {
            let banks = self.rest.get("bank-accounts").await?;
            match banks
                .as_array()
                .into_iter()
                .flatten()
                .find(|b| s(&b["id"]) == self.clearing_bank)
            {
                Some(b) => Some(map::money(
                    &b["balance"],
                    map::currency(&b["currency"], Currency::USD),
                )?),
                None => None,
            }
        } else {
            None
        };
        Ok(Account {
            code: AccountCode(code.clone()),
            name: s(&row["name"]),
            balance,
        })
    }

    async fn post_journal(&self, draft: &JournalDraft) -> BooksResult<JournalEntry> {
        draft.check()?;
        if let Some(r) = &draft.reference {
            if let Some(j) = self.journal_by_reference(r).await? {
                return Ok(j);
            }
        }
        let reference = draft.reference.clone().unwrap_or_default();
        let side = |m: Money, as_num: bool| -> Value {
            if !m.is_positive() {
                Value::Null
            } else if as_num {
                map::num(m)
            } else {
                dec(m)
            }
        };
        let tool = json!({
            "date": draft.date.to_string(),
            "reference": reference,
            "description": draft.memo,
            "lines": draft.lines.iter().map(|l| json!({
                "account_code": l.account.0,
                "debit": side(l.debit, true),
                "credit": side(l.credit, true),
                "currency": l.debit.currency.code(),
                "fx_rate": 1.0,
                "description": l.memo,
            })).collect::<Vec<_>>(),
        });
        let rest = json!({
            "date": draft.date.to_string(),
            "source": "Manual",
            "reference": reference,
            "description": draft.memo,
            "post_immediately": true,
            "lines": draft.lines.iter().map(|l| json!({
                "account_code": l.account.0,
                "debit": side(l.debit, false),
                "credit": side(l.credit, false),
                "currency": l.debit.currency.code(),
                "fx_rate": "1",
                "description": l.memo,
            })).collect::<Vec<_>>(),
        });
        match self
            .write(
                "post_journal_entry",
                tool,
                Method::POST,
                "journal-entries",
                rest,
            )
            .await
        {
            Ok(created) => {
                let id = created["entry"]["id"]
                    .as_str()
                    .map(str::to_owned)
                    .map_or_else(|| id_of(&created), Ok)?;
                map::journal(&self.rest.get(&format!("journal-entries/{id}")).await?)
            }
            // Two writers raced on the same reference: ERA keeps references unique, so the
            // other one's entry is this one.
            Err(BooksError::Invalid(msg))
                if msg.contains("already exists") && draft.reference.is_some() =>
            {
                let r = draft.reference.as_deref().unwrap_or_default();
                self.journal_by_reference(r)
                    .await?
                    .ok_or(BooksError::Invalid(msg))
            }
            Err(e) => Err(e),
        }
    }

    async fn journal_entry(&self, id: &JournalId) -> BooksResult<JournalEntry> {
        map::journal(&self.rest.get(&format!("journal-entries/{id}")).await?)
    }

    async fn journal(&self, filter: &JournalFilter) -> BooksResult<Vec<JournalEntry>> {
        let mut out = Vec::new();
        for row in self.rest.get_all("journal-entries").await? {
            let date = map::date(&row["date"])?;
            if filter.from.is_some_and(|f| date < f)
                || filter.to.is_some_and(|t| date > t)
                || filter
                    .reference
                    .as_deref()
                    .is_some_and(|r| row["reference"].as_str() != Some(r))
            {
                continue;
            }
            out.push(map::journal(
                &self
                    .rest
                    .get(&format!("journal-entries/{}", s(&row["id"])))
                    .await?,
            )?);
        }
        Ok(out)
    }

    async fn audit_trail(&self, doc: &DocumentRef) -> BooksResult<Vec<AuditEntry>> {
        let (kind, id) = match doc {
            DocumentRef::Customer(id) => ("customer", id.0.as_str()),
            DocumentRef::Vendor(id) => ("vendor", id.0.as_str()),
            DocumentRef::Invoice(id) => ("invoice", id.0.as_str()),
            DocumentRef::Bill(id) => ("bill", id.0.as_str()),
            DocumentRef::Payment(id) => ("payment", id.0.as_str()),
            DocumentRef::CreditNote(id) => ("invoice", id.0.as_str()),
            DocumentRef::Journal(id) => ("journal_entry", id.0.as_str()),
        };
        let v = self.rest.get(&format!("audit/{kind}/{id}")).await?;
        Ok(v["events"]
            .as_array()
            .into_iter()
            .flatten()
            .map(map::audit)
            .collect())
    }

    async fn ar_ageing(&self, as_of: NaiveDate) -> BooksResult<AgeingReport> {
        let entity = self.entity_id().await?;
        let report = self
            .rest
            .post("reports", &json!({"entity_id": entity, "report_type": "ArAgeing", "parameters": {"as_at": as_of.to_string()}}))
            .await?;
        map::ageing(&report, "ArAgeing", as_of, Currency::USD)
    }

    async fn ap_ageing(&self, as_of: NaiveDate) -> BooksResult<AgeingReport> {
        let entity = self.entity_id().await?;
        let report = self
            .rest
            .post("reports", &json!({"entity_id": entity, "report_type": "ApAgeing", "parameters": {"as_at": as_of.to_string()}}))
            .await?;
        map::ageing(&report, "ApAgeing", as_of, Currency::USD)
    }

    async fn set_credit_limit(
        &self,
        id: &CustomerId,
        limit: Option<Money>,
    ) -> BooksResult<Customer> {
        // ERA reads this one field as a JSON number.
        let value = limit.map(map::num).unwrap_or(Value::Null);
        self.rest
            .put(&format!("customers/{id}"), &json!({"credit_limit": value}))
            .await?;
        self.customer(id).await
    }

    async fn credit_position(&self, id: &CustomerId) -> BooksResult<CreditPosition> {
        let c = self.customer(id).await?;
        let available = match c.credit_limit {
            Some(l) => Some(l.checked_sub(c.balance)?),
            None => None,
        };
        Ok(CreditPosition {
            customer: id.clone(),
            limit: c.credit_limit,
            exposure: c.balance,
            available,
        })
    }

    async fn create_lead(&self, draft: &LeadDraft) -> BooksResult<Lead> {
        let body = json!({
            "name": draft.contact_name,
            "company": draft.company_name,
            "email": draft.email,
            "source": draft.source,
            "notes": draft.notes,
        });
        let id = id_of(&self.rest.post("crm/leads", &body).await?)?;
        Ok(Lead {
            id: LeadId(id),
            contact_name: draft.contact_name.clone(),
            company_name: draft.company_name.clone(),
            email: draft.email.clone(),
            status: LeadStatus::New,
        })
    }

    async fn leads(&self) -> BooksResult<Vec<Lead>> {
        Ok(self
            .rest
            .get("crm/leads")
            .await?
            .as_array()
            .into_iter()
            .flatten()
            .map(map::lead)
            .collect())
    }

    async fn convert_lead(
        &self,
        id: &LeadId,
        draft: &OpportunityDraft,
    ) -> BooksResult<Opportunity> {
        let lead = self
            .leads()
            .await?
            .into_iter()
            .find(|l| &l.id == id)
            .ok_or_else(|| BooksError::NotFound {
                what: "lead",
                id: id.0.clone(),
            })?;
        if matches!(lead.status, LeadStatus::Converted | LeadStatus::Lost) {
            return Err(BooksError::Conflict(format!(
                "lead {id} is already {:?}",
                lead.status
            )));
        }
        // Opened directly rather than through ERA's convert, which fixes the currency to KES.
        let body = json!({
            "name": draft.name,
            "lead_id": id.0,
            "customer_id": draft.customer.as_ref().map(|c| c.0.clone()),
            "amount": dec(draft.amount),
            "currency": draft.amount.currency.code(),
            "expected_close_date": draft.expected_close.map(|d| d.to_string()),
        });
        let opp = id_of(&self.rest.post("crm/opportunities", &body).await?)?;
        self.rest
            .put(&format!("crm/leads/{id}"), &json!({"status": "Converted"}))
            .await?;
        self.opportunities()
            .await?
            .into_iter()
            .find(|o| o.id.0 == opp)
            .ok_or(BooksError::NotFound {
                what: "opportunity",
                id: opp.clone(),
            })
    }

    async fn win_opportunity(&self, id: &OpportunityId) -> BooksResult<Opportunity> {
        self.rest
            .post(&format!("crm/opportunities/{id}/win"), &json!({}))
            .await?;
        self.opportunities()
            .await?
            .into_iter()
            .find(|o| &o.id == id)
            .ok_or_else(|| BooksError::NotFound {
                what: "opportunity",
                id: id.0.clone(),
            })
    }

    async fn lose_opportunity(&self, id: &OpportunityId, reason: &str) -> BooksResult<Opportunity> {
        self.rest
            .post(
                &format!("crm/opportunities/{id}/lose"),
                &json!({"reason": reason}),
            )
            .await?;
        self.opportunities()
            .await?
            .into_iter()
            .find(|o| &o.id == id)
            .ok_or_else(|| BooksError::NotFound {
                what: "opportunity",
                id: id.0.clone(),
            })
    }

    async fn opportunities(&self) -> BooksResult<Vec<Opportunity>> {
        self.rest
            .get("crm/opportunities")
            .await?
            .as_array()
            .into_iter()
            .flatten()
            .map(map::opportunity)
            .collect()
    }

    async fn open_ticket(&self, draft: &TicketDraft) -> BooksResult<Ticket> {
        let customer = match &draft.customer {
            Some(c) => c.clone(),
            None => {
                self.ensure_customer(&Counterparty::usd("Website visitors", None), None)
                    .await?
                    .id
            }
        };
        let row = self.rest.get(&format!("customers/{customer}")).await?;
        let email = map::counterparty(&row)
            .email
            .unwrap_or_else(|| format!("support+{customer}@agentrix.website"));
        let password = self.portal_password(&customer);
        // ERA's staff can't open tickets: the customer's own portal login does. Inviting an
        // existing portal user fails harmlessly; the login then decides.
        let _ = self
            .rest
            .post("crm/customers/invite-portal", &json!({"email": email, "customer_id": customer.0, "display_name": s(&row["name"]), "password": password}))
            .await;
        let login = self
            .rest
            .raw(
                Method::POST,
                "customerportal/login",
                &json!({"email": email, "password": password}),
                None,
            )
            .await
            .map_err(|e| match e {
                BooksError::Backend(_) | BooksError::Invalid(_) | BooksError::NotFound { .. } => {
                    BooksError::Unsupported(
                        "tickets: ERA's customer portal serves another tenant (its ENTITY_ID)",
                    )
                }
                other => other,
            })?;
        let token = login["access_token"]
            .as_str()
            .ok_or_else(|| BooksError::Backend("portal login returned no token".into()))?;
        let body = json!({"subject": draft.subject, "description": draft.description, "priority": map::priority_name(draft.priority)});
        let created = self
            .rest
            .raw(Method::POST, "customerportal/tickets", &body, Some(token))
            .await?;
        let id = id_of(&created)?;
        Ok(map::ticket(
            &self.rest.get(&format!("crm/tickets/{id}")).await?,
        ))
    }

    async fn update_ticket(
        &self,
        id: &TicketId,
        status: TicketStatus,
        note: Option<&str>,
    ) -> BooksResult<Ticket> {
        if let Some(n) = note {
            self.rest
                .post(&format!("crm/tickets/{id}/reply"), &json!({"body": n}))
                .await?;
        }
        self.rest
            .put(
                &format!("crm/tickets/{id}/status"),
                &json!({"status": map::ticket_status_name(status)}),
            )
            .await?;
        Ok(map::ticket(
            &self.rest.get(&format!("crm/tickets/{id}")).await?,
        ))
    }

    async fn tickets(&self, status: Option<TicketStatus>) -> BooksResult<Vec<Ticket>> {
        let path = match status {
            Some(st) => format!("crm/tickets?status={}", map::ticket_status_name(st)),
            None => "crm/tickets".to_owned(),
        };
        Ok(self
            .rest
            .get(&path)
            .await?
            .as_array()
            .into_iter()
            .flatten()
            .map(map::ticket)
            .collect())
    }
}
