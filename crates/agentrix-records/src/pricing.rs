//! Price lists and price rules, and the configurable cake quoted from them.
//!
//! A configurable product's price is the sum of rule lines (`size.2`, `sponge.lemon`,
//! `finish.ombre`, `deco.berries`, `delivery.zone1`), each read from the live price list. Rules
//! carry their constraints in `meta`: `serves`, `notice_days`, `only_sizes`, `max_chars`,
//! `swatch`. No model ever prices anything.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Records, Site, money, time};

/// A price list version.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PriceList {
    /// `v14`.
    pub version: String,
    /// Its currency.
    pub currency: String,
    /// `live` or `draft`.
    pub status: String,
    /// Local minor units per US dollar (KSh 129.00 is 12900).
    pub usd_rate_minor: i64,
    /// From when it applies.
    pub valid_from: i64,
    /// Until when quotes from it hold.
    pub valid_until: i64,
    /// Where it came from, such as "Wanjiru confirmed the price list".
    pub source: String,
}

/// One priced rule.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Rule {
    /// `size.2`.
    pub code: String,
    /// `size`, `sponge`, `colour`, `finish`, `deco`, `message`, `delivery`.
    pub grp: String,
    /// "2 tiers".
    pub label: String,
    /// The amount it adds.
    pub amount_minor: i64,
    /// Constraints and display hints, JSON text.
    #[serde(skip)]
    pub meta: String,
    /// Order within its group.
    pub sort: i64,
}

impl Rule {
    /// The constraints, parsed.
    pub fn meta(&self) -> Value {
        serde_json::from_str(&self.meta).unwrap_or(Value::Null)
    }
}

/// What a customer picked in the cake studio. Codes are rule codes; `None` is not chosen yet.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CakeChoice {
    /// `size.1`, `size.2`, `size.3`.
    pub size: Option<String>,
    /// `sponge.vanilla` …
    pub sponge: Option<String>,
    /// `colour.forest` …
    pub colour: Option<String>,
    /// `finish.smooth` …
    pub finish: Option<String>,
    /// `deco.berries`, or `deco.none`.
    pub deco: Option<String>,
    /// The message on the cake.
    #[serde(default)]
    pub message: String,
    /// The day wanted, `2026-11-14`.
    pub date: Option<String>,
    /// A note such as "No nuts".
    #[serde(default)]
    pub note: String,
    /// The idea of the cake the customer saw, `/media/ideas/<hex>.webp` (ADR-032), kept with
    /// the held request so the owner sees the same picture. Set only by the server after it
    /// checks the picture was drawn for exactly these options; never read from a request.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub idea: Option<String>,
}

/// A priced line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuoteLine {
    /// The rule code.
    pub code: String,
    /// What it is.
    pub label: String,
    /// What it adds, in minor units.
    pub amount_minor: i64,
}

/// A flag on a quote: a constraint broken or something to know.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Flag {
    /// `error` stops the request; `note` is for information.
    pub level: &'static str,
    /// The words.
    pub text: String,
}

/// A cake priced from the live list.
#[derive(Debug, Clone, Serialize)]
pub struct CakeQuote {
    /// Lines in order.
    pub lines: Vec<QuoteLine>,
    /// Their sum.
    pub total_minor: i64,
    /// The same in US cents at the posted rate.
    pub usd_cents: i64,
    /// The price list version.
    pub price_list: String,
    /// The posted rate.
    pub usd_rate_minor: i64,
    /// Until when this price holds.
    pub valid_until: i64,
    /// Flags.
    pub flags: Vec<Flag>,
    /// Every group chosen and no error flag.
    pub complete: bool,
    /// "Serves 25–30", when a size is chosen.
    pub serves: Option<String>,
    /// The earliest day the size can be baked, as `2026-11-10`.
    pub earliest_day: Option<String>,
}

/// The groups a cake needs, in the order the studio asks.
pub const CAKE_GROUPS: [&str; 5] = ["size", "sponge", "colour", "finish", "deco"];

impl Records {
    /// The live price list.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn live_price_list(&self, site: &str) -> anyhow::Result<Option<PriceList>> {
        Ok(sqlx::query_as::<_, PriceList>(
            "SELECT version, currency, status, usd_rate_minor, valid_from, valid_until, source
             FROM rec_price_lists WHERE site = ? AND status = 'live' ORDER BY valid_from DESC LIMIT 1",
        )
        .bind(site)
        .fetch_optional(self.pool())
        .await?)
    }

    /// Every price list, newest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn price_lists(&self, site: &str) -> anyhow::Result<Vec<PriceList>> {
        Ok(sqlx::query_as::<_, PriceList>(
            "SELECT version, currency, status, usd_rate_minor, valid_from, valid_until, source
             FROM rec_price_lists WHERE site = ? ORDER BY valid_from DESC",
        )
        .bind(site)
        .fetch_all(self.pool())
        .await?)
    }

    /// The rules of a price list version, by group and order.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn rules(&self, site: &str, version: &str) -> anyhow::Result<Vec<Rule>> {
        Ok(sqlx::query_as::<_, Rule>(
            "SELECT code, grp, label, amount_minor, meta, sort FROM rec_price_rules
             WHERE site = ? AND version = ? ORDER BY grp, sort",
        )
        .bind(site)
        .bind(version)
        .fetch_all(self.pool())
        .await?)
    }

    /// Prices a cake from the live list and checks its constraints.
    ///
    /// # Errors
    ///
    /// When the site has no live price list, or on a database error.
    pub async fn quote_cake(
        &self,
        site: &Site,
        choice: &CakeChoice,
        now: i64,
    ) -> anyhow::Result<CakeQuote> {
        let list = self
            .live_price_list(&site.site)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no live price list"))?;
        let rules = self.rules(&site.site, &list.version).await?;
        let by_code: BTreeMap<&str, &Rule> = rules.iter().map(|r| (r.code.as_str(), r)).collect();
        let mut lines = Vec::new();
        let mut flags = Vec::new();
        let mut complete = true;
        let picks = [
            &choice.size,
            &choice.sponge,
            &choice.colour,
            &choice.finish,
            &choice.deco,
        ];
        for (grp, pick) in CAKE_GROUPS.iter().zip(picks) {
            match pick.as_deref().and_then(|c| by_code.get(c)) {
                Some(rule) if rule.grp == *grp => lines.push(QuoteLine {
                    code: rule.code.clone(),
                    label: rule.label.clone(),
                    amount_minor: rule.amount_minor,
                }),
                Some(_) | None => complete = false,
            }
        }
        let size = choice.size.as_deref().and_then(|c| by_code.get(c)).copied();
        // Constraints: options limited to some sizes.
        for pick in [&choice.sponge, &choice.finish, &choice.deco]
            .into_iter()
            .flatten()
        {
            if let Some(rule) = by_code.get(pick.as_str()) {
                let meta = rule.meta();
                if let (Some(only), Some(size)) =
                    (meta.get("only_sizes").and_then(Value::as_array), size)
                    && !only.iter().any(|s| s.as_str() == Some(size.code.as_str()))
                {
                    flags.push(Flag {
                        level: "error",
                        text: format!(
                            "{} is only for {}. Pick another size or sponge.",
                            rule.label,
                            labels(&by_code, only)
                        ),
                    });
                }
                if let Some(note) = meta.get("note").and_then(Value::as_str) {
                    flags.push(Flag {
                        level: "note",
                        text: note.to_owned(),
                    });
                }
            }
        }
        // The message.
        let max = by_code
            .get("message")
            .and_then(|r| r.meta().get("max_chars").and_then(Value::as_u64))
            .unwrap_or(22) as usize;
        if choice.message.chars().count() > max {
            flags.push(Flag {
                level: "error",
                text: format!("The message fits {max} characters."),
            });
        }
        if !choice.message.trim().is_empty() {
            lines.push(QuoteLine {
                code: "message".into(),
                label: format!("Message: \u{201c}{}\u{201d}", choice.message.trim()),
                amount_minor: 0,
            });
        }
        // Notice: the earliest day this size can be baked.
        let notice = size
            .and_then(|s| s.meta().get("notice_days").and_then(Value::as_i64))
            .unwrap_or(2);
        let today = time::local(now, site.utc_offset_min).date_naive();
        let earliest = today + chrono::Duration::days(notice);
        let earliest_day = size.map(|_| earliest.format("%Y-%m-%d").to_string());
        match (&choice.date, size) {
            (Some(d), Some(_)) => {
                if let Ok(wanted) = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d") {
                    if wanted < earliest {
                        flags.push(Flag {
                            level: "error",
                            text: format!(
                                "This size needs {notice} days' notice. The earliest is {}.",
                                earliest.format("%a %-d %b")
                            ),
                        });
                    }
                } else {
                    flags.push(Flag {
                        level: "error",
                        text: "Pick a day from the calendar.".into(),
                    });
                }
            }
            (None, _) => complete = false,
            _ => {}
        }
        let total_minor: i64 = lines.iter().map(|l| l.amount_minor).sum();
        let complete = complete && !flags.iter().any(|f| f.level == "error");
        Ok(CakeQuote {
            usd_cents: money::usd_cents(total_minor, list.usd_rate_minor),
            serves: size.and_then(|s| {
                s.meta()
                    .get("serves")
                    .and_then(Value::as_str)
                    .map(|v| format!("Serves {v}"))
            }),
            lines,
            total_minor,
            price_list: list.version,
            usd_rate_minor: list.usd_rate_minor,
            valid_until: list.valid_until,
            flags,
            complete,
            earliest_day,
        })
    }
}

fn labels(by_code: &BTreeMap<&str, &Rule>, codes: &[Value]) -> String {
    let names: Vec<String> = codes
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|c| by_code.get(c).map(|r| r.label.to_lowercase()))
        .collect();
    match names.as_slice() {
        [] => "some sizes".into(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}
