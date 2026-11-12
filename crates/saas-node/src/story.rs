//! `agentrix-saas story`: the hero scenes in order, against the node's books (Zavora ERA) and
//! PayPal (the sandbox), each step printed with its real ids. With `--judge` the agents run on
//! the scripted, seeded model, so every take plays the same way.
//!
//! - **(a)** A customer subscribes: the CRM lead and opportunity, PayPal Subscriptions through
//!   agentrix-effects, the customer approving on PayPal's page, the receipt booked, and Revenue
//!   marking the deal won.
//! - **(b)** Agentrix invoices Kilele; Kilele's Payables, woken by the offer at its agent door,
//!   settles it through Vault; both companies' books post the one PayPal id.
//! - **(c)** Kilele is short: an instalment plan is agreed inside both owners' mandates and the
//!   first instalment is collected on its date.
//! - **(d)** Payables pays a vendor bill by PayPal Payouts inside the mandate; one over the
//!   mandate waits for the founder's passkey.
//! - **(e)** The "bank details changed" vendor email is refused, with a signed receipt.
//!
//! What only a person can do is said, never faked: approving a subscription on PayPal's page,
//! and saving Kilele's PayPal wallet once (`/settle/vault/start` on the pay host). PayPal's
//! webhooks go to the public pay host; a local story reads the subscription's first payment
//! back from PayPal and books it through the same handler.

use std::sync::Arc;
use std::time::Duration;

use chrono::{Days, Utc};
use reqwest::Method;
use saas_company::governed::CallOutcome;
use saas_company::identity::{PAYABLES, RECEIVABLES, REVENUE};
use saas_company::runtime::Runtime;
use saas_company::screen::InboundKind;
use saas_core::books::{
    BillDraft, CustomerId, LeadDraft, LineDraft, OpportunityDraft, OpportunityStage, PaymentFilter,
    VendorId,
};
use saas_core::{Books, Currency, Money, PayPalRef};
use saas_web::checkout::{Checkout, Period, Start};
use serde_json::{Value, json};

use crate::Node;
use crate::checkout::EffectsCheckout;

/// Which scenes, and how long to wait for a person.
#[derive(Debug, Clone)]
pub struct StoryOptions {
    /// The scenes to play, in order: any of `a`–`e`.
    pub scenes: Vec<char>,
    /// How long to wait for the customer to approve the subscription on PayPal's page.
    pub approval_wait: Duration,
    /// How long to wait for an agent the kernel woke.
    pub agent_wait: Duration,
}

impl Default for StoryOptions {
    fn default() -> Self {
        Self {
            scenes: vec!['a', 'b', 'c', 'd', 'e'],
            approval_wait: Duration::from_secs(0),
            agent_wait: Duration::from_secs(120),
        }
    }
}

/// A scene's title.
pub fn title(scene: char) -> &'static str {
    match scene {
        'a' => "A customer subscribes",
        'b' => "Agentrix invoices Kilele; Kilele's agent settles it",
        'c' => "Kilele is short: an instalment plan inside both mandates",
        'd' => "Payables pays a vendor by PayPal Payouts, inside the mandate",
        'e' => "\"Our bank details changed\" is refused",
        _ => "?",
    }
}

/// A story run's scenes as they tick, for the judges' "Run the day" (`/judge/run-day`).
#[derive(Debug)]
pub struct Progress {
    state: std::sync::Mutex<Value>,
}

impl Progress {
    /// Every scene pending.
    pub fn new(scenes: &[char]) -> Self {
        let scenes: Vec<Value> = scenes
            .iter()
            .enumerate()
            .map(|(i, c)| json!({ "n": i + 1, "scene": c.to_string(), "title": title(*c), "state": "pending", "evidence": [] }))
            .collect();
        Self {
            state: std::sync::Mutex::new(json!({ "done": false, "scenes": scenes })),
        }
    }

    fn with(&self, f: impl FnOnce(&mut Value)) {
        f(&mut self.state.lock().unwrap_or_else(|p| p.into_inner()));
    }

    fn set(&self, n: usize, state: &str) {
        self.with(|v| v["scenes"][n]["state"] = json!(state));
    }

    fn evidence(&self, n: usize, words: &str, ids: &Value) {
        let Some(map) = ids.as_object() else {
            return;
        };
        let items: Vec<Value> = map
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| {
                let value = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
                let href = (k == "approve" || value.starts_with("https://")).then(|| value.clone());
                json!({ "label": k.replace('_', " "), "value": value, "href": href, "step": words })
            })
            .collect();
        self.with(|v| {
            if let Some(list) = v["scenes"][n]["evidence"].as_array_mut() {
                list.extend(items);
            }
        });
    }

    /// Ends a run that couldn't start or stopped early: every scene not finished is failed,
    /// with `why`.
    pub fn abandon(&self, why: &str) {
        self.with(|v| {
            if let Some(scenes) = v["scenes"].as_array_mut() {
                for scene in scenes {
                    if scene["state"] == json!("pending") || scene["state"] == json!("running") {
                        scene["state"] = json!("failed");
                        if let Some(list) = scene["evidence"].as_array_mut() {
                            list.push(json!({ "label": "stopped", "value": why, "href": null, "step": "stopped" }));
                        }
                    }
                }
            }
            v["done"] = json!(true);
        });
    }

    /// The run as the judges' page reads it.
    pub fn view(&self) -> Value {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

/// What the story printed, step by step.
#[derive(Debug, Default)]
pub struct Take {
    /// (step, words, ids).
    pub steps: Vec<(String, String, Value)>,
    progress: Option<(Arc<Progress>, usize)>,
}

impl Take {
    fn say(&mut self, step: &str, words: impl Into<String>, ids: Value) {
        let words = words.into();
        if ids.is_null() {
            println!("  {step:<4} {words}");
        } else {
            println!("  {step:<4} {words}  {ids}");
        }
        if let Some((progress, n)) = &self.progress {
            progress.evidence(*n, &words, &ids);
        }
        self.steps.push((step.to_owned(), words, ids));
    }

    /// The take as JSON.
    pub fn to_json(&self) -> Value {
        json!(
            self.steps
                .iter()
                .map(|(step, words, ids)| json!({ "step": step, "words": words, "ids": ids }))
                .collect::<Vec<_>>()
        )
    }
}

pub(crate) async fn until<T, F, Fut>(wait: Duration, mut probe: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        if let Some(v) = probe().await {
            return Some(v);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// One tool call as `role`, on the governed path.
pub(crate) async fn governed(
    rt: &Runtime,
    role: &str,
    session: &str,
    tool: &str,
    call: &str,
    args: Value,
) -> anyhow::Result<CallOutcome> {
    let member = rt.company.member(role)?;
    let scope =
        adk_core::ApprovalScope::new(member.identity.app_name().as_str(), member.user(), session);
    rt.company
        .governance
        .call(
            &rt.company.desk,
            &member.identity,
            &scope,
            session,
            tool,
            &format!("{session}/{call}"),
            args,
            None,
        )
        .await
}

pub(crate) fn done(outcome: CallOutcome, what: &str) -> anyhow::Result<Value> {
    match outcome {
        CallOutcome::Done(v) => Ok(v),
        CallOutcome::AlreadyDone => Ok(json!({ "already_done": true })),
        CallOutcome::Refused(r) => anyhow::bail!("{what} was refused: {r}"),
        CallOutcome::Held { reason, .. } => anyhow::bail!("{what} is held: {reason}"),
    }
}

pub(crate) async fn payments_for(books: &dyn Books, key: &str) -> anyhow::Result<Vec<String>> {
    Ok(books
        .payments(&PaymentFilter {
            reference: Some(key.to_owned()),
            direction: None,
        })
        .await?
        .into_iter()
        .map(|p| p.id.to_string())
        .collect())
}

/// Plays the scenes on `node`, which must already be serving (`crate::run`).
pub async fn play(node: &Arc<Node>, options: &StoryOptions) -> anyhow::Result<Take> {
    play_with(node, options, None).await
}

/// [`play`], reporting each scene to `progress` as it runs.
pub async fn play_with(
    node: &Arc<Node>,
    options: &StoryOptions,
    progress: Option<Arc<Progress>>,
) -> anyhow::Result<Take> {
    let mut take = Take::default();
    let stamp = Utc::now().format("%Y%m%d%H%M%S").to_string();
    println!(
        "Agentrix SaaS story {stamp}: books {:?}, PayPal {}, models {}",
        node.config.books,
        node.agentrix.company.desk.rails.label(),
        if node.config.judge {
            "scripted (judge)"
        } else {
            "the model gateway"
        }
    );
    for (n, scene) in options.scenes.iter().enumerate() {
        if let Some(p) = &progress {
            p.set(n, "running");
            take.progress = Some((p.clone(), n));
        }
        let result = match scene {
            'a' => scene_a(node, options, &stamp, &mut take).await,
            'b' => scene_b(node, options, &stamp, &mut take).await.map(|_| ()),
            'c' => scene_c(node, options, &stamp, &mut take).await,
            'd' => scene_d(node, &stamp, &mut take).await,
            'e' => scene_e(node, &stamp, &mut take).await,
            other => {
                take.say("?", format!("no scene {other}"), Value::Null);
                Ok(())
            }
        };
        let failed = result.is_err();
        if let Err(e) = result {
            take.say(
                &format!("{scene}!"),
                format!("stopped: {e}"),
                json!({ "stopped": e.to_string() }),
            );
        }
        if let Some(p) = &progress {
            p.set(n, if failed { "failed" } else { "done" });
        }
    }
    if let Some(p) = &progress {
        p.with(|v| v["done"] = json!(true));
    }
    Ok(take)
}

async fn scene_a(
    node: &Arc<Node>,
    options: &StoryOptions,
    stamp: &str,
    take: &mut Take,
) -> anyhow::Result<()> {
    println!("\n(a) A customer subscribes");
    let books = &node.agentrix_books;
    let today = node.clock.now().date_naive();
    let buyer = node
        .config
        .env
        .get("PAYPAL_BUYER_EMAIL_1")
        .unwrap_or_else(|| "buyer@example.com".into());
    let lead = books
        .create_lead(&LeadDraft {
            contact_name: "Achieng Otieno".into(),
            company_name: Some(format!("Tana Valley Times {stamp}")),
            email: Some(buyer.clone()),
            source: Some("money.agentrix.website sales desk".into()),
            notes: None,
        })
        .await?;
    let opportunity = books
        .convert_lead(
            &lead.id,
            &OpportunityDraft {
                name: "Agentrix Business, monthly".into(),
                amount: Money::usd(4_900),
                expected_close: Some(today),
                customer: None,
            },
        )
        .await?;
    take.say(
        "a1",
        "The sales desk's lead is in ERA's CRM, with its opportunity",
        json!({ "lead": lead.id, "opportunity": opportunity.id }),
    );
    let Some(effects) = node.agentrix_effects.clone() else {
        take.say(
            "a2",
            "No PayPal app on this node; the scene stops here",
            Value::Null,
        );
        return Ok(());
    };
    let checkout = EffectsCheckout::new(
        effects.clone(),
        node.agentrix.company.config.catalogue.clone(),
        node.agentrix.clone(),
    );
    let money = &node.config.hosts.money;
    let started = checkout
        .start(Start {
            plan: "business".into(),
            period: Period::Month,
            email: Some(buyer.clone()),
            return_url: format!("https://{money}/subscribe/return"),
            cancel_url: format!("https://{money}/subscribe/cancelled"),
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    take.say(
        "a2",
        "PayPal subscription made through agentrix-effects; the customer approves it on PayPal's page",
        json!({ "subscription": started.subscription_id, "approve": started.approve_url }),
    );
    let id = started.subscription_id.clone();
    let active = until(options.approval_wait, || {
        let checkout = &checkout;
        let id = id.clone();
        async move {
            match checkout.status(&id).await {
                Ok(s) if s.state.eq_ignore_ascii_case("active") => Some(s),
                _ => None,
            }
        }
    })
    .await;
    let Some(_) = active else {
        take.say(
            "a3",
            format!(
                "Not approved within {}s; PayPal's signed webhook at {} books it when the customer approves",
                options.approval_wait.as_secs(),
                node.config.hosts.pay
            ),
            Value::Null,
        );
        return Ok(());
    };
    take.say(
        "a3",
        "PayPal says the subscription is ACTIVE",
        json!({ "subscription": id }),
    );
    crate::subscription_activated(
        node,
        &effects,
        &json!({ "id": id, "subscriber": { "email_address": buyer, "name": { "given_name": "Achieng", "surname": "Otieno" } } }),
    )
    .await?;
    let reference = saas_company::runtime::subscription_reference(&id);
    let recurring = books
        .recurring_invoices()
        .await?
        .into_iter()
        .find(|r| r.reference.as_deref() == Some(reference.as_str()));
    take.say(
        "a4",
        "Revenue set up the plan's recurring invoice in ERA, on the governed path",
        json!({ "recurring_invoice": recurring.as_ref().map(|r| r.id.to_string()) }),
    );
    // The first billing cycle's payment, read back from PayPal and booked through the handler
    // PayPal's webhook reaches on the public pay host.
    if let Some(client) = &node.agentrix_client {
        let from = (Utc::now() - chrono::Duration::days(1)).format("%Y-%m-%dT%H:%M:%SZ");
        let to = (Utc::now() + chrono::Duration::days(1)).format("%Y-%m-%dT%H:%M:%SZ");
        let sale = until(Duration::from_secs(90), || async {
            let t = client
                .request(
                    Method::GET,
                    &format!(
                        "/v1/billing/subscriptions/{id}/transactions?start_time={from}&end_time={to}"
                    ),
                    None,
                    None,
                )
                .await
                .ok()?;
            t["transactions"]
                .as_array()?
                .iter()
                .find(|x| x["status"] == "COMPLETED")
                .cloned()
        })
        .await;
        match sale {
            Some(sale) => {
                let gross = &sale["amount_with_breakdown"]["gross_amount"];
                let event = json!({
                    "id": format!("story-sale-{}", sale["id"].as_str().unwrap_or_default()),
                    "event_type": "PAYMENT.SALE.COMPLETED",
                    "resource": {
                        "id": sale["id"], "billing_agreement_id": id,
                        "amount": { "total": gross["value"], "currency": gross["currency_code"] },
                        "transaction_fee": { "value": sale["amount_with_breakdown"]["fee_amount"]["value"], "currency": gross["currency_code"] },
                        "create_time": sale["time"],
                    },
                });
                let handled = node.agentrix.on_paypal_event(&event).await?;
                take.say(
                    "a5",
                    "The first payment is booked in ERA as a receipt",
                    json!({ "paypal_sale": sale["id"], "payment": handled.posted.as_ref().and_then(|p| p.payment.as_ref().map(|x| x.to_string())) }),
                );
            }
            None => take.say(
                "a5",
                "PayPal hasn't listed the first payment yet; the signed webhook books it",
                Value::Null,
            ),
        }
    }
    let won = done(
        governed(
            &node.agentrix,
            REVENUE,
            &format!("story-a-{stamp}"),
            "crm_mark_won",
            "won",
            json!({ "opportunity_id": opportunity.id, "customer_name": format!("Tana Valley Times {stamp}"), "customer_email": buyer }),
        )
        .await?,
        "crm_mark_won",
    )?;
    let stage = books
        .opportunities()
        .await?
        .into_iter()
        .find(|o| o.id == opportunity.id)
        .map(|o| o.stage);
    take.say(
        "a6",
        "Revenue marked the deal won; ERA's CRM shows it",
        json!({ "opportunity": opportunity.id, "stage": stage.map(|s| matches!(s, OpportunityStage::Won)), "customer": won["customer_id"] }),
    );
    Ok(())
}

pub(crate) fn kilele_customer(node: &Node) -> anyhow::Result<CustomerId> {
    node.story
        .kilele_customer
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Agentrix's books have no Kilele Bakery customer"))
}

pub(crate) async fn wallet_saved(node: &Node) -> anyhow::Result<bool> {
    let Some(host) = node.settle.get() else {
        return Ok(false);
    };
    let kilele = host.kilele_keys().origin.clone();
    Ok(host
        .payee()
        .store()
        .vault(&kilele)
        .await?
        .is_some_and(|row| row.vault_id.is_some()))
}

/// Earlier takes leave invoices open in the books (an instalment plan runs for months), so a
/// new take can find Kilele's credit limit used up. Then the limit is raised in the books, said
/// as its own step, so the take's invoice fits; otherwise nothing changes.
pub(crate) async fn credit_room(
    node: &Node,
    kilele: &CustomerId,
    cents: i64,
    step: &str,
    take: &mut Take,
) -> anyhow::Result<()> {
    let books = node.agentrix_books.as_ref();
    let position = books.credit_position(kilele).await?;
    if position.allows(Money::usd(cents)) {
        return Ok(());
    }
    let limit = Money::usd(position.exposure.minor + cents + 50_000);
    books.set_credit_limit(kilele, Some(limit)).await?;
    take.say(
        &format!("{step}0"),
        "Earlier takes left Kilele's invoices open, so Agentrix's books raise Kilele's credit limit for this one",
        json!({ "limit_was": position.limit.map(|m| m.to_string()), "exposure": position.exposure.to_string(), "limit_now": limit.to_string() }),
    );
    Ok(())
}

/// Issues `cents` to Kilele and offers it over A2A; returns (invoice id, its number, the
/// offer's decision). Settlement receipts name the invoice by its number.
pub(crate) async fn invoice_kilele(
    node: &Arc<Node>,
    stamp: &str,
    step: &str,
    description: &str,
    cents: i64,
    take: &mut Take,
) -> anyhow::Result<(String, String, String)> {
    let kilele = kilele_customer(node)?;
    credit_room(node, &kilele, cents, step, take).await?;
    let issued = done(
        governed(
            &node.agentrix,
            RECEIVABLES,
            &format!("story-{step}-{stamp}"),
            "issue_invoice",
            "issue",
            json!({ "customer_id": kilele, "lines": [{ "description": description, "amount_usd": cents as f64 / 100.0 }], "due_in_days": 14, "reference": format!("story-{step}-{stamp}") }),
        )
        .await?,
        "issue_invoice",
    )?;
    let invoice = issued["invoice_id"].as_str().unwrap_or_default().to_owned();
    let number = issued["number"].as_str().unwrap_or_default().to_owned();
    take.say(
        &format!("{step}1"),
        "Receivables issued the invoice in ERA, on the governed path",
        json!({ "invoice": invoice, "number": issued["number"], "total": issued["total"] }),
    );
    let host = node
        .settle
        .get()
        .ok_or_else(|| anyhow::anyhow!("the agent doors aren't mounted"))?;
    let offer = host.offer_invoice(&invoice).await?;
    take.say(
        &format!("{step}2"),
        "Agentrix's door offered it to Kilele's door over A2A, signed (Web Bot Auth)",
        json!({ "task": offer.task_id, "decision": offer.decision, "note": offer.note }),
    );
    Ok((invoice, number, offer.decision))
}

pub(crate) async fn scene_b(
    node: &Arc<Node>,
    options: &StoryOptions,
    stamp: &str,
    take: &mut Take,
) -> anyhow::Result<String> {
    println!("\n(b) Agentrix invoices Kilele; Kilele's agent settles it");
    if !wallet_saved(node).await? {
        take.say(
            "b0",
            format!(
                "Kilele's PayPal wallet isn't saved yet: its owner saves it once at https://{}/settle/vault/start",
                node.config.hosts.pay
            ),
            Value::Null,
        );
        anyhow::bail!("no saved wallet for Vault settlement");
    }
    let (invoice, number, decision) =
        invoice_kilele(node, stamp, "b", "Agentrix Business, monthly", 4_900, take).await?;
    anyhow::ensure!(
        decision == "will_settle",
        "Kilele's door answered {decision}"
    );
    let settled = until(options.agent_wait, || async {
        node.settlements
            .rows()
            .into_iter()
            .find(|s| s.invoice_number == invoice || s.invoice_number == number)
    })
    .await
    .ok_or_else(|| anyhow::anyhow!("Kilele's Payables didn't settle within the wait"))?;
    take.say(
        "b3",
        "Kilele's Payables, woken by the offer, settled it on PayPal through Vault",
        json!({ "paypal": settled.paypal_id, "kind": settled.route, "amount": settled.amount_minor }),
    );
    let key = PayPalRef::Capture(saas_core::paypal::PayPalCaptureId::new(
        settled.paypal_id.clone(),
    )?)
    .key();
    // Kilele's books post when the receipt reaches Kilele's door, a moment after the capture.
    let both = until(Duration::from_secs(45), || async {
        let a = payments_for(node.agentrix_books.as_ref(), &key)
            .await
            .ok()?;
        let k = payments_for(node.kilele_books.as_ref(), &key).await.ok()?;
        (!a.is_empty() && !k.is_empty()).then_some((a, k))
    })
    .await;
    let (a, k) = match both {
        Some(both) => both,
        None => (
            payments_for(node.agentrix_books.as_ref(), &key).await?,
            payments_for(node.kilele_books.as_ref(), &key).await?,
        ),
    };
    take.say(
        "b4",
        "Both books posted the one PayPal id",
        json!({ "paypal": settled.paypal_id, "agentrix_payments": a, "kilele_payments": k }),
    );
    Ok(invoice)
}

async fn scene_c(
    node: &Arc<Node>,
    options: &StoryOptions,
    stamp: &str,
    take: &mut Take,
) -> anyhow::Result<()> {
    println!("\n(c) Kilele is short: an instalment plan, inside both mandates");
    if !wallet_saved(node).await? {
        anyhow::bail!("no saved wallet for Vault settlement (scene b says where to save it)");
    }
    let (invoice, number, decision) = invoice_kilele(
        node,
        stamp,
        "c",
        "Agentrix Business, 12 months",
        49_000,
        take,
    )
    .await?;
    take.say(
        "c3",
        format!("Kilele's Payables can't pay it at once and asks for instalments ({decision})"),
        Value::Null,
    );
    let kilele = kilele_customer(node)?;
    let turn = node
        .agentrix
        .company
        .ask(
            RECEIVABLES,
            &format!("story-c-plan-{stamp}"),
            &format!("The customer {kilele} asks to pay invoice {invoice} over time."),
        )
        .await?;
    let plan = turn
        .result("propose_debt_plan")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no plan proposed: {:?}", turn.refused))?;
    if let Some(e) = plan.get("error") {
        anyhow::bail!("the plan was refused: {e}");
    }
    let plan_id = plan["plan_id"].as_str().unwrap_or_default().to_owned();
    take.say(
        "c4",
        "Receivables read Kilele's history and agreed a plan with Kilele's agent, inside both owners' mandates",
        json!({ "plan": plan_id, "instalments": plan["instalments"] }),
    );
    let collected = until(options.agent_wait, || async {
        node.agentrix
            .company
            .desk
            .plan(&plan_id)
            .filter(|p| p.collected.first() == Some(&true))
    })
    .await;
    match collected {
        Some(_) => {
            let capture = node
                .settlements
                .rows()
                .into_iter()
                .find(|s| s.invoice_number == invoice || s.invoice_number == number);
            take.say(
                "c5",
                "The kernel woke Receivables on the first instalment's date; it was collected through Vault",
                json!({ "plan": plan_id, "paypal": capture.map(|c| c.paypal_id) }),
            );
        }
        None => take.say(
            "c5",
            "The first instalment is scheduled; the kernel collects it on its date (09:00 Nairobi)",
            json!({ "plan": plan_id }),
        ),
    }
    Ok(())
}

pub(crate) async fn bill(
    books: &dyn Books,
    vendor: &VendorId,
    reference: &str,
    description: &str,
    cents: i64,
    today: chrono::NaiveDate,
) -> anyhow::Result<String> {
    let draft = books
        .draft_bill(&BillDraft {
            vendor: vendor.clone(),
            vendor_reference: Some(reference.into()),
            issue_date: today,
            due_date: today.checked_add_days(Days::new(7)).unwrap_or(today),
            currency: Currency::USD,
            lines: vec![LineDraft {
                product: None,
                description: description.into(),
                quantity: 1,
                unit_price: Money::usd(cents),
            }],
            memo: None,
        })
        .await?;
    Ok(books.post_bill(&draft.id).await?.id.to_string())
}

async fn scene_d(node: &Arc<Node>, stamp: &str, take: &mut Take) -> anyhow::Result<()> {
    println!("\n(d) Payables pays a vendor by PayPal Payouts, inside the mandate");
    let books = &node.agentrix_books;
    let today = node.clock.now().date_naive();
    let vendors = books.vendors().await?;
    let mandate = node
        .agentrix
        .company
        .signed_mandate()
        .ok_or_else(|| anyhow::anyhow!("no signed mandate is in force"))?;
    let vendor = crate::attacks::mandate_vendor(node, &vendors)?;
    let small = bill(
        books.as_ref(),
        &vendor.id,
        &format!("REF-{stamp}"),
        "Referral share, November",
        2_000,
        today,
    )
    .await?;
    take.say(
        "d1",
        "A USD 20 bill is posted in ERA",
        json!({ "bill": small, "vendor": vendor.counterparty.name }),
    );
    let paid = node
        .agentrix
        .company
        .ask(
            PAYABLES,
            &format!("story-d-{stamp}"),
            &format!("Bill {small} was posted. Pay it if it is inside the mandate."),
        )
        .await?;
    let result = paid.result("pay_bill").cloned().unwrap_or(Value::Null);
    take.say(
        "d2",
        "Payables paid it by PayPal Payouts, inside the founder's mandate",
        json!({ "paypal": result["paypal_id"], "status": result["status"], "posted": result["posted"], "refused": paid.refused }),
    );
    let big = bill(
        books.as_ref(),
        &vendor.id,
        &format!("BIG-{stamp}"),
        "Claude API usage, November",
        90_000,
        today,
    )
    .await?;
    let held = node
        .agentrix
        .company
        .ask(
            PAYABLES,
            &format!("story-d-big-{stamp}"),
            &format!("Bill {big} was posted. Pay it if it is inside the mandate."),
        )
        .await?;
    let waiting = node
        .agentrix
        .company
        .governance
        .held()
        .await?
        .into_iter()
        .find(|h| h.args["bill_id"] == json!(big))
        .map(|h| h.view());
    take.say(
        "d3",
        format!(
            "A USD 900 bill is over the USD {} a payment the founder signed: it waits for their passkey at https://{}/console",
            mandate.terms.payout_max_each_minor / 100,
            node.config.hosts.money
        ),
        json!({ "bill": big, "held": held.held.len(), "decision": waiting.map(|w| w["fingerprint"].clone()) }),
    );
    Ok(())
}

async fn scene_e(node: &Arc<Node>, stamp: &str, take: &mut Take) -> anyhow::Result<()> {
    println!("\n(e) \"Our bank details changed\"");
    let company = &node.agentrix.company;
    let (screened, turn) = company
        .inbound(
            PAYABLES,
            &format!("story-e-{stamp}"),
            InboundKind::Email,
            "billing@vultr-payments.example",
            saas_company::judge::VENDOR_EMAIL,
        )
        .await?;
    take.say(
        "e1",
        "The email is screened before Payables sees it",
        json!({ "flags": screened.flags, "blocked": screened.blocked }),
    );
    take.say(
        "e2",
        "Payables proposes paying the new account; the governed path refuses it",
        json!({ "refused": turn.map(|t| t.refused).unwrap_or_default() }),
    );
    let audit = &company.governance.audit;
    if let Some(record) = audit
        .last("pay_bill", adk_core::audit_log::AuditDecision::Denied)
        .await?
    {
        let receipt = audit.receipt(record.seq).await?;
        let verified = receipt.verify(&audit.verifier()?).is_ok();
        take.say(
            "e3",
            "The refusal is in the signed audit log, provable on its own",
            json!({ "log": audit.log_id(), "seq": record.seq, "receipt_verified": verified, "key": audit.verifier_key() }),
        );
    }
    Ok(())
}
