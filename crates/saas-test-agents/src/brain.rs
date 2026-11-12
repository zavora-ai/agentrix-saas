//! What a test agent does on a visit: chosen by a cheap live model (OpenAI-compatible, Vultr's
//! fast tier) within a hard daily budget per agent, or by a plain random pick when the model is
//! unreachable or the day's budget is spent.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{NaiveDate, Utc};
use rand::Rng;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::Config;
use crate::identity::Agent;
use crate::shops::{Kind, Shop};

/// What to do on this visit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// A signed quote.
    Quote,
    /// A signed checkout link.
    Checkout,
    /// A question to the site.
    Ask,
    /// A custom cake held for the owner.
    Custom,
    /// A Supporter month on a newspaper's own subscribe page, paid with the saved buyer.
    Subscribe,
    /// Look around only.
    Browse,
}

/// One line to buy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// The product's sku.
    pub sku: String,
    /// How many.
    pub qty: i64,
}

/// The choice for one visit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    /// What to do.
    pub action: Action,
    /// What to buy.
    #[serde(default)]
    pub lines: Vec<Line>,
    /// A question for the site.
    #[serde(default)]
    pub question: Option<String>,
    /// Why, in a few words.
    #[serde(default)]
    pub why: Option<String>,
    /// Whether the model chose (otherwise a random pick).
    #[serde(default)]
    pub by_model: bool,
}

/// The model and each agent's spend.
pub struct Brain {
    http: reqwest::Client,
    base: Option<String>,
    key: Option<String>,
    model: String,
    daily_usd: f64,
    usd_per_mtok: f64,
    spend_file: PathBuf,
    spend: Mutex<HashMap<String, (NaiveDate, f64)>>,
}

impl std::fmt::Debug for Brain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Brain")
            .field("model", &self.model)
            .field("daily_usd", &self.daily_usd)
            .finish_non_exhaustive()
    }
}

impl Brain {
    /// From the settings; the day's spend so far is read back from the data directory.
    pub fn new(config: &Config, http: reqwest::Client) -> Self {
        let spend_file = config.data_dir.join("model-spend.json");
        let spend = std::fs::read(&spend_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            http,
            base: config.model_base.clone(),
            key: config.model_key.clone(),
            model: config.model.clone(),
            daily_usd: config.daily_model_usd,
            usd_per_mtok: config.usd_per_mtok,
            spend_file,
            spend: Mutex::new(spend),
        }
    }

    /// What `slug` has spent today, in US dollars.
    pub fn spent_today(&self, slug: &str) -> f64 {
        let today = Utc::now().date_naive();
        self.spend
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(slug)
            .filter(|(day, _)| *day == today)
            .map_or(0.0, |(_, usd)| *usd)
    }

    fn add_spend(&self, slug: &str, usd: f64) {
        let today = Utc::now().date_naive();
        let mut spend = self.spend.lock().unwrap_or_else(|e| e.into_inner());
        let entry = spend.entry(slug.to_owned()).or_insert((today, 0.0));
        if entry.0 != today {
            *entry = (today, 0.0);
        }
        entry.1 += usd;
        if let Ok(bytes) = serde_json::to_vec(&*spend) {
            let _ = std::fs::write(&self.spend_file, bytes);
        }
    }

    /// The choice for a visit to `shop` with these `tools` and what the visit found (`items`:
    /// products at a shop, stories at a newspaper), within `budget_usd`. Falls back to a random
    /// pick.
    pub async fn choose(
        &self,
        agent: &Agent,
        shop: &Shop,
        tools: &[String],
        items: &[Value],
        budget_usd: f64,
    ) -> Choice {
        let allowed = allowed(tools, shop);
        if self.spent_today(&agent.slug) < self.daily_usd {
            match self
                .ask_model(agent, shop, &allowed, items, budget_usd)
                .await
            {
                Ok(choice) if allowed.contains(&choice.action) => return choice,
                Ok(choice) => {
                    tracing::info!(agent = %agent.slug, action = ?choice.action, "test agents: the model chose an action this site doesn't have");
                }
                Err(e) => {
                    tracing::warn!(agent = %agent.slug, error = %e, "test agents: model unavailable; choosing at random")
                }
            }
        }
        random_choice(shop, &allowed, items)
    }

    async fn ask_model(
        &self,
        agent: &Agent,
        shop: &Shop,
        allowed: &[Action],
        items: &[Value],
        budget_usd: f64,
    ) -> anyhow::Result<Choice> {
        let (Some(base), Some(key)) = (&self.base, &self.key) else {
            anyhow::bail!("no model configured");
        };
        let actions: Vec<Value> = allowed.iter().map(|a| json!(a)).collect();
        let (seen, user) = if shop.kind == Kind::News {
            let stories: Vec<Value> = items
                .iter()
                .take(10)
                .map(|s| json!({"headline": s["headline"], "date": s["date"], "subscriber_only": s["subscriber"]}))
                .collect();
            (
                "Stories from your search are below. Use no skus: this site sells nothing at its door.",
                format!("Stories: {}", serde_json::to_string(&stories)?),
            )
        } else {
            let catalogue: Vec<Value> = items
                .iter()
                .take(15)
                .map(|i| json!({"sku": i["sku"], "name": i["name"], "price": i["price"], "stock": i["stock"]["words"]}))
                .collect();
            (
                "Use only skus from the catalogue below.",
                format!("Catalogue: {}", serde_json::to_string(&catalogue)?),
            )
        };
        let system = format!(
            "You are {name}, a test customer agent Agentrix runs on its sample sites ({persona}). You are not a person. \
             This visit is to {about} What fits here: {intents} \
             Choose what to do. Answer with one JSON object only, no prose: \
             {{\"action\": one of {actions}, \"lines\": [{{\"sku\": \"...\", \"qty\": 1}}], \"question\": \"...\", \"why\": \"a few words\"}}. \
             {seen} Keep any order within about USD {budget:.0} (prices are in Kenya shillings; 1 USD is {rate} KES). \
             When the action is ask, ask one short, natural question about this site's own goods or stories, never about things it doesn't offer.",
            name = agent.name,
            persona = agent.persona,
            about = shop.about,
            intents = shop.intents,
            actions = serde_json::to_string(&actions)?,
            budget = budget_usd,
            rate = crate::shops::KES_PER_USD,
        );
        let answer: Value = self
            .http
            .post(format!("{}/chat/completions", base.trim_end_matches('/')))
            .bearer_auth(key)
            .json(&json!({
                "model": self.model,
                "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
                "max_tokens": 250,
                "temperature": 0.8,
            }))
            .timeout(std::time::Duration::from_secs(40))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let tokens = answer["usage"]["total_tokens"].as_f64().unwrap_or(1500.0);
        self.add_spend(&agent.slug, tokens * self.usd_per_mtok / 1_000_000.0);
        let text = answer["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default();
        let json =
            first_object(text).ok_or_else(|| anyhow::anyhow!("no JSON in the model's answer"))?;
        let mut choice: Choice = serde_json::from_str(json)?;
        choice.by_model = true;
        choice.lines.retain(|l| {
            l.qty > 0
                && items
                    .iter()
                    .any(|i| i["sku"].as_str() == Some(l.sku.as_str()) && buyable(i))
        });
        for l in &mut choice.lines {
            l.qty = l.qty.min(3);
        }
        if matches!(choice.action, Action::Quote | Action::Checkout) && choice.lines.is_empty() {
            anyhow::bail!("the model chose no lines from the catalogue");
        }
        Ok(choice)
    }
}

/// The actions this site offers: its door's tools, and a newspaper's subscribe page.
pub fn allowed(tools: &[String], shop: &Shop) -> Vec<Action> {
    let has = |t: &str| tools.iter().any(|x| x == t);
    let mut out = vec![Action::Browse];
    if shop.kind == Kind::News {
        out.push(Action::Subscribe);
    }
    if has("quote.create") {
        out.push(Action::Quote);
    }
    if has("checkout.create") {
        out.push(Action::Checkout);
    }
    if has("ask_site") {
        out.push(Action::Ask);
    }
    if has("request_custom_order") {
        out.push(Action::Custom);
    }
    out
}

/// A random pick by `shop`'s own weights: at a shop, a line from what the visit found (in
/// stock and not free); a question from the shop's own, or about a story the visit found.
pub fn random_choice(shop: &Shop, allowed: &[Action], items: &[Value]) -> Choice {
    let mut rng = rand::thread_rng();
    let pool: Vec<(Action, u32)> = shop
        .weights
        .iter()
        .copied()
        .filter(|(a, _)| allowed.contains(a))
        .collect();
    let total: u32 = pool.iter().map(|(_, w)| w).sum();
    let mut roll = rng.gen_range(0..total.max(1));
    let mut action = Action::Browse;
    for (a, w) in &pool {
        if roll < *w {
            action = *a;
            break;
        }
        roll -= w;
    }
    let buyable: Vec<&Value> = items.iter().filter(|i| buyable(i)).collect();
    let lines = buyable
        .choose(&mut rng)
        .and_then(|i| i["sku"].as_str())
        .map(|sku| {
            vec![Line {
                sku: sku.to_owned(),
                qty: rng.gen_range(1..=2),
            }]
        })
        .unwrap_or_default();
    if matches!(action, Action::Quote | Action::Checkout) && lines.is_empty() {
        action = if allowed.contains(&Action::Ask) {
            Action::Ask
        } else {
            Action::Browse
        };
    }
    let story = items
        .iter()
        .filter_map(|s| s["headline"].as_str())
        .collect::<Vec<_>>()
        .choose(&mut rng)
        .map(|h| format!("What's the latest on \u{201c}{h}\u{201d}?"));
    let question = match story {
        Some(q) if rng.gen_bool(0.4) => Some(q),
        _ => shop.questions.choose(&mut rng).map(|q| (*q).to_owned()),
    };
    Choice {
        action,
        lines,
        question,
        why: Some("random pick".into()),
        by_model: false,
    }
}

/// Whether a catalogue item can go in an order: priced (not the free sample) and not out of
/// stock.
pub fn buyable(item: &Value) -> bool {
    item["sku"].is_string()
        && item["price_minor"].as_i64().is_some_and(|p| p > 0)
        && item["stock"]["left"].as_i64() != Some(0)
}

fn first_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in text[start..].char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_json_object_is_read_out_of_prose() {
        let text =
            "Sure! {\"action\": \"quote\", \"lines\": [{\"sku\": \"a{b}\", \"qty\": 2}]} done";
        let choice: Choice = serde_json::from_str(first_object(text).unwrap()).unwrap();
        assert_eq!(choice.action, Action::Quote);
        assert_eq!(choice.lines[0].sku, "a{b}");
    }

    fn shop(slug: &str) -> &'static Shop {
        crate::shops::by_slug(slug).unwrap()
    }

    #[test]
    fn a_random_pick_at_the_newspaper_reads_asks_or_subscribes() {
        let tana = shop("tana-valley-times-sample");
        let tools = ["ask_site".to_owned(), "stories.search".to_owned()];
        let stories = vec![json!({"headline": "Kora Junction bridge reopens to lorries"})];
        let mut subscribed = false;
        for _ in 0..200 {
            let c = random_choice(tana, &allowed(&tools, tana), &stories);
            assert!(
                matches!(c.action, Action::Ask | Action::Browse | Action::Subscribe),
                "{c:?}"
            );
            subscribed |= c.action == Action::Subscribe;
            let q = c.question.unwrap();
            assert!(
                tana.questions.contains(&q.as_str()) || q.contains("Kora Junction"),
                "{q}"
            );
        }
        assert!(subscribed);
    }

    #[test]
    fn a_random_pick_at_a_shop_buys_only_what_is_priced_and_in_stock() {
        let classy = shop("classy-perfumes-sample");
        let tools: Vec<String> = ["catalog.search", "quote.create", "checkout.create", "ask_site"]
            .map(str::to_owned)
            .to_vec();
        let allowed = allowed(&tools, classy);
        assert!(!allowed.contains(&Action::Subscribe) && !allowed.contains(&Action::Custom));
        let items = vec![
            json!({"sku": "sample-2ml", "price_minor": 0, "stock": {"left": null}}),
            json!({"sku": "lime-market", "price_minor": 400_000, "stock": {"left": 5}}),
            json!({"sku": "amber-noir", "price_minor": 640_000, "stock": {"left": 0}}),
        ];
        for _ in 0..100 {
            let c = random_choice(classy, &allowed, &items);
            for l in &c.lines {
                assert_eq!(l.sku, "lime-market");
            }
            if let Some(q) = &c.question {
                assert!(classy.questions.contains(&q.as_str()), "{q}");
            }
        }
    }
}
