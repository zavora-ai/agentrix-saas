//! The owner's decision cards for custom orders that an agent asked for at a sample site's
//! agent door (v3, "Verify at sample doors"). Each held custom order of a sample site that is a
//! company ([`crate::samples::joined`]) is a card in that company's Agentrix OS, naming the
//! proven agent and who vouched for it.
//!
//! The card's id is `order:<site>:<order id>`. Its fingerprint is `sha256:` and the hex SHA-256
//! of the order's held terms as canonical JSON (RFC 8785): the site, the order, the amount, the
//! items and the proven agent ([`terms`]). The owner's passkey signs that fingerprint, so the
//! owner approves exactly the order they see. On approval the terms are read again and must
//! give the same fingerprint. Approving or declining is the sample site's own decision, so the
//! customer is told just as from the site's Custom orders page.

use agentrix_records::orders::OrderView;
use saas_core::CompanyId;
use saas_web::console::ConsoleError;
use saas_web::feed::{Decision, RequestedBy};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Every door card's id starts with this.
pub const PREFIX: &str = "order:";

/// How many held orders a site shows as cards at most.
const MOST: i64 = 20;

/// A door card's id: `order:<site>:<order id>`.
pub fn card_id(site: &str, order_id: &str) -> String {
    format!("{PREFIX}{site}:{order_id}")
}

/// The site and order a door card's id names.
pub fn parse_id(id: &str) -> Option<(&str, &str)> {
    let (site, order) = id.strip_prefix(PREFIX)?.split_once(':')?;
    (!site.is_empty() && !order.is_empty()).then_some((site, order))
}

/// The company slug (`kilele`, `agentrix`) a joined sample site's orders belong to.
fn company_slug(company: &CompanyId) -> &'static str {
    if *company == CompanyId::kilele() {
        saas_web::feed::KILELE
    } else {
        "agentrix"
    }
}

/// Whether `site` is a sample site whose orders are `company`'s (a company slug).
pub fn is_companys(site: &str, company: &str) -> bool {
    crate::samples::joined()
        .iter()
        .any(|(s, c)| *s == site && company_slug(c) == company)
}

/// The held terms the owner signs: the site, the order, what it costs, what is in it, and the
/// agent that asked for it as it proved itself (`null` when none did).
pub fn terms(site: &str, view: &OrderView) -> Value {
    let o = &view.order;
    json!({
        "site": site,
        "order_id": o.order_id,
        "amount": {
            "currency": o.currency,
            "total_minor": o.total_minor,
            "usd_cents": o.usd_cents(),
        },
        "items": view.lines.iter().map(|l| json!({
            "sku": l.sku,
            "name": l.name,
            "qty": l.qty,
            "unit_minor": l.unit_minor,
            "amount_minor": l.amount_minor,
        })).collect::<Vec<_>>(),
        "requested_by": o.proven_agent().map(|p| json!({
            "id": p.id,
            "vouched_by": p.vouched_by,
            "method": p.method,
        })),
    })
}

/// `sha256:<hex>` of the held terms as canonical JSON.
pub fn fingerprint(site: &str, view: &OrderView) -> String {
    let terms = terms(site, view);
    // Canonical JSON fails only on numbers it can't write, which these terms don't hold.
    let text = adk_auth::signed_audit::canonicalize(&terms).unwrap_or_else(|_| terms.to_string());
    format!("sha256:{}", hex::encode(Sha256::digest(text.as_bytes())))
}

/// Minor units in words: `KES 4,500.00`.
fn money(currency: &str, minor: i64) -> String {
    let units = minor.abs() / 100;
    let digits = units.to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let sign = if minor < 0 { "-" } else { "" };
    format!("{currency} {sign}{grouped}.{:02}", minor.abs() % 100)
}

/// The card for held order `view` of `site`, in `company`'s Agentrix OS.
pub fn card(site: &str, company: &str, view: &OrderView) -> Decision {
    let o = &view.order;
    let items = view
        .lines
        .iter()
        .map(|l| format!("{} x{}", l.name, l.qty))
        .collect::<Vec<_>>()
        .join(", ");
    let mut body = format!(
        "{} asked for a custom order: {}. {} in all ({}), for {}.",
        o.contact_name,
        if items.is_empty() {
            "see the design"
        } else {
            items.as_str()
        },
        money(&o.currency, o.total_minor),
        money("USD", o.usd_cents()),
        o.fulfilment,
    );
    if let Some(note) = o.note.as_deref().filter(|n| !n.trim().is_empty()) {
        body.push_str(&format!(" Their note: {note}"));
    }
    Decision {
        id: card_id(site, &o.order_id),
        company: company.to_owned(),
        agent: "door".into(),
        title: format!("Custom order {} waits for you", o.order_id),
        body,
        effects: vec![
            format!(
                "If you approve, {} gets a payment request for {} on the order page",
                o.contact_name,
                money(&o.currency, o.total_minor)
            ),
            format!(
                "If you decline, nothing is charged and {} is told",
                o.contact_name
            ),
        ],
        why: "A custom order is made to order, so it waits for you before anyone pays".into(),
        currency: "USD".into(),
        amount_minor: Some(o.usd_cents()),
        state: "open".into(),
        fingerprint: fingerprint(site, view),
        requested_by: o.proven_agent().map(|p| RequestedBy {
            id: p.id,
            vouched_by: p.vouched_by,
            method: p.method,
        }),
    }
}

/// The cards for `company`'s sample sites' held custom orders, newest first.
pub async fn cards(company: &str) -> anyhow::Result<Vec<Decision>> {
    let mut out = Vec::new();
    for (site, owner) in crate::samples::joined() {
        if company_slug(&owner) != company {
            continue;
        }
        for view in agentrix_samples::held::held(site, MOST).await? {
            out.push(card(site, company, &view));
        }
    }
    Ok(out)
}

/// The owner of `company` approves or declines door card `id`. The order must still be held;
/// an approval's passkey `proof`, when there is one, must be over the terms held now.
pub async fn decide(
    company: &str,
    id: &str,
    approve: bool,
    proof: Option<&Value>,
) -> Result<String, ConsoleError> {
    let (site, order_id) = parse_id(id).ok_or(ConsoleError::NotFound)?;
    if !is_companys(site, company) {
        return Err(ConsoleError::NotFound);
    }
    let view = agentrix_samples::held::order(site, order_id)
        .await
        .map_err(|e| ConsoleError::Failed(e.to_string()))?
        .ok_or(ConsoleError::NotFound)?;
    if view.order.state != "held" {
        return Err(ConsoleError::Decided(view.order.state.clone()));
    }
    if approve
        && let Some(proof) = proof
        && proof["fingerprint"].as_str() != Some(fingerprint(site, &view).as_str())
    {
        return Err(ConsoleError::BadProof(
            "the order changed since you looked at it; nothing was approved".into(),
        ));
    }
    agentrix_samples::held::decide(site, order_id, approve, None)
        .await
        .map_err(|e| ConsoleError::Failed(e.to_string()))?;
    tracing::info!(
        company,
        site,
        order = order_id,
        approve,
        signed = proof.is_some(),
        "the owner decided a custom order from Agentrix OS"
    );
    Ok(if approve { "approved" } else { "declined" }.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_id_names_its_site_and_order() {
        let id = card_id("kilele-bakery-sample", "KB-1052");
        assert_eq!(id, "order:kilele-bakery-sample:KB-1052");
        assert_eq!(parse_id(&id), Some(("kilele-bakery-sample", "KB-1052")));
        assert_eq!(parse_id("sha256:abc"), None);
        assert_eq!(parse_id("order::KB-1"), None);
        assert!(is_companys("kilele-bakery-sample", "kilele"));
        assert!(!is_companys("kilele-bakery-sample", "agentrix"));
        assert!(!is_companys("classy-perfumes-sample", "kilele"));
    }

    #[test]
    fn money_reads_as_the_cards_say_it() {
        assert_eq!(money("KES", 450_000), "KES 4,500.00");
        assert_eq!(money("USD", 2_171), "USD 21.71");
        assert_eq!(money("USD", 5), "USD 0.05");
    }
}
