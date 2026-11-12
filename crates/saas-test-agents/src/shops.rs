//! What each sample site is for, so a test agent's visit fits it: the words it searches with
//! (each from that site's own products or stories), the questions a customer there would ask,
//! what one visit may spend, and how often each agent goes there. The shops that can sell get
//! most of the visits.
//!
//! - Kilele Bakery sells bakes: breads, pastries, the weekend bake box, the fruit cake, and
//!   custom cakes held for the owner.
//! - Classy Perfumes sells fragrances and gift sets.
//! - Tana Valley Times is a newspaper: its stories, a Supporter subscription on its own page,
//!   the Friday digest, and questions about advertising. Its door sells nothing.

use rand::Rng;
use rand::seq::SliceRandom;

use crate::brain::Action;

/// US dollars to Kenya shillings, as the sample sites' price lists set it.
pub const KES_PER_USD: f64 = 129.0;

/// What kind of site it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A bakery: bakes for collection, custom cakes.
    Bakery,
    /// A perfume shop: fragrances and gift sets, delivered.
    Perfumery,
    /// A newspaper: stories and a subscription.
    News,
}

/// One sample site, as a test agent sees it.
#[derive(Debug, Clone, Copy)]
pub struct Shop {
    /// The site's host label under the platform host.
    pub slug: &'static str,
    /// Its name.
    pub name: &'static str,
    /// What kind of site it is.
    pub kind: Kind,
    /// What the site is, for the model.
    pub about: &'static str,
    /// What a visit there is for, for the model.
    pub intents: &'static str,
    /// Search words, each from the site's own products (shops) or stories (news).
    pub searches: &'static [&'static str],
    /// Questions a customer there would ask.
    pub questions: &'static [&'static str],
    /// What one visit may spend, in US dollars (low, high), inside USD 5–40. Set so the
    /// cheapest thing the shop sells is within reach.
    pub budget_usd: (f64, f64),
    /// How a random pick weighs each action here.
    pub weights: &'static [(Action, u32)],
}

/// The three sample sites. Their order is the order of [`VISITS`]' weights.
pub static SHOPS: [Shop; 3] = [
    Shop {
        slug: "kilele-bakery-sample",
        name: "Kilele Bakery",
        kind: Kind::Bakery,
        about: "Kilele Bakery, a Nairobi bakery: breads and pastries baked through the day, a weekend bake box, \
                a seasonal Christmas fruit cake, and custom celebration cakes made to order. It sells bakes only.",
        intents: "Buy bakes for collection at the bakery (a quote or a checkout for breads, pastries or the bake box), \
                  ask about today's bakes, batches or collection, or now and then ask for a custom cake.",
        searches: &[
            "sourdough",
            "rye",
            "croissant",
            "cardamom bun",
            "mandazi",
            "banana bread",
            "cinnamon roll",
            "red velvet",
            "bake box",
            "fruit cake",
            "bread",
            "",
        ],
        questions: &[
            "What's fresh this morning, and when is the next croissant batch?",
            "Is the country sourdough still available for collection today?",
            "What's in the weekend bake box?",
            "Can you make a gluten-free birthday cake, and how far ahead should I order?",
            "Does the Christmas fruit cake contain nuts?",
            "What time can I collect on Saturday?",
        ],
        budget_usd: (5.0, 40.0),
        weights: &[
            (Action::Checkout, 35),
            (Action::Quote, 25),
            (Action::Ask, 15),
            (Action::Custom, 12),
            (Action::Browse, 8),
        ],
    },
    Shop {
        slug: "classy-perfumes-sample",
        name: "Classy Perfumes",
        kind: Kind::Perfumery,
        about: "Classy Perfumes, a Nairobi perfume shop: fragrances in fresh citrus, floral, woody, amber and gourmand \
                families, and gift sets, delivered in Nairobi and to other towns. It sells fragrances and gift sets only.",
        intents: "Buy a fragrance or a gift set for delivery (a quote or a checkout), or ask about gifts, wrapping, \
                  samples or delivery.",
        searches: &[
            "citrus",
            "vanilla gift set",
            "gift set",
            "amber",
            "floral",
            "honey baobab",
            "lime",
            "grapefruit",
            "karura moss",
            "frangipani",
            "travel duo",
            "discovery set",
            "",
        ],
        questions: &[
            "Which fragrance would you suggest as a gift under KSh 5,000?",
            "Do you gift-wrap, and what does it cost?",
            "Can I add a free 2 ml sample to my order?",
            "Do you deliver the same day in Nairobi?",
            "What's in the Floral Discovery Set?",
            "Which of your citrus scents lasts longest?",
        ],
        // The cheapest bottle is KSh 4,000 (USD 31.01).
        budget_usd: (32.0, 40.0),
        weights: &[
            (Action::Checkout, 40),
            (Action::Quote, 25),
            (Action::Ask, 20),
            (Action::Browse, 15),
        ],
    },
    Shop {
        slug: "tana-valley-times-sample",
        name: "Tana Valley Times",
        kind: Kind::News,
        about: "Tana Valley Times, a weekly local newspaper: stories on transport, the county council, farming and \
                weather, business and the community, a Supporter subscription (KSh 300 a month, paid on its own \
                page) and a free Friday email digest. It sells no products.",
        intents: "Read and subscribe: subscribe for a Supporter month (paid with your saved sandbox buyer), or ask \
                  about a story you found, subscribing, the Friday digest, or placing an ad.",
        searches: &[
            "rail",
            "budget",
            "Ndovu market",
            "matatu",
            "rains",
            "bridge",
            "library",
            "maize",
            "water tariff",
            "bursary",
            "clinic",
            "street lights",
        ],
        questions: &[
            "How much is a subscription, and what does it include?",
            "How do I get the Friday five digest by email?",
            "Can a local business place an ad in the paper, and what does it cost?",
            "Where does the northern rail extension stand?",
            "What did the council decide on the water tariff?",
            "Who has been reporting the Ndovu market move?",
        ],
        budget_usd: (5.0, 40.0),
        weights: &[
            (Action::Subscribe, 30),
            (Action::Ask, 50),
            (Action::Browse, 20),
        ],
    },
];

/// How often each agent visits each of [`SHOPS`] (Kilele, Classy, Tana): mostly the shops that
/// sell, each agent leaning to what it shops for.
pub const VISITS: [(&str, [u32; 3]); 3] = [
    ("amani", [60, 25, 15]),
    ("baraka", [35, 55, 10]),
    ("chiku", [45, 25, 30]),
];

/// For an agent [`VISITS`] doesn't name.
const DEFAULT_VISITS: [u32; 3] = [45, 40, 15];

/// The shop with this slug.
pub fn by_slug(slug: &str) -> Option<&'static Shop> {
    SHOPS.iter().find(|s| s.slug == slug)
}

/// The shop at this origin (`https://<slug>.<platform host>`).
pub fn by_origin(origin: &str) -> Option<&'static Shop> {
    let host = origin
        .split_once("://")
        .map_or(origin, |(_, rest)| rest)
        .split(['.', ':', '/'])
        .next()
        .unwrap_or_default();
    by_slug(host)
}

/// `agent`'s weights over [`SHOPS`].
pub fn visits(agent: &str) -> [u32; 3] {
    VISITS
        .iter()
        .find(|(a, _)| *a == agent)
        .map_or(DEFAULT_VISITS, |(_, w)| *w)
}

/// Where `agent` goes on this visit.
pub fn pick(agent: &str, rng: &mut impl Rng) -> &'static Shop {
    let weights = visits(agent);
    let total: u32 = weights.iter().sum();
    let mut roll = rng.gen_range(0..total.max(1));
    for (shop, w) in SHOPS.iter().zip(weights) {
        if roll < w {
            return shop;
        }
        roll -= w;
    }
    &SHOPS[0]
}

impl Shop {
    /// Whether the site sells through its door (quotes and checkouts).
    pub fn sells_at_the_door(&self) -> bool {
        self.kind != Kind::News
    }

    /// A search for this visit.
    pub fn search(&self, rng: &mut impl Rng) -> &'static str {
        self.searches.choose(rng).copied().unwrap_or_default()
    }

    /// What this visit may spend, in US dollars.
    pub fn budget(&self, rng: &mut impl Rng) -> f64 {
        let (lo, hi) = self.budget_usd;
        if hi > lo { rng.gen_range(lo..hi) } else { lo }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;

    use super::*;
    use crate::config::{AGENTS, SITES};

    fn records(slug: &str, file: &str) -> Vec<Value> {
        let dir = slug.trim_end_matches("-sample");
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/samples")
            .join(dir)
            .join("records")
            .join(file);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// Each record's searchable words, lowercased.
    fn texts(shop: &Shop) -> Vec<String> {
        let (file, fields): (&str, &[&str]) = match shop.kind {
            Kind::News => ("stories.jsonl", &["title", "section", "topic", "standfirst"]),
            _ => (
                "products.jsonl",
                &["name", "section", "sku", "family", "description"],
            ),
        };
        records(shop.slug, file)
            .iter()
            .map(|r| {
                fields
                    .iter()
                    .filter_map(|f| r[*f].as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase()
            })
            .collect()
    }

    #[test]
    fn the_shops_are_the_sample_sites() {
        let slugs: Vec<&str> = SHOPS.iter().map(|s| s.slug).collect();
        assert_eq!(slugs, SITES);
        for (agent, ..) in AGENTS {
            assert!(VISITS.iter().any(|(a, _)| *a == agent), "{agent}");
        }
    }

    #[test]
    fn every_search_comes_from_that_sites_own_catalogue() {
        for shop in &SHOPS {
            if shop.kind == Kind::News {
                // stories.search needs words.
                assert!(shop.searches.iter().all(|t| !t.is_empty()));
            }
            let texts = texts(shop);
            for term in shop.searches {
                let words: Vec<String> =
                    term.split_whitespace().map(str::to_lowercase).collect();
                assert!(
                    texts.iter().any(|t| words.iter().all(|w| t.contains(w))),
                    "{}: \"{term}\" matches nothing it publishes",
                    shop.name
                );
            }
        }
        // The searches from the digest that found nothing: perfume at the bakery, cakes at the
        // newspaper.
        let bakery = by_slug("kilele-bakery-sample").unwrap();
        assert!(!bakery.searches.iter().any(|t| t.contains("perfume") || t.contains("oud")));
        let news = by_slug("tana-valley-times-sample").unwrap();
        assert!(!news.searches.iter().any(|t| t.contains("cake") || t.contains("perfume")));
    }

    #[test]
    fn each_shops_budget_reaches_what_it_sells_within_usd_5_to_40() {
        for shop in SHOPS.iter().filter(|s| s.sells_at_the_door()) {
            let (lo, hi) = shop.budget_usd;
            assert!((5.0..=40.0).contains(&lo) && (5.0..=40.0).contains(&hi) && lo <= hi);
            let cheapest = records(shop.slug, "products.jsonl")
                .iter()
                .filter_map(|p| p["price"].as_f64())
                .filter(|p| *p > 0.0)
                .fold(f64::MAX, f64::min);
            assert!(
                cheapest / KES_PER_USD <= lo,
                "{}: the cheapest item (KSh {cheapest}) is out of reach of USD {lo}",
                shop.name
            );
        }
    }

    #[test]
    fn most_visits_go_to_the_shops_that_sell() {
        let mut selling = 0;
        let mut all = 0;
        for (agent, weights) in VISITS {
            let sum: u32 = weights.iter().sum();
            let sells: u32 = SHOPS
                .iter()
                .zip(weights)
                .filter(|(s, _)| s.sells_at_the_door())
                .map(|(_, w)| w)
                .sum();
            assert!(sells * 10 >= sum * 7, "{agent}: {sells} of {sum}");
            selling += sells;
            all += sum;
        }
        assert!(selling * 5 >= all * 4, "{selling} of {all}");
        let mut rng = rand::thread_rng();
        let mut news = 0;
        for _ in 0..3000 {
            if pick("baraka", &mut rng).kind == Kind::News {
                news += 1;
            }
        }
        assert!((150..=450).contains(&news), "{news} of 3000");
    }

    #[test]
    fn a_shop_is_found_by_its_origin() {
        assert_eq!(
            by_origin("https://classy-perfumes-sample.paypal.agentrix.website").map(|s| s.kind),
            Some(Kind::Perfumery)
        );
        assert_eq!(
            by_origin("http://tana-valley-times-sample.localhost:18182").map(|s| s.kind),
            Some(Kind::News)
        );
        assert!(by_origin("https://elsewhere.example").is_none());
    }
}
