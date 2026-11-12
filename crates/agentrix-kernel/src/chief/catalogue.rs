//! The Chief's catalogue tool: one compact line per product (its name, section and family, then
//! each size with its price and stock), filters so the model asks for exactly what it needs, and
//! a `more` cursor that pages a long list instead of cutting it. Nothing here reads a database:
//! the Chief hands it the site's rows.

use std::collections::HashMap;

use agentrix_records::catalogue::{Product, Stock};
use agentrix_records::pricing::{PriceList, Rule};
use agentrix_records::{money, time};
use serde_json::{Map, Value, json};

use super::clip;

/// Characters of product lines on one page. With the price list and the JSON around them, a page
/// stays well under what the model reads of one tool's result ([`super::TOOL_CHARS`]).
pub(super) const PAGE_CHARS: usize = 3500;

/// The longest one product's line may be, details included.
const LINE_CHARS: usize = 700;

/// The tool's arguments, as the model sees them.
pub(super) fn args() -> Value {
    json!({
        "type": "object",
        "properties": {
            "size": { "type": "string", "description": "Only this size, such as \"50 ml\"." },
            "family_or_category": { "type": "string", "description": "Only this scent family or section, such as \"Woody\", \"gift\" or \"bread\"." },
            "in_stock_only": { "type": "boolean", "description": "Only what can be bought now, leaving out what is sold out." },
            "name_contains": { "type": "string", "description": "Only products whose name contains this, such as \"amber\"." },
            "details": { "type": "boolean", "description": "Add each product's description and attributes: scent notes, ingredients, allergens. Use it with a filter." },
            "more": { "type": "string", "description": "The `more` value from the page before, to read the next page. Keep the same filters." }
        }
    })
}

/// What the model asked for.
#[derive(Debug, Default, Clone)]
pub(super) struct Filter {
    size: Option<String>,
    family: Option<String>,
    name: Option<String>,
    in_stock: bool,
    details: bool,
    from: usize,
}

impl Filter {
    /// The filter in a tool call's arguments. Anything missing or malformed asks for everything.
    pub(super) fn from_args(args: &Value) -> Self {
        let text = |key: &str| {
            args[key]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.chars().take(80).collect::<String>())
        };
        let from = match &args["more"] {
            Value::String(s) => s.trim().parse().unwrap_or(0),
            Value::Number(n) => n
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0),
            _ => 0,
        };
        Self {
            size: text("size").map(|s| squash(&s)),
            family: text("family_or_category").map(|s| s.to_lowercase()),
            name: text("name_contains").map(|s| s.to_lowercase()),
            in_stock: args["in_stock_only"].as_bool().unwrap_or(false),
            details: args["details"].as_bool().unwrap_or(false),
            from,
        }
    }

    /// Whether only some products were asked for (by size, family or stock).
    fn narrows_products(&self) -> bool {
        self.size.is_some() || self.family.is_some() || self.in_stock
    }
}

/// The site's rows the catalogue reads.
pub(super) struct Rows<'a> {
    pub currency: &'a str,
    pub utc_offset_min: i32,
    pub products: &'a [Product],
    pub stock: &'a HashMap<String, Stock>,
    pub price_list: Option<(&'a PriceList, &'a [Rule])>,
}

/// One page of the catalogue for this filter.
pub(super) fn page(rows: &Rows<'_>, f: &Filter) -> Value {
    let lines = lines(rows, f);
    let matching = lines.len();
    let mut out = Map::new();
    out.insert("currency".into(), json!(rows.currency));
    out.insert("matching".into(), json!(matching));
    if matching == 0 {
        out.insert(
            "note".into(),
            json!("No product matches those filters. These are the sizes, families and categories there are."),
        );
        out.insert(
            "sizes".into(),
            json!(distinct(rows.products, |p| p.size.as_deref())),
        );
        out.insert(
            "families".into(),
            json!(distinct(rows.products, |p| p.family.as_deref())),
        );
        out.insert(
            "categories".into(),
            json!(distinct(rows.products, |p| Some(p.section.as_str()))),
        );
    } else if f.from >= matching {
        out.insert(
            "note".into(),
            json!(format!(
                "That was the last page: all {matching} were shown."
            )),
        );
    } else {
        let end = fit(&lines, f.from, PAGE_CHARS);
        out.insert(
            "showing".into(),
            json!(format!("{} to {end} of {matching}", f.from + 1)),
        );
        out.insert("products".into(), json!(lines[f.from..end]));
        if end < matching {
            out.insert("more".into(), json!(end.to_string()));
            out.insert(
                "note".into(),
                json!(format!(
                    "{} more. Call catalogue again with the same filters and more: \"{end}\".",
                    matching - end
                )),
            );
        }
    }
    if f.from == 0
        && let Some(list) = price_list(rows, f)
    {
        out.insert("price_list".into(), list);
    }
    Value::Object(out)
}

/// One line per product that passes the filter, in list order. A product sold in several sizes
/// is one line with every size that passes.
fn lines(rows: &Rows<'_>, f: &Filter) -> Vec<String> {
    let mut groups: Vec<(&str, &str, Vec<&Product>)> = Vec::new();
    for p in rows.products {
        match groups
            .iter_mut()
            .find(|(section, name, _)| *section == p.section && *name == p.name)
        {
            Some(group) => group.2.push(p),
            None => groups.push((&p.section, &p.name, vec![p])),
        }
    }
    groups
        .into_iter()
        .filter_map(|(section, name, variants)| {
            let first = variants[0];
            if let Some(want) = &f.name
                && !name.to_lowercase().contains(want.as_str())
            {
                return None;
            }
            if let Some(want) = &f.family
                && !family_matches(first.family.as_deref(), section, want)
            {
                return None;
            }
            let kept: Vec<String> = variants
                .iter()
                .filter(|p| {
                    f.size
                        .as_deref()
                        .is_none_or(|want| size_matches(p.size.as_deref().unwrap_or(""), want))
                })
                .filter_map(|p| {
                    let st = rows.stock.get(&p.sku);
                    (!f.in_stock || available(st)).then(|| variant(rows, p, st))
                })
                .collect();
            if kept.is_empty() {
                return None;
            }
            let mut line = format!("{name} · {section}");
            if let Some(family) = first.family.as_deref().filter(|x| !x.trim().is_empty()) {
                line.push_str(" · ");
                line.push_str(family);
            }
            line.push_str(" · ");
            line.push_str(&kept.join("; "));
            if f.details
                && let Some(d) = details(first)
            {
                line.push_str(" · ");
                line.push_str(&d);
            }
            Some(clip(&line, LINE_CHARS))
        })
        .collect()
}

/// One size of a product: `50 ml KSh 4,900 (low, 3 left)`.
fn variant(rows: &Rows<'_>, p: &Product, st: Option<&Stock>) -> String {
    let mut v = String::new();
    if let Some(size) = p.size.as_deref().filter(|s| !s.trim().is_empty()) {
        v.push_str(size);
        v.push(' ');
    }
    v.push_str(&price(rows.currency, p.price_minor, "free"));
    if let Some(words) = stock_words(st, rows.utc_offset_min) {
        v.push_str(&format!(" ({words})"));
    }
    if p.status != "published" {
        v.push_str(" (a draft, not on the site)");
    }
    v
}

/// Stock in a few words, or nothing when the site keeps no count for it.
fn stock_words(st: Option<&Stock>, offset: i32) -> Option<String> {
    let st = st?;
    Some(match st.state {
        "in" => match st.left {
            Some(n) => format!("in stock, {n} left"),
            None => "in stock".to_string(),
        },
        "low" => format!("low, {} left", st.left.unwrap_or(0)),
        "batch" => {
            let next = st.next_batch_at.map_or_else(
                || "next batch today".to_string(),
                |t| format!("next batch {}", time::fmt_clock(t, offset)),
            );
            match st.left {
                Some(n) if n > 0 => format!("{n} left, {next}"),
                _ => format!("none left, {next}"),
            }
        }
        "out" => match st.back_at {
            Some(t) => format!("sold out, back {}", time::fmt_day_clock(t, offset)),
            None => "sold out".to_string(),
        },
        _ => return None,
    })
}

/// Whether it can be bought now. A product with no stock count kept always can.
fn available(st: Option<&Stock>) -> bool {
    st.is_none_or(|s| match s.state {
        "out" => false,
        "batch" => s.left.unwrap_or(0) > 0,
        _ => true,
    })
}

/// The description and attributes, for `details`. Display hints (tone, image) are left out.
fn details(p: &Product) -> Option<String> {
    let mut parts = Vec::new();
    let description = p.description.trim();
    if !description.is_empty() {
        parts.push(clip(description, 240));
    }
    if let Ok(Value::Object(attrs)) = serde_json::from_str::<Value>(&p.attrs) {
        for (key, value) in &attrs {
            if matches!(key.as_str(), "tone" | "image" | "family") {
                continue;
            }
            let key = key.replace('_', " ");
            parts.push(match value {
                Value::String(s) if !s.trim().is_empty() => format!("{key}: {}", s.trim()),
                Value::Array(items) if !items.is_empty() => {
                    let items: Vec<String> = items
                        .iter()
                        .map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_string))
                        .collect();
                    format!("{key}: {}", items.join(", "))
                }
                Value::Bool(true) => key,
                _ => continue,
            });
        }
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// The live price list for custom orders, one line per group of choices. It comes with the first
/// page, unless the model asked for products by size, family or stock; a name filter keeps only
/// the choices it names.
fn price_list(rows: &Rows<'_>, f: &Filter) -> Option<Value> {
    let (list, rules) = rows.price_list?;
    if f.narrows_products() {
        return None;
    }
    let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
    for r in rules.iter().take(80) {
        if let Some(want) = &f.name
            && !r.label.to_lowercase().contains(want.as_str())
            && !r.grp.to_lowercase().contains(want.as_str())
        {
            continue;
        }
        let choice = format!(
            "{} {}",
            r.label,
            price(&list.currency, r.amount_minor, "included")
        );
        match groups.iter_mut().find(|(g, _)| *g == r.grp) {
            Some(group) => group.1.push(choice),
            None => groups.push((&r.grp, vec![choice])),
        }
    }
    if groups.is_empty() {
        return None;
    }
    let choices: Vec<String> = groups
        .iter()
        .map(|(g, c)| clip(&format!("{g}: {}", c.join(", ")), 600))
        .collect();
    Some(json!({
        "version": list.version,
        "source": list.source,
        "valid_until": time::fmt_day(list.valid_until, rows.utc_offset_min),
        "note": "For custom orders: each choice adds its price.",
        "choices": choices,
    }))
}

/// The end of the page that starts at `from`: as many lines as fit in `budget` characters, and
/// always at least one.
fn fit(lines: &[String], from: usize, budget: usize) -> usize {
    let mut used = 0;
    let mut end = from;
    while end < lines.len() {
        // The line, its quotes and comma, and a little room for escapes.
        let n = lines[end].chars().count() + 4;
        if end > from && used + n > budget {
            break;
        }
        used += n;
        end += 1;
    }
    end
}

fn price(currency: &str, minor: i64, zero: &str) -> String {
    if minor == 0 {
        zero.to_string()
    } else {
        money::fmt(currency, minor)
    }
}

/// Lowercase with no spaces, so `50 ml`, `50ml` and `50 ML` are the same size.
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .replace("millilitres", "ml")
        .replace("milliliters", "ml")
}

/// Whether `size` is the size asked for: `50 ml` is in `50 ml` and in `2 × 50 ml`, never in
/// `150 ml`.
fn size_matches(size: &str, want: &str) -> bool {
    let size = squash(size);
    let digit = |c: char| c.is_ascii_digit() || c == '.';
    size.match_indices(want).any(|(i, _)| {
        !size[..i].chars().next_back().is_some_and(digit)
            && !size[i + want.len()..].chars().next().is_some_and(digit)
    })
}

/// Whether the family or section is the one asked for, either way round: `citrus` finds `Fresh
/// citrus`, and `gift sets` finds the `gift` section.
fn family_matches(family: Option<&str>, section: &str, want: &str) -> bool {
    let hit = |have: &str| {
        let have = have.trim().to_lowercase();
        !have.is_empty() && (have.contains(want) || want.contains(have.as_str()))
    };
    family.is_some_and(hit) || hit(section)
}

fn distinct<'a>(
    products: &'a [Product],
    key: impl Fn(&'a Product) -> Option<&'a str>,
) -> Vec<&'a str> {
    let mut seen: Vec<&str> = Vec::new();
    for p in products {
        if let Some(k) = key(p).filter(|k| !k.trim().is_empty())
            && !seen.contains(&k)
        {
            seen.push(k);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(
        sku: &str,
        section: &str,
        name: &str,
        family: &str,
        size: &str,
        price: i64,
    ) -> Product {
        Product {
            sku: sku.into(),
            slug: sku.into(),
            section: section.into(),
            name: name.into(),
            description: format!("About {name}."),
            family: (!family.is_empty()).then(|| family.into()),
            size: (!size.is_empty()).then(|| size.into()),
            price_minor: price * 100,
            attrs: r#"{"notes":"Amber, oud","tone":"amber","gift_ready":true,"strength":2}"#.into(),
            status: "published".into(),
            sort: 0,
        }
    }

    fn stock(sku: &str, state: &'static str, left: i64) -> (String, Stock) {
        (
            sku.into(),
            Stock {
                sku: sku.into(),
                left: Some(left),
                next_batch_at: None,
                back_at: None,
                as_of: 0,
                state,
            },
        )
    }

    fn shop() -> (Vec<Product>, HashMap<String, Stock>) {
        let products = vec![
            product(
                "amber-30",
                "fragrance",
                "Amber Noir",
                "Amber",
                "30 ml",
                4200,
            ),
            product(
                "amber-50",
                "fragrance",
                "Amber Noir",
                "Amber",
                "50 ml",
                6400,
            ),
            product(
                "cedar",
                "fragrance",
                "Highland Cedar",
                "Woody",
                "150 ml",
                9900,
            ),
            product(
                "duo",
                "gift",
                "Citrus Travel Duo",
                "Fresh citrus",
                "2 × 50 ml",
                4400,
            ),
            product("sample", "sample", "Free 2 ml sample", "", "2 ml", 0),
        ];
        let stock = [
            stock("amber-30", "out", 0),
            stock("amber-50", "low", 3),
            stock("cedar", "in", 9),
        ]
        .into_iter()
        .collect();
        (products, stock)
    }

    fn rows<'a>(products: &'a [Product], stock: &'a HashMap<String, Stock>) -> Rows<'a> {
        Rows {
            currency: "KES",
            utc_offset_min: 180,
            products,
            stock,
            price_list: None,
        }
    }

    fn got(rows: &Rows<'_>, args: Value) -> Value {
        page(rows, &Filter::from_args(&args))
    }

    fn names(v: &Value) -> Vec<String> {
        v["products"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|l| l.as_str().unwrap().split(" · ").next().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn one_line_per_product_with_every_size_price_and_stock() {
        let (products, stock) = shop();
        let v = got(&rows(&products, &stock), json!({}));
        assert_eq!(v["matching"], 4);
        assert_eq!(
            v["products"][0],
            "Amber Noir · fragrance · Amber · 30 ml KSh 4,200 (sold out); 50 ml KSh 6,400 (low, 3 left)"
        );
        assert_eq!(v["products"][3], "Free 2 ml sample · sample · 2 ml free");
        assert!(v.get("more").is_none());
    }

    #[test]
    fn a_size_is_that_size_and_never_a_longer_number() {
        let (products, stock) = shop();
        let r = rows(&products, &stock);
        let v = got(&r, json!({ "size": "50ML" }));
        assert_eq!(names(&v), ["Amber Noir", "Citrus Travel Duo"]);
        assert_eq!(
            v["products"][0], "Amber Noir · fragrance · Amber · 50 ml KSh 6,400 (low, 3 left)",
            "only the size asked for"
        );
        assert_eq!(
            names(&got(&r, json!({ "size": "150 ml" }))),
            ["Highland Cedar"]
        );
    }

    #[test]
    fn filters_by_family_category_name_and_stock() {
        let (products, stock) = shop();
        let r = rows(&products, &stock);
        assert_eq!(
            names(&got(&r, json!({ "family_or_category": "citrus" }))),
            ["Citrus Travel Duo"]
        );
        assert_eq!(
            names(&got(&r, json!({ "family_or_category": "Gift sets" }))),
            ["Citrus Travel Duo"]
        );
        assert_eq!(
            names(&got(&r, json!({ "name_contains": "NOIR" }))),
            ["Amber Noir"]
        );
        let v = got(
            &r,
            json!({ "in_stock_only": true, "name_contains": "amber" }),
        );
        assert_eq!(
            v["products"][0],
            "Amber Noir · fragrance · Amber · 50 ml KSh 6,400 (low, 3 left)"
        );
        let none = got(&r, json!({ "size": "75 ml" }));
        assert_eq!(none["matching"], 0);
        assert!(none["sizes"].as_array().unwrap().contains(&json!("50 ml")));
        assert!(
            none["families"]
                .as_array()
                .unwrap()
                .contains(&json!("Woody"))
        );
    }

    #[test]
    fn details_add_the_description_and_attributes_without_display_hints() {
        let (products, stock) = shop();
        let v = got(
            &rows(&products, &stock),
            json!({ "name_contains": "cedar", "details": true }),
        );
        let line = v["products"][0].as_str().unwrap();
        assert!(
            line.starts_with("Highland Cedar · fragrance · Woody · 150 ml KSh 9,900 (in stock, 9 left) · About Highland Cedar.; "),
            "{line}"
        );
        assert!(
            line.contains("notes: Amber, oud") && line.contains("gift ready"),
            "{line}"
        );
        assert!(
            !line.contains("tone") && !line.contains("strength"),
            "{line}"
        );
    }

    #[test]
    fn a_long_catalogue_pages_with_more_instead_of_cutting() {
        let products: Vec<Product> = (0..300)
            .map(|i| {
                product(
                    &format!("p{i}"),
                    "fragrance",
                    &format!("Scent number {i:03}"),
                    "Woody",
                    "50 ml",
                    5000 + i,
                )
            })
            .collect();
        let stock = HashMap::new();
        let r = rows(&products, &stock);
        let mut seen = Vec::new();
        let mut args = json!({ "size": "50 ml" });
        loop {
            let v = got(&r, args.clone());
            assert!(
                v.to_string().chars().count() < super::super::TOOL_CHARS,
                "a page fits"
            );
            seen.extend(names(&v));
            match v["more"].as_str() {
                Some(more) => args["more"] = json!(more),
                None => break,
            }
        }
        assert_eq!(seen.len(), 300, "every product, once");
        assert_eq!(seen.last().unwrap(), "Scent number 299");
        let past = got(&r, json!({ "more": "300" }));
        assert!(past["note"].as_str().unwrap().contains("last page"));
    }

    #[test]
    fn the_price_list_comes_on_the_first_page_by_group() {
        let (products, stock) = shop();
        let list = PriceList {
            version: "v31".into(),
            currency: "KES".into(),
            status: "live".into(),
            usd_rate_minor: 0,
            valid_from: 0,
            valid_until: 0,
            source: "owner".into(),
        };
        let rule = |grp: &str, label: &str, amount: i64| Rule {
            code: format!("{grp}.{label}"),
            grp: grp.into(),
            label: label.into(),
            amount_minor: amount * 100,
            meta: String::new(),
            sort: 0,
        };
        let rules = vec![
            rule("size", "1 tier", 4000),
            rule("size", "2 tiers", 4800),
            rule("wrap", "Gift wrap", 0),
        ];
        let mut r = rows(&products, &stock);
        r.price_list = Some((&list, &rules));
        let v = got(&r, json!({}));
        assert_eq!(
            v["price_list"]["choices"],
            json!([
                "size: 1 tier KSh 4,000, 2 tiers KSh 4,800",
                "wrap: Gift wrap included"
            ])
        );
        assert_eq!(
            got(&r, json!({ "name_contains": "wrap" }))["price_list"]["choices"],
            json!(["wrap: Gift wrap included"])
        );
        assert!(
            got(&r, json!({ "size": "50 ml" }))
                .get("price_list")
                .is_none()
        );
    }
}
