//! A live check of the PayPal rails against PayPal's **sandbox**: real calls through
//! `agentrix-effects`, printing every PayPal id as a row for `specs/saas/EVIDENCE.md`.
//!
//! ```text
//! AGENTRIX_PAYPAL_FILE=~/.config/agentrix-saas/paypal-sandbox.json \
//!   scripts/wcargo run -p agentrix-effects --example paypal_sandbox_probe -- run
//! ```
//!
//! - `run`: a product and plan, a subscription (prints the approval link for the sandbox
//!   buyer), a card order captured with PayPal's published test card and partly refunded
//!   through `Effects::refund`, a wallet order (prints its approval link), a payout and its
//!   batch, and PayPal's own answers to a repeated request id and a repeated `sender_batch_id`.
//! - `status`: reads back every subscription and payout the probe made.
//! - `webhook-forged`: a forged event through PayPal's verify-webhook-signature, with the
//!   probe's own made-up webhook id (none of the app's webhooks is touched).
//! - `suspend <sub_…>`, `activate <sub_…>`, `cancel <sub_…>`: after the buyer approved.
//! - `lose-answer <po_…>`: forgets PayPal's answer to a payout, then reads it back.
//! - `capture <order id>`: captures a wallet order the buyer approved, then refunds part of it.
//! - `payout <email> <cents>`: one more payout, for example to a sandbox personal account.
//!
//! Sandbox only: it refuses any other environment. The probe's own state (which PayPal ids
//! are its) is in `PROBE_DB`, by default `~/.config/agentrix-saas/probe.db`. It never prints
//! the credentials.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use agentrix_effects::paypal::{self, Environment, PayPalClient, PayPalConfig};
use agentrix_effects::{Effects, PaymentRequest, PayoutRequest, PlanPrice, SubscriptionStart};
use agentrix_host::testing::MemoryReceipts;
use agentrix_types::{Money, SiteId};
use anyhow::{Context, bail};
use reqwest::Method;
use serde_json::{Value, json};

const SITE: &str = "site_agentrix_saas_probe";

/// PayPal's published sandbox test cards, tried in order.
const TEST_CARDS: [&str; 3] = ["4111111111111111", "4032039317984658", "5555555555554444"];

fn usd(minor: i64) -> Money {
    Money {
        currency: "USD".into(),
        minor,
    }
}

fn eat() -> String {
    let offset = chrono::FixedOffset::east_opt(3 * 3600).expect("EAT is UTC+3");
    chrono::Utc::now()
        .with_timezone(&offset)
        .format("%a %d %b %H:%M:%S EAT")
        .to_string()
}

/// One evidence row: when, which API, what, PayPal's id, PayPal's status.
fn row(api: &str, what: &str, id: &str, status: &str) {
    println!("ROW | {} | {api} | {what} | `{id}` | {status} |", eat());
}

fn note(text: &str) {
    println!("     {text}");
}

struct Probe {
    effects: Effects,
    config: PayPalConfig,
    client: Arc<PayPalClient>,
    pool: sqlx::SqlitePool,
    receipts: Arc<MemoryReceipts>,
}

fn site() -> SiteId {
    SiteId::new(SITE)
}

fn approval(effect: &agentrix_types::EffectRequest) -> anyhow::Result<Value> {
    let fp = effect.fingerprint()?.to_string();
    Ok(
        json!({ "method": "probe", "account": "sandbox-probe", "fingerprint": fp.trim_start_matches("sha256:") }),
    )
}

impl Probe {
    async fn open() -> anyhow::Result<Self> {
        let home = std::env::var("HOME").context("HOME")?;
        let file = std::env::var("AGENTRIX_PAYPAL_FILE")
            .unwrap_or_else(|_| format!("{home}/.config/agentrix-saas/paypal-sandbox.json"));
        let config = PayPalConfig::from_file(std::path::Path::new(&file))?
            .with_context(|| format!("no PayPal keys in {file}"))?;
        if config.environment != Environment::Sandbox {
            bail!("the probe runs against PayPal's sandbox only");
        }
        let db = std::env::var("PROBE_DB")
            .unwrap_or_else(|_| format!("{home}/.config/agentrix-saas/probe.db"));
        let options = sqlx::sqlite::SqliteConnectOptions::from_str(&format!("sqlite://{db}"))?
            .create_if_missing(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        agentrix_effects::migrate(&pool).await?;
        let client = Arc::new(PayPalClient::new(config.clone(), reqwest::Client::new()));
        let receipts = Arc::new(MemoryReceipts::default());
        let effects = Effects::new(pool.clone(), Some(client.clone()), receipts.clone());
        Ok(Self {
            effects,
            config,
            client,
            pool,
            receipts,
        })
    }

    async fn plan_and_subscription(&self) -> anyhow::Result<()> {
        println!("\n== Subscriptions: product, plan, subscription");
        let plan = PlanPrice {
            plan_id: "agentrix-saas-probe-starter".into(),
            version: 1,
            name: "Agentrix SaaS Starter (sandbox probe)".into(),
            monthly: usd(1900),
            annual: None,
        };
        let plan_id = self.effects.ensure_plan(&plan).await?;
        let (product,): (String,) =
            sqlx::query_as("SELECT product_id FROM paypal_plans WHERE paypal_plan_id = ?")
                .bind(&plan_id)
                .fetch_one(&self.pool)
                .await?;
        row(
            "Catalog Products",
            "product \"Agentrix SaaS Starter (sandbox probe)\"",
            &product,
            "created",
        );
        let read = self
            .client
            .request(
                Method::GET,
                &format!("/v1/billing/plans/{plan_id}"),
                None,
                None,
            )
            .await?;
        row(
            "Subscriptions",
            "plan USD 19.00 a month",
            &plan_id,
            read["status"].as_str().unwrap_or("?"),
        );
        let started = self
            .effects
            .start_subscription(SubscriptionStart {
                site: site(),
                plan,
                subscriber_email: None,
                return_url: "https://example.com/agentrix-saas/subscribed".into(),
                cancel_url: "https://example.com/agentrix-saas/cancelled".into(),
                starts_at: None,
            })
            .await?;
        let read = self
            .client
            .request(
                Method::GET,
                &format!("/v1/billing/subscriptions/{}", started.provider_ref),
                None,
                None,
            )
            .await?;
        row(
            "Subscriptions",
            &format!("subscription {} for the plan", started.subscription_id),
            &started.provider_ref,
            read["status"].as_str().unwrap_or("?"),
        );
        note(&format!(
            "Approve as the sandbox buyer: {}",
            started.approve_url
        ));
        Ok(())
    }

    /// Orders v2 with `payment_source.card` and a published test card: PayPal captures without
    /// a buyer login. Returns the order id, or `None` when the app takes no cards.
    async fn card_order(&self) -> anyhow::Result<Option<String>> {
        println!("\n== Orders v2: card order, capture, refund");
        let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
        for number in TEST_CARDS {
            let body = json!({
                "intent": "CAPTURE",
                "purchase_units": [{
                    "reference_id": format!("probe-card-{stamp}"),
                    "description": "Agentrix SaaS sandbox probe (test card)",
                    "amount": paypal::amount("USD", 1200),
                }],
                "payment_source": { "card": {
                    "number": number, "expiry": "2030-12", "security_code": "123",
                    "name": "Agentrix SaaS Probe",
                    "billing_address": {
                        "address_line_1": "2211 North First Street", "admin_area_2": "San Jose",
                        "admin_area_1": "CA", "postal_code": "95131", "country_code": "US",
                    },
                }},
            });
            let key = format!(
                "agentrix-saas-probe-card-{stamp}-{}",
                &number[number.len() - 4..]
            );
            let order = match self
                .client
                .request(Method::POST, "/v2/checkout/orders", Some(&body), Some(&key))
                .await
            {
                Ok(order) => order,
                Err(e) => {
                    note(&format!("card …{}: {e}", &number[number.len() - 4..]));
                    continue;
                }
            };
            let order_id = order["id"].as_str().unwrap_or("?").to_owned();
            row(
                "Orders v2",
                &format!(
                    "create order USD 12.00, payment_source.card (test card …{})",
                    &number[number.len() - 4..]
                ),
                &order_id,
                order["status"].as_str().unwrap_or("?"),
            );
            let order = if order["status"] == "COMPLETED" {
                order
            } else {
                self.client
                    .request(
                        Method::POST,
                        &format!("/v2/checkout/orders/{order_id}/capture"),
                        None,
                        Some(&format!("{key}-capture")),
                    )
                    .await?
            };
            let capture = &order["purchase_units"][0]["payments"]["captures"][0];
            let capture_id = capture["id"].as_str().unwrap_or("?").to_owned();
            row(
                "Orders v2",
                "capture",
                &capture_id,
                capture["status"].as_str().unwrap_or("?"),
            );
            if capture["status"] != "COMPLETED" {
                note("capture not COMPLETED; no refund attempted");
                return Ok(Some(order_id));
            }
            self.refund_through_effects(&order_id, &capture_id, 1200, 500, &stamp)
                .await?;
            return Ok(Some(order_id));
        }
        Ok(None)
    }

    /// Records a captured order as a payment and refunds `refund` of it through
    /// `Effects::refund`, with the probe's approval of that exact fingerprint.
    async fn refund_through_effects(
        &self,
        order_id: &str,
        capture_id: &str,
        captured: i64,
        refund: i64,
        stamp: &str,
    ) -> anyhow::Result<()> {
        let payment_id = format!("pay_probe_{stamp}");
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT OR IGNORE INTO customer_payments (id, site_id, order_id, fingerprint, provider, provider_ref, currency, amount_minor, state, capture_id, captured_minor, sandbox, created_at, updated_at)
             VALUES (?, ?, ?, ?, 'paypal', ?, 'USD', ?, 'captured', ?, ?, 1, ?, ?)",
        )
        .bind(&payment_id)
        .bind(SITE)
        .bind(format!("probe-{stamp}"))
        .bind(format!("probe:{order_id}"))
        .bind(order_id)
        .bind(captured)
        .bind(capture_id)
        .bind(captured)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        let payment = self
            .effects
            .payment(&site(), &payment_id)
            .await?
            .context("payment row")?;
        self.refund(&payment, refund).await
    }

    async fn refund(&self, payment: &agentrix_effects::Payment, minor: i64) -> anyhow::Result<()> {
        let effect = self.effects.refund_request(
            payment,
            usd(minor),
            "Agentrix SaaS sandbox probe: partial refund",
        )?;
        let done = self.effects.refund(&effect, &approval(&effect)?).await?;
        let status = match &done.provider_ref {
            Some(id) => self
                .client
                .request(
                    Method::GET,
                    &format!("/v2/payments/refunds/{id}"),
                    None,
                    None,
                )
                .await
                .map(|r| r["status"].as_str().unwrap_or("?").to_owned())
                .unwrap_or_else(|e| format!("read failed: {e}")),
            None => format!("{:?}", done.outcome),
        };
        row(
            "Payments v2",
            &format!(
                "refund USD {}.{:02} of the capture, through Effects::refund ({:?})",
                minor / 100,
                minor % 100,
                done.outcome
            ),
            done.provider_ref.as_deref().unwrap_or("none"),
            &status,
        );
        Ok(())
    }

    async fn wallet_order(&self) -> anyhow::Result<()> {
        println!("\n== Orders v2: PayPal wallet order (buyer approves)");
        let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
        let started = self
            .effects
            .create_payment(PaymentRequest {
                site: site(),
                site_name: "Agentrix SaaS (sandbox probe)".into(),
                order_id: format!("probe-wallet-{stamp}"),
                amount: usd(1200),
                description: "Agentrix SaaS sandbox probe (PayPal wallet)".into(),
                customer: None,
                return_url: "https://example.com/agentrix-saas/paid".into(),
                cancel_url: "https://example.com/agentrix-saas/cancelled".into(),
            })
            .await?;
        let read = self
            .client
            .request(
                Method::GET,
                &format!("/v2/checkout/orders/{}", started.provider_ref),
                None,
                None,
            )
            .await?;
        row(
            "Orders v2",
            "create order USD 12.00, payment_source.paypal, through Effects::create_payment",
            &started.provider_ref,
            read["status"].as_str().unwrap_or("?"),
        );
        note(&format!(
            "Approve as the sandbox buyer, then `capture {}`: {}",
            started.provider_ref, started.approve_url
        ));
        Ok(())
    }

    async fn payout(&self, receiver: &str, minor: i64) -> anyhow::Result<()> {
        println!("\n== Payouts: one payout, its batch and item");
        let effect = self.effects.payout_request(PayoutRequest {
            site: site(),
            agent: "finance".into(),
            receiver: receiver.into(),
            amount: usd(minor),
            note: "Agentrix SaaS sandbox probe: partner payout".into(),
        })?;
        let fp = effect.fingerprint()?.to_string();
        let sender_batch_id = fp.trim_start_matches("sha256:").to_owned();
        let paid = self.effects.payout(&effect, &approval(&effect)?).await?;
        row(
            "Payouts",
            &format!(
                "payout USD {}.{:02} to {receiver}, through Effects::payout ({:?})",
                minor / 100,
                minor % 100,
                paid.outcome
            ),
            paid.batch_id.as_deref().unwrap_or("none"),
            paid.batch_status.as_deref().unwrap_or("?"),
        );
        note(&format!(
            "sender_batch_id = PayPal-Request-Id = {sender_batch_id}"
        ));
        let Some(batch_id) = paid.batch_id.clone() else {
            note(&paid.message);
            return Ok(());
        };

        // PayPal's own idempotency, asked directly: the same request id, then the same
        // sender_batch_id under a new request id.
        let payout = self
            .effects
            .stored_payout(&site(), &paid.payout_id)
            .await?
            .context("payout row")?;
        let body = json!({
            "sender_batch_header": {
                "sender_batch_id": sender_batch_id,
                "email_subject": "You have a payment",
                "email_message": payout.note,
            },
            "items": [{
                "recipient_type": "EMAIL",
                "receiver": payout.receiver,
                "amount": { "value": format!("{}.{:02}", minor / 100, minor % 100), "currency": "USD" },
                "note": payout.note,
                "sender_item_id": payout.id,
            }],
        });
        match self
            .client
            .request(
                Method::POST,
                "/v1/payments/payouts",
                Some(&body),
                Some(&sender_batch_id),
            )
            .await
        {
            Ok(again) => row(
                "Payouts",
                "the same request id again: PayPal answers with",
                again["batch_header"]["payout_batch_id"]
                    .as_str()
                    .unwrap_or("?"),
                again["batch_header"]["batch_status"]
                    .as_str()
                    .unwrap_or("?"),
            ),
            Err(e) => {
                let links = match &e {
                    paypal::PayPalError::Api { links, .. } => links.join(" "),
                    _ => String::new(),
                };
                row(
                    "Payouts",
                    "the same request id again: refused",
                    "USER_BUSINESS_ERROR",
                    &format!("{e}; links: {links}"),
                );
            }
        }
        let fresh_key = format!("{sender_batch_id}-probe-dup");
        match self
            .client
            .request(
                Method::POST,
                "/v1/payments/payouts",
                Some(&body),
                Some(&fresh_key),
            )
            .await
        {
            Ok(second) => {
                row(
                    "Payouts",
                    "the same sender_batch_id, new request id: PayPal TOOK A SECOND BATCH",
                    second["batch_header"]["payout_batch_id"]
                        .as_str()
                        .unwrap_or("?"),
                    second["batch_header"]["batch_status"]
                        .as_str()
                        .unwrap_or("?"),
                );
            }
            Err(e) => {
                let (name, links) = match &e {
                    paypal::PayPalError::Api { name, links, .. } => (name.clone(), links.join(" ")),
                    other => (other.to_string(), String::new()),
                };
                row(
                    "Payouts",
                    "the same sender_batch_id, new request id: refused",
                    &name,
                    &format!("{e}; links: {links}"),
                );
            }
        }

        // Let PayPal process the batch, then read it back.
        tokio::time::sleep(Duration::from_secs(8)).await;
        let read = self
            .effects
            .reconcile_payout(&site(), &paid.payout_id)
            .await?;
        let batch = self.effects.payout_batch(&batch_id).await?;
        row("Payouts", "GET the batch", &batch.batch_id, &batch.status);
        if let Some(item_id) = &read.item_id {
            let item = self.effects.payout_item(item_id).await?;
            row(
                "Payouts",
                &format!(
                    "GET the item (transaction {})",
                    item.transaction_id.as_deref().unwrap_or("none")
                ),
                &item.item_id,
                &item.status,
            );
        }
        note(&read.message);
        Ok(())
    }

    async fn status(&self) -> anyhow::Result<()> {
        println!("\n== Read back");
        let subs: Vec<(String, String)> =
            sqlx::query_as("SELECT id, provider_ref FROM plan_subscriptions WHERE site_id = ?")
                .bind(SITE)
                .fetch_all(&self.pool)
                .await?;
        for (id, provider_ref) in subs {
            let s = self.effects.refresh_subscription(&provider_ref).await?;
            row(
                "Subscriptions",
                &format!("GET subscription {id}"),
                &provider_ref,
                &s.state,
            );
        }
        for payout in self.effects.payouts_for_site(&site()).await? {
            let read = self.effects.reconcile_payout(&site(), &payout.id).await?;
            row(
                "Payouts",
                &format!(
                    "read back payout {} (item {})",
                    payout.id,
                    read.item_id.as_deref().unwrap_or("none")
                ),
                read.batch_id.as_deref().unwrap_or("none"),
                &format!(
                    "batch {} / item {}",
                    read.batch_status.as_deref().unwrap_or("?"),
                    read.item_status.as_deref().unwrap_or("?")
                ),
            );
        }
        Ok(())
    }

    async fn subscription_action(&self, action: &str, id: &str) -> anyhow::Result<()> {
        let reason = "Agentrix SaaS sandbox probe";
        let s = match action {
            "suspend" => {
                self.effects
                    .suspend_subscription(&site(), id, reason)
                    .await?
            }
            "activate" => {
                self.effects
                    .activate_subscription(&site(), id, reason)
                    .await?
            }
            _ => {
                self.effects
                    .cancel_subscription(&site(), id, reason)
                    .await?
            }
        };
        let provider_ref = s.provider_ref.clone().unwrap_or_default();
        let read = self
            .client
            .request(
                Method::GET,
                &format!("/v1/billing/subscriptions/{provider_ref}"),
                None,
                None,
            )
            .await?;
        row(
            "Subscriptions",
            &format!("{action} subscription {id}"),
            &provider_ref,
            read["status"].as_str().unwrap_or("?"),
        );
        Ok(())
    }

    /// A forged event, checked by PayPal's verify-webhook-signature. The webhook id is the
    /// probe's own made-up one, so none of the app's webhooks is touched.
    async fn forged_webhook(&self) -> anyhow::Result<()> {
        println!("\n== Webhooks: a forged event, checked by PayPal");
        let mut config = self.config.clone();
        config.webhook_id = Some("0AGENTRIXSAASPROBE00000000".into());
        let effects = Effects::new(
            self.pool.clone(),
            Some(Arc::new(PayPalClient::new(config, reqwest::Client::new()))),
            self.receipts.clone(),
        );
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let event = json!({
            "id": "WH-AGENTRIX-SAAS-PROBE-FORGED", "event_type": "PAYMENT.CAPTURE.COMPLETED",
            "resource_type": "capture", "create_time": now,
            "resource": { "id": "FORGED-CAPTURE", "status": "COMPLETED" },
        });
        let mut headers = axum::http::HeaderMap::new();
        for (name, value) in [
            ("paypal-auth-algo", "SHA256withRSA".to_owned()),
            (
                "paypal-cert-url",
                "https://api.sandbox.paypal.com/v1/notifications/certs/CERT-360caa42-fca2a594-a5cafa77".to_owned(),
            ),
            ("paypal-transmission-id", uuid::Uuid::new_v4().to_string()),
            ("paypal-transmission-sig", "Zm9yZ2VkIGJ5IHRoZSBwcm9iZQ==".to_owned()),
            ("paypal-transmission-time", now.clone()),
        ] {
            headers.insert(name, value.parse()?);
        }
        let check = json!({
            "auth_algo": "SHA256withRSA",
            "cert_url": "https://api.sandbox.paypal.com/v1/notifications/certs/CERT-360caa42-fca2a594-a5cafa77",
            "transmission_id": headers["paypal-transmission-id"].to_str()?,
            "transmission_sig": "Zm9yZ2VkIGJ5IHRoZSBwcm9iZQ==",
            "transmission_time": now,
            "webhook_id": "0AGENTRIXSAASPROBE00000000",
            "webhook_event": event,
        });
        let answer = match self
            .client
            .request(
                Method::POST,
                "/v1/notifications/verify-webhook-signature",
                Some(&check),
                None,
            )
            .await
        {
            Ok(v) => format!(
                "verification_status {}",
                v["verification_status"].as_str().unwrap_or("?")
            ),
            Err(e) => e.to_string(),
        };
        row(
            "Webhooks",
            "verify-webhook-signature, forged signature, the probe's own made-up webhook id",
            "WH-AGENTRIX-SAAS-PROBE-FORGED",
            &answer,
        );
        let verdict = effects.handle_webhook(&headers, &serde_json::to_vec(&event).unwrap_or_default()).await;
        row(
            "Webhooks",
            "Effects::handle_webhook on the same forged event",
            "WH-AGENTRIX-SAAS-PROBE-FORGED",
            &format!("{verdict:?}"),
        );
        Ok(())
    }

    /// Forgets PayPal's answer to a payout, as if the process died before it arrived (the
    /// row keeps no batch id and reads unknown), then settles it with
    /// `Effects::reconcile_payout`: PayPal must link the first batch, never make a second.
    async fn lose_answer(&self, payout_id: &str) -> anyhow::Result<()> {
        println!("\n== Payouts: a lost answer, settled by reading back");
        let before = self
            .effects
            .stored_payout(&site(), payout_id)
            .await?
            .context("no such payout")?;
        sqlx::query(
            "UPDATE payouts SET state = 'unknown', batch_id = NULL, batch_status = NULL, item_id = NULL, item_status = NULL WHERE id = ?",
        )
        .bind(payout_id)
        .execute(&self.pool)
        .await?;
        let settled = self.effects.reconcile_payout(&site(), payout_id).await?;
        row(
            "Payouts",
            &format!(
                "payout {payout_id} with its answer lost (was batch {}), settled by Effects::reconcile_payout ({:?})",
                before.batch_id.as_deref().unwrap_or("none"),
                settled.outcome
            ),
            settled.batch_id.as_deref().unwrap_or("none"),
            &format!(
                "batch {} / item {}; same batch: {}",
                settled.batch_status.as_deref().unwrap_or("?"),
                settled.item_status.as_deref().unwrap_or("?"),
                settled.batch_id == before.batch_id
            ),
        );
        Ok(())
    }

    async fn capture(&self, order_id: &str) -> anyhow::Result<()> {
        let done = self.effects.capture(&site(), order_id).await?;
        row(
            "Orders v2",
            &format!("capture, through Effects::capture ({:?})", done.outcome),
            done.provider_capture_id.as_deref().unwrap_or("none"),
            &format!("{:?}: {}", done.state, done.message),
        );
        if done.provider_capture_id.is_some() {
            let (id,): (String,) =
                sqlx::query_as("SELECT id FROM customer_payments WHERE provider_ref = ?")
                    .bind(order_id)
                    .fetch_one(&self.pool)
                    .await?;
            let payment = self
                .effects
                .payment(&site(), &id)
                .await?
                .context("payment row")?;
            self.refund(&payment, 500).await?;
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let probe = Probe::open().await?;
    println!("PayPal sandbox probe, {}", eat());
    match args.first().map(String::as_str).unwrap_or("run") {
        "run" => {
            probe.plan_and_subscription().await?;
            if probe.card_order().await?.is_none() {
                note("No test card was accepted: this app may not have card processing.");
            }
            probe.wallet_order().await?;
            let receiver = std::env::var("PROBE_PAYOUT_RECEIVER")
                .unwrap_or_else(|_| "agentrix-saas-partner@example.com".into());
            probe.payout(&receiver, 1000).await?;
        }
        "status" => probe.status().await?,
        "webhook-forged" => probe.forged_webhook().await?,
        "lose-answer" => {
            let id = args.get(1).context("which po_…?")?;
            probe.lose_answer(id).await?;
        }
        action @ ("suspend" | "activate" | "cancel") => {
            let id = args.get(1).context("which sub_…?")?;
            probe.subscription_action(action, id).await?;
        }
        "capture" => {
            let order = args.get(1).context("which order id?")?;
            probe.capture(order).await?;
        }
        "payout" => {
            let receiver = args.get(1).context("to which email?")?;
            let minor: i64 = args.get(2).map_or(Ok(1000), |a| a.parse())?;
            probe.payout(receiver, minor).await?;
        }
        other => bail!("unknown command {other}"),
    }
    println!(
        "\n{} platform receipts signed in memory this run.",
        probe.receipts.issued.lock().map_or(0, |r| r.len())
    );
    Ok(())
}
