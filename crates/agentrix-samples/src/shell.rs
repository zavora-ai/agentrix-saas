//! The shell every sample page shares (01a §2.2): header with desks (S-1), the team sheet on
//! phones (S-2), the Ask bar (S-3), the rail (S-4), the footer (S-5), living marks (S-6) and the
//! Sample band (S-7). Desk facts are live: computed from records at render time.

use agentrix_records::{Records, Site, time};
use serde::Serialize;
use serde_json::Value;

use crate::App;
use crate::web::Ctx;

/// A live fact in a desk's peek.
#[derive(Debug, Clone, Serialize)]
pub struct Fact {
    /// "Sourdough".
    pub k: String,
    /// "6 left".
    pub v: String,
}

/// Facts for one desk, with the time they were read.
#[derive(Debug, Clone, Serialize, Default)]
pub struct DeskFacts {
    /// The facts.
    pub facts: Vec<Fact>,
    /// "as of 09:42", or empty.
    pub asof: String,
}

/// A suggested question.
#[derive(Debug, Clone, Serialize)]
pub struct Starter {
    /// The words on the button (also what is asked).
    pub text: String,
    /// The intent it maps to.
    pub intent: String,
    /// Answered without a model.
    pub instant: bool,
}

/// A desk as the shell draws it.
#[derive(Debug, Clone, Serialize)]
pub struct Desk {
    /// `bread`.
    pub id: String,
    /// "Bread".
    pub name: String,
    /// Its static page.
    pub href: String,
    /// Mark colour.
    pub color: String,
    /// Mark glyph name.
    pub glyph: String,
    /// Mark seed.
    pub seed: i64,
    /// What it does, in visitor words.
    pub mandate: String,
    /// Live facts.
    pub facts: Vec<Fact>,
    /// When the facts were read.
    pub asof: String,
    /// Starters.
    pub starters: Vec<Starter>,
}

/// The site, as the shell needs it.
#[derive(Debug, Clone, Serialize)]
pub struct SiteView {
    /// Slug.
    pub slug: String,
    /// Name.
    pub name: String,
    /// `bakery`, `shop`, `news`.
    pub archetype: String,
    /// Host as served.
    pub host: String,
    /// The domain the business would use (`kilele-bakery.example`).
    pub domain: String,
    /// The owner's first name ("Wanjiru"), or the shop's name for shops without one.
    pub owner: String,
    /// The owner's initial.
    pub owner_initial: String,
    /// "WhatsApp".
    pub owner_channel: String,
    /// "~2 h".
    pub owner_wait: String,
    /// "v14".
    pub release: String,
    /// The key marks are seeded with (`kilele`, as in the concepts).
    pub mark_key: String,
    /// The mark family.
    pub family: String,
    /// The mark squareness.
    pub squareness: f64,
    /// The display font for monogram marks.
    pub font: String,
    /// The current year, for the footer.
    pub year: String,
}

/// The signed-in customer, as the header shows them.
#[derive(Debug, Clone, Serialize)]
pub struct Account {
    /// Name.
    pub name: String,
    /// Initial for the chip.
    pub initial: String,
}

/// The shell.
#[derive(Debug, Clone, Serialize)]
pub struct Shell {
    /// The site.
    pub site: SiteView,
    /// Its desks.
    pub desks: Vec<Desk>,
    /// The desk now speaking in the rail by default.
    pub front: Desk,
    /// Which desk's page this is (`bread`), or empty.
    pub current: String,
    /// The path.
    pub path: String,
    /// The signed-in customer.
    pub account: Option<Account>,
    /// Where sign-in is.
    pub sign_in: String,
    /// Items in the bag, or -1 where the page can't know (W6's account pages render without the
    /// request, so they show the bag without a count rather than a wrong one).
    pub bag: i64,
    /// Asset version.
    pub v: &'static str,
    /// The UI kit's asset version (W7 serves its files as immutable).
    pub ui_v: &'static str,
    /// The page title.
    pub title: String,
    /// The page description.
    pub description: String,
    /// Whether the site is a sample.
    pub sample: bool,
    /// Where the owner's apps are (for the Sample band).
    pub owner_url: String,
    /// The site's layout template: shared pages (bag, orders, account) extend it.
    pub layout: String,
    /// Approved pictures by slot (`hero`, `product:<sku>`); each carries its "Sample image" label.
    pub pictures: std::collections::HashMap<String, agentrix_media::library::Shown>,
    /// The desks' model is answering (not resting after a failure): labels that promise AI
    /// answers show only then.
    pub ai: bool,
}

fn starter(v: &Value) -> Option<Starter> {
    let a = v.as_array()?;
    Some(Starter {
        text: a.first()?.as_str()?.to_owned(),
        intent: a.get(1).and_then(Value::as_str).unwrap_or("").to_owned(),
        // Only the starters a capability answers without a model are "Instant"; Ask decides
        // that, not the manifest's flag (the rest go to the desks' model).
        instant: a
            .get(1)
            .and_then(Value::as_str)
            .is_some_and(|i| crate::ask::INSTANT_INTENTS.contains(&i)),
    })
}

/// The desks in a site's manifest, with live facts from `facts`.
pub fn desks(site: &Site, facts: &std::collections::HashMap<String, DeskFacts>) -> Vec<Desk> {
    site.facts
        .get("desks")
        .and_then(Value::as_array)
        .map(|ds| {
            ds.iter()
                .filter_map(|d| {
                    let id = d.get("id")?.as_str()?.to_owned();
                    let live = facts.get(&id).cloned().unwrap_or_default();
                    Some(Desk {
                        name: d.get("name")?.as_str()?.to_owned(),
                        href: d
                            .get("href")
                            .and_then(Value::as_str)
                            .unwrap_or("/")
                            .to_owned(),
                        color: d
                            .get("color")
                            .and_then(Value::as_str)
                            .unwrap_or("#38356d")
                            .to_owned(),
                        glyph: d
                            .get("glyph")
                            .and_then(Value::as_str)
                            .unwrap_or("chat")
                            .to_owned(),
                        seed: d.get("seed").and_then(Value::as_i64).unwrap_or(1),
                        mandate: d
                            .get("mandate")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        starters: d
                            .get("starters")
                            .and_then(Value::as_array)
                            .map(|s| s.iter().filter_map(starter).collect())
                            .unwrap_or_default(),
                        facts: live.facts,
                        asof: live.asof,
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The Front desk behind the Ask bar.
pub fn front(site: &Site) -> Desk {
    Desk {
        id: "front".into(),
        name: "Front desk".into(),
        href: "/".into(),
        color: site
            .fact("/front_color")
            .chars()
            .next()
            .map_or_else(|| "#3b2416".to_owned(), |_| site.fact("/front_color")),
        glyph: "search".into(),
        seed: 1,
        mandate: "Routes your question to the right desk.".into(),
        facts: Vec::new(),
        asof: String::new(),
        starters: Vec::new(),
    }
}

/// "as of 09:42".
pub fn asof(site: &Site, now: i64) -> String {
    format!("as of {}", time::fmt_clock(now, site.utc_offset_min))
}

impl App {
    /// The shell for a page.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn shell(
        &self,
        ctx: &Ctx,
        current: &str,
        title: &str,
        description: &str,
    ) -> anyhow::Result<Shell> {
        let records: &Records = self.records().await?;
        let site = &ctx.site;
        let now = time::now();
        let facts = match site.archetype.as_str() {
            "bakery" => crate::bakery::desk_facts(records, site, now).await?,
            "shop" => crate::classy::desk_facts(records, site, now).await?,
            "news" => crate::news::desk_facts(records, site, now).await?,
            _ => Default::default(),
        };
        let bag = match &ctx.cart {
            Some(id) => records
                .cart(&site.site, id)
                .await?
                .lines
                .iter()
                .map(|l| l.qty)
                .sum(),
            None => 0,
        };
        let owner_first = site.fact("/owner/first");
        Ok(Shell {
            site: SiteView {
                slug: site.site.clone(),
                name: site.name.clone(),
                archetype: site.archetype.clone(),
                host: ctx.host.host.clone(),
                domain: site.fact("/domain"),
                owner_initial: owner_first
                    .chars()
                    .next()
                    .map(String::from)
                    .unwrap_or_default(),
                owner: owner_first,
                owner_channel: site.fact("/owner/channel"),
                owner_wait: site.fact("/owner/wait"),
                release: site.fact("/release"),
                mark_key: site.fact("/mark/key"),
                family: site.fact("/mark/family"),
                squareness: site
                    .facts
                    .pointer("/mark/squareness")
                    .and_then(Value::as_f64)
                    .unwrap_or(4.6),
                font: site.fact("/mark/font"),
                year: time::local(now, site.utc_offset_min)
                    .format("%Y")
                    .to_string(),
            },
            desks: desks(site, &facts),
            front: front(site),
            current: current.to_owned(),
            path: ctx.path.clone(),
            account: ctx.visitor.as_ref().map(|v| Account {
                initial: v
                    .name
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default(),
                name: v.name.clone(),
            }),
            sign_in: format!("/account/sign-in?next={}", urlencode(&ctx.path)),
            bag,
            v: agentrix_themes::VERSION,
            ui_v: agentrix_ui::VERSION,
            title: title.to_owned(),
            description: description.to_owned(),
            sample: site.is_sample(),
            owner_url: format!("{}/samples/{}/", self.config.platform_origin(), site.site),
            layout: match site.archetype.as_str() {
                "shop" => "classy/layout.html",
                "news" => "tana/layout.html",
                _ => "kilele/layout.html",
            }
            .into(),
            pictures: self.pictures(&site.site).await,
            ai: !crate::ask::resting(self),
        })
    }
}

/// Percent-encodes a path for a query value.
pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl App {
    /// The shell around W6's account pages, built from the cached site without live facts.
    pub fn account_shell(
        &self,
        slug: &str,
        host: &str,
        title: &str,
        signed_in: Option<&str>,
    ) -> Option<Shell> {
        let site = self.sites.read().ok()?.get(slug)?.clone();
        let owner_first = site.fact("/owner/first");
        Some(Shell {
            site: SiteView {
                slug: site.site.clone(),
                name: site.name.clone(),
                archetype: site.archetype.clone(),
                host: host.to_owned(),
                domain: site.fact("/domain"),
                owner_initial: owner_first
                    .chars()
                    .next()
                    .map(String::from)
                    .unwrap_or_default(),
                owner: owner_first,
                owner_channel: site.fact("/owner/channel"),
                owner_wait: site.fact("/owner/wait"),
                release: site.fact("/release"),
                mark_key: site.fact("/mark/key"),
                family: site.fact("/mark/family"),
                squareness: site
                    .facts
                    .pointer("/mark/squareness")
                    .and_then(Value::as_f64)
                    .unwrap_or(4.6),
                font: site.fact("/mark/font"),
                year: time::local(time::now(), site.utc_offset_min)
                    .format("%Y")
                    .to_string(),
            },
            desks: desks(&site, &Default::default()),
            front: front(&site),
            current: String::new(),
            path: "/account".into(),
            account: signed_in.map(|n| Account {
                initial: n
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default(),
                name: n.to_owned(),
            }),
            sign_in: "/account/sign-in".into(),
            bag: -1,
            v: agentrix_themes::VERSION,
            ui_v: agentrix_ui::VERSION,
            title: title.to_owned(),
            description: String::new(),
            sample: site.is_sample(),
            owner_url: format!("{}/samples/{}/", self.config.platform_origin(), site.site),
            layout: match site.archetype.as_str() {
                "shop" => "classy/layout.html",
                "news" => "tana/layout.html",
                _ => "kilele/layout.html",
            }
            .into(),
            pictures: Default::default(),
            ai: !crate::ask::resting(self),
        })
    }
}
