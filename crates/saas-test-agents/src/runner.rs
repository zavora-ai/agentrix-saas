//! Each test agent's visits: every 20–60 minutes it goes to a sample site, mostly the shops that
//! sell ([`crate::shops`]), looks around anonymously with that site's own words, then does what
//! its model chooses from what fits there: bakes or a custom cake at the bakery, a fragrance or a
//! gift set at the perfume shop, a story, a question or a Supporter month at the newspaper. It
//! signs as itself where the door needs a proven agent. About 2% of visits are an impostor's (a
//! name only, or a signature by a key its directory doesn't publish), which the door must refuse.

use std::sync::Arc;
use std::time::Duration;

use adk_auth::web_bot_auth::Ed25519SigningKey;
use chrono::Utc;
use rand::Rng;
use rand::seq::SliceRandom;
use serde_json::{Value, json};

use crate::activity::Log;
use crate::brain::{Action, Brain, Choice};
use crate::config::{Config, LABEL};
use crate::identity::Agent;
use crate::mcp::{Door, ToolAnswer};
use crate::pay::Buyers;
use crate::shop::Subscribed;
use crate::shops::{self, KES_PER_USD, Kind, Shop};

/// What every agent shares.
pub struct Shared {
    /// Settings.
    pub config: Config,
    /// The HTTP client.
    pub http: reqwest::Client,
    /// The model.
    pub brain: Brain,
    /// The activity log.
    pub log: Log,
    /// The saved sandbox buyers, when PayPal is configured.
    pub buyers: Option<Buyers>,
    /// When the run started.
    pub started: chrono::DateTime<Utc>,
}

/// Runs one agent until the run's time is up.
pub async fn run(shared: Arc<Shared>, agent: Agent) {
    let mut first = true;
    loop {
        let wait = {
            let mut rng = rand::thread_rng();
            if first {
                Duration::from_secs(rng.gen_range(30..240))
            } else {
                let (lo, hi) = shared.config.every_min;
                Duration::from_secs(rng.gen_range(lo.max(1) * 60..=hi.max(lo.max(1)) * 60))
            }
        };
        first = false;
        tokio::time::sleep(wait).await;
        if Utc::now() > shared.started + shared.config.run_for {
            shared.log.record(
                &agent.slug,
                "",
                "stopped",
                true,
                "The run's time is up; this agent stops.",
            );
            return;
        }
        if shared.config.is_off() {
            continue;
        }
        if let Err(e) = visit(&shared, &agent).await {
            shared
                .log
                .record(&agent.slug, "", "error", false, format!("{e:#}"));
        }
    }
}

/// One visit, to a site picked by the agent's weights.
pub async fn visit(shared: &Shared, agent: &Agent) -> anyhow::Result<()> {
    let shop = shops::pick(&agent.slug, &mut rand::thread_rng());
    visit_shop(shared, agent, shop).await
}

/// One visit to `shop`.
pub async fn visit_shop(shared: &Shared, agent: &Agent, shop: &Shop) -> anyhow::Result<()> {
    let site = shared.config.origin(shop.slug);
    let impostor = rand::thread_rng().gen_bool(shared.config.impostor_rate.clamp(0.0, 1.0));
    let mut door = Door::new(shared.http.clone(), &site, &agent.user_agent());
    door.initialize(&agent.display()).await?;
    let tools_list = door.tools().await?;
    let tools: Vec<String> = tools_list
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect();
    if impostor && tools.iter().any(|t| t == "quote.create") {
        return impostor_visit(shared, agent, &mut door, &site).await;
    }

    // Look around, anonymously, with this site's own words.
    let (query, budget) = {
        let mut rng = rand::thread_rng();
        (shop.search(&mut rng), shop.budget(&mut rng))
    };
    let items: Vec<Value> = if tools.iter().any(|t| t == "catalog.search") {
        let max_price = (budget * KES_PER_USD).floor();
        let found = door
            .call(
                "catalog.search",
                json!({"query": query, "max_price": max_price}),
                None,
            )
            .await?;
        let mut items = found.structured["items"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut words = format!(
            "catalog.search \"{query}\" up to KSh {max_price:.0}: {} items",
            items.len()
        );
        if items.is_empty() && !query.is_empty() {
            let all = door
                .call("catalog.search", json!({"query": "", "max_price": max_price}), None)
                .await?;
            items = all.structured["items"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            words = format!("{words}; {} items within that price", items.len());
        }
        shared.log.record(&agent.slug, &site, "browse", true, words);
        items
    } else if tools.iter().any(|t| t == "stories.search") {
        let found = door
            .call("stories.search", json!({"query": query}), None)
            .await?;
        let stories = found.structured["stories"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        shared.log.record(
            &agent.slug,
            &site,
            "browse",
            !found.is_error,
            format!("stories.search \"{query}\": {} stories", stories.len()),
        );
        stories
    } else {
        Vec::new()
    };

    let choice = shared
        .brain
        .choose(agent, shop, &tools, &items, budget)
        .await;
    act(shared, agent, &mut door, shop, &tools_list, choice, budget).await
}

async fn act(
    shared: &Shared,
    agent: &Agent,
    door: &mut Door,
    shop: &Shop,
    tools: &[Value],
    choice: Choice,
    budget: f64,
) -> anyhow::Result<()> {
    let origin = shared.config.origin(shop.slug);
    let site = origin.as_str();
    let by = if choice.by_model { "model" } else { "random" };
    let signer = agent.signer()?;
    match choice.action {
        Action::Browse => {
            shared.log.record(
                &agent.slug,
                site,
                "browse",
                true,
                format!("looked around only ({by})"),
            );
        }
        Action::Ask => {
            let question = choice
                .question
                .or_else(|| shop.questions.first().map(|q| (*q).to_owned()))
                .unwrap_or_else(|| "What do you offer?".into());
            let answer = door
                .call("ask_site", json!({"question": question}), None)
                .await?;
            shared.log.record(
                &agent.slug,
                site,
                "ask",
                !answer.is_error,
                format!("asked \"{question}\" ({by})"),
            );
        }
        Action::Quote | Action::Checkout => {
            let mut lines = choice.lines.clone();
            let mut quote = door
                .call("quote.create", json!({"lines": lines}), Some(&signer))
                .await?;
            // Within budget, or one of the first line, or nothing.
            if !quote.is_error && usd(&quote) > budget {
                lines.truncate(1);
                if let Some(l) = lines.first_mut() {
                    l.qty = 1;
                }
                quote = door
                    .call("quote.create", json!({"lines": lines}), Some(&signer))
                    .await?;
            }
            shared.log.record(
                &agent.slug,
                site,
                "quote",
                !quote.is_error,
                quote_words(&quote, &lines, by),
            );
            if quote.is_error || choice.action == Action::Quote {
                return Ok(());
            }
            if usd(&quote) > budget {
                shared.log.record(
                    &agent.slug,
                    site,
                    "checkout",
                    true,
                    format!("over the USD {budget:.0} budget; no checkout"),
                );
                return Ok(());
            }
            // Collection at the bakery; delivery to a zone the door lists elsewhere.
            let offered = door
                .call("delivery.slots", json!({}), None)
                .await
                .ok()
                .filter(|a| !a.is_error);
            let slot = offered.as_ref().and_then(|a| {
                a.structured["slots"][0]["slot_id"]
                    .as_str()
                    .map(str::to_owned)
            });
            let zone = offered
                .as_ref()
                .and_then(|a| a.structured["zones"][0]["zone"].as_str().map(str::to_owned));
            let bakery = shop.kind == Kind::Bakery;
            let mut args = json!({"lines": lines, "fulfilment": if bakery { "collection" } else { "delivery" }});
            if let (false, Some(z)) = (bakery, &zone) {
                args["zone"] = json!(z);
            }
            let checkout = door.call("checkout.create", args, Some(&signer)).await?;
            let link = checkout.structured["link"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            shared.log.record(
                &agent.slug,
                site,
                "checkout",
                !checkout.is_error,
                if checkout.is_error {
                    first_line(&checkout.text)
                } else {
                    format!(
                        "checkout link {link}, verified as {}",
                        checkout.structured["agent"]["id"].as_str().unwrap_or("?")
                    )
                },
            );
            if !checkout.is_error {
                pay_after_checkout(shared, agent, site, &link, slot.as_deref(), zone.as_deref())
                    .await;
            }
        }
        Action::Subscribe => subscribe(shared, agent, site, by).await,
        Action::Custom => {
            if cake_from(tools).is_none() {
                shared.log.record(
                    &agent.slug,
                    site,
                    "custom",
                    true,
                    "no cake options at this door",
                );
                return Ok(());
            }
            // Price a few random cakes until the shop's rules call one complete (a choice can
            // break a rule, such as gluten-free on three tiers), then ask for that one.
            let mut complete = None;
            let mut last_words = String::new();
            for _ in 0..CAKE_TRIES {
                let Some(cake) = cake_from(tools) else { break };
                let quote = door
                    .call("quote.create", json!({"cake": cake}), Some(&signer))
                    .await?;
                if !quote.is_error && quote.structured["complete"] == true {
                    complete = Some(cake);
                    break;
                }
                last_words = first_line(&quote.text);
                if let Some(flag) = quote.structured["flags"]
                    .as_array()
                    .and_then(|f| f.iter().find(|f| f["level"] == "error"))
                {
                    last_words = flag["text"].as_str().unwrap_or_default().to_owned();
                }
            }
            let Some(cake) = complete else {
                shared.log.record(
                    &agent.slug,
                    site,
                    "custom",
                    true,
                    format!("no complete cake in {CAKE_TRIES} tries ({last_words}); nothing asked"),
                );
                return Ok(());
            };
            let held = door
                .call(
                    "request_custom_order",
                    json!({
                        "cake": cake,
                        "name": agent.display(),
                        "email": format!("test-agent-{}@example.com", agent.slug),
                        "fulfilment": "collection",
                    }),
                    Some(&signer),
                )
                .await?;
            shared.log.record(
                &agent.slug,
                site,
                "custom",
                !held.is_error,
                if held.is_error {
                    first_line(&held.text)
                } else {
                    format!(
                        "custom cake {} held for the owner",
                        held.structured["order_id"].as_str().unwrap_or("?")
                    )
                },
            );
        }
    }
    Ok(())
}

/// An impostor's visit: a name only, or a signature by a key the named directory doesn't
/// publish. The door must refuse it with a signed receipt; anything else is an alarm.
async fn impostor_visit(
    shared: &Shared,
    agent: &Agent,
    door: &mut Door,
    site: &str,
) -> anyhow::Result<()> {
    let wrong_key = rand::thread_rng().gen_bool(0.5);
    let signer = if wrong_key {
        Some(agent.signer_with(Arc::new(Ed25519SigningKey::generate()?))?)
    } else {
        None
    };
    let items = door
        .call("catalog.search", json!({"query": ""}), None)
        .await?;
    let sku = items.structured["items"][0]["sku"]
        .as_str()
        .unwrap_or("x")
        .to_owned();
    let answer = door
        .call(
            "quote.create",
            json!({"lines": [{"sku": sku, "qty": 1}]}),
            signer.as_ref(),
        )
        .await?;
    let how = if wrong_key {
        "signed with a key its directory doesn't publish"
    } else {
        "a name only"
    };
    let refused = answer.is_error && answer.structured["refused"] == true;
    let receipt = answer.structured["receipt"].is_object();
    shared.log.record(
        &agent.slug,
        site,
        "impostor",
        refused && receipt,
        if refused {
            format!(
                "impostor ({how}) refused: {} {}",
                answer.structured["code"].as_str().unwrap_or("?"),
                if receipt {
                    "with a signed receipt"
                } else {
                    "WITHOUT a receipt"
                }
            )
        } else {
            format!("ALARM: impostor ({how}) was NOT refused")
        },
    );
    Ok(())
}

/// Subscribes for a Supporter month on the newspaper's own page, paid with the agent's saved
/// sandbox buyer; the reason in the log when it can't.
async fn subscribe(shared: &Shared, agent: &Agent, site: &str, by: &str) {
    let Some(buyers) = &shared.buyers else {
        shared.log.record(
            &agent.slug,
            site,
            "subscribe",
            true,
            format!("not subscribed: no sandbox PayPal app is configured here ({by})"),
        );
        return;
    };
    if buyers.vault_id(agent).is_none() {
        shared.log.record(
            &agent.slug,
            site,
            "subscribe",
            true,
            format!(
                "not subscribed: no saved sandbox buyer yet (approve one at {}/paypal/save)",
                agent.origin
            ),
        );
        return;
    }
    let (ok, words) = match crate::shop::subscribe(&shared.http, buyers, agent, site).await {
        Ok(Subscribed::Paid { paypal_order }) => (
            true,
            format!(
                "subscribed for a Supporter month with its saved sandbox buyer (PayPal order …{}) ({by})",
                last4(&paypal_order)
            ),
        ),
        Ok(Subscribed::StandIn) => (
            true,
            format!("subscribed with the sample payment; PayPal isn't configured on this site ({by})"),
        ),
        Ok(Subscribed::Already) => (
            true,
            format!("already a Supporter subscriber; nothing paid ({by})"),
        ),
        Err(e) => (false, format!("not subscribed: {e:#}")),
    };
    shared.log.record(&agent.slug, site, "subscribe", ok, words);
}

fn last4(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

/// Pays for the checkout as the agent's saved sandbox buyer, with no one present; the reason
/// in the log when it can't.
async fn pay_after_checkout(
    shared: &Shared,
    agent: &Agent,
    site: &str,
    link: &str,
    slot: Option<&str>,
    zone: Option<&str>,
) {
    let Some(buyers) = &shared.buyers else {
        shared.log.record(
            &agent.slug,
            site,
            "pay",
            true,
            "not paid: no sandbox PayPal app is configured here",
        );
        return;
    };
    if buyers.vault_id(agent).is_none() {
        shared.log.record(
            &agent.slug,
            site,
            "pay",
            true,
            format!(
                "not paid: no saved sandbox buyer yet (approve one at {}/paypal/save)",
                agent.origin
            ),
        );
        return;
    }
    match crate::shop::pay(&shared.http, buyers, agent, site, link, slot, zone).await {
        Ok(paid) => shared.log.record(
            &agent.slug,
            site,
            "pay",
            true,
            format!(
                "paid order {} with its saved sandbox buyer (PayPal order …{})",
                paid.order_id,
                last4(&paid.paypal_order)
            ),
        ),
        Err(e) => shared
            .log
            .record(&agent.slug, site, "pay", false, format!("not paid: {e:#}")),
    }
}

fn usd(quote: &ToolAnswer) -> f64 {
    quote.structured["usd_cents"].as_f64().unwrap_or(0.0) / 100.0
}

fn quote_words(quote: &ToolAnswer, lines: &[crate::brain::Line], by: &str) -> String {
    if quote.is_error {
        return first_line(&quote.text);
    }
    let what: Vec<String> = lines
        .iter()
        .map(|l| format!("{} x{}", l.sku, l.qty))
        .collect();
    format!(
        "quote {} = {} ({}) for {}, verified as {} ({by})",
        what.join(", "),
        quote.structured["total"].as_str().unwrap_or("?"),
        quote.structured["usd"].as_str().unwrap_or("?"),
        LABEL,
        quote.structured["agent"]["id"].as_str().unwrap_or("?"),
    )
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect()
}

/// How many random cakes a visit prices before giving up.
const CAKE_TRIES: usize = 4;

/// Words a customer might put on a cake; cut to the shop's own limit.
const CAKE_MESSAGES: [&str; 5] = [
    "Happy birthday",
    "Congratulations",
    "Well done",
    "Happy anniversary",
    "Karibu",
];

/// What the note on every test cake says.
const CAKE_NOTE: &str = "Test order from a test customer agent (sandbox). No cake is needed.";

/// `text` cut to the schema's `maxLength` for that field, when it has one.
fn fit(text: &str, spec: &Value) -> String {
    match spec["maxLength"].as_u64() {
        Some(max) => text.chars().take(max as usize).collect::<String>().trim_end().to_owned(),
        None => text.to_owned(),
    }
}

/// A cake from the door's own options: each choice field from its enum, a date two to three
/// weeks out, a short message within the shop's own limit, and a note that says it's a test.
fn cake_from(tools: &[Value]) -> Option<Value> {
    let tool = tools.iter().find(|t| t["name"] == "request_custom_order")?;
    let schema = &tool["inputSchema"]["properties"]["cake"];
    let props = schema["properties"].as_object()?;
    let mut rng = rand::thread_rng();
    let mut cake = serde_json::Map::new();
    for (name, spec) in props {
        if let Some(options) = spec["enum"].as_array() {
            if let Some(v) = options.choose(&mut rng) {
                cake.insert(name.clone(), v.clone());
            }
        }
    }
    if props.contains_key("date") {
        let day = Utc::now().date_naive() + chrono::Duration::days(rng.gen_range(14..21));
        cake.insert("date".into(), json!(day.format("%Y-%m-%d").to_string()));
    }
    if let Some(spec) = props.get("message") {
        let words = CAKE_MESSAGES.choose(&mut rng).copied().unwrap_or("Happy birthday");
        cake.insert("message".into(), json!(fit(words, spec)));
    }
    if let Some(spec) = props.get("note") {
        cake.insert("note".into(), json!(fit(CAKE_NOTE, spec)));
    }
    Some(Value::Object(cake))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn door(message_max: u64) -> Vec<Value> {
        vec![json!({
            "name": "request_custom_order",
            "inputSchema": {"properties": {"cake": {"properties": {
                "size": {"type": "string", "enum": ["size.1", "size.2"]},
                "sponge": {"type": "string", "enum": ["sponge.vanilla"]},
                "message": {"type": "string", "maxLength": message_max},
                "date": {"type": "string", "format": "date"},
                "note": {"type": "string", "maxLength": 300},
            }}}},
        })]
    }

    #[test]
    fn a_cake_keeps_to_the_shops_own_limits_and_says_its_a_test_in_the_note() {
        for _ in 0..50 {
            let cake = cake_from(&door(22)).unwrap();
            let message = cake["message"].as_str().unwrap();
            assert!(!message.is_empty() && message.chars().count() <= 22, "{message}");
            assert!(cake["note"].as_str().unwrap().contains("test customer agent"));
            assert_eq!(cake["sponge"], "sponge.vanilla");
            assert!(cake["date"].as_str().unwrap().starts_with("20"));
        }
        // A shop with a smaller plaque still gets a message that fits.
        let cake = cake_from(&door(6)).unwrap();
        assert!(cake["message"].as_str().unwrap().chars().count() <= 6);
        assert!(cake_from(&[json!({"name": "ask_site"})]).is_none());
    }
}
