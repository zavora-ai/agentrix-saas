//! Tana Valley Times's pages (01a §2.7, N-01 to N-10), its desks' live facts and the answers
//! the desks give without a model.
//!
//! Every figure comes from records: stories, bylines, corrections, the story so far,
//! subscribers and digest subscriptions. A subscriber story shows its headline and link to
//! everyone and its standfirst and full text only to a visitor with an active subscription.
//! Ask retrieves from free stories' paragraphs and, for subscriber stories, from the headline
//! only, unless the visitor subscribes.

use std::collections::{BTreeMap, HashMap, HashSet};

use agentrix_records::news::{Author, Correction, Story};
use agentrix_records::{Records, Site, money, time};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Extension, Form, Router};
use chrono::{Datelike as _, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::App;
use crate::ask::{Action, Answer, Section, Source, plain, said};
use crate::ports::{PaymentRequest, PaymentStart};
use crate::shell::{DeskFacts, Fact, asof, urlencode};
use crate::web::{Ctx, Page, html, see_other};

/// Tana's page routes.
pub fn routes() -> Router<App> {
    Router::new()
        .route("/", get(home))
        .route("/archive", get(archive))
        .route("/corrections", get(corrections))
        .route("/digest", get(digest).post(digest_start))
        .route("/digest/confirm", get(digest_confirm))
        .route("/digest/stop", get(digest_stop))
        .route("/subscribe", get(subscribe).post(subscribe_pay))
        .route("/subscribe/paid", get(subscribe_paid))
        .route("/stories/{topic}", get(topic_page))
        .route("/{section}", get(section_page))
        .route("/{year}/{month}/{slug}", get(story_page))
        .fallback(missing)
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];
const WEEKDAYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

/// A section of the paper, from the manifest.
#[derive(Debug, Clone, Serialize)]
pub struct SectionInfo {
    /// `transport`, also its path.
    pub id: String,
    /// "Transport".
    pub name: String,
    /// What it covers.
    pub about: String,
}

/// The sections in a site's manifest.
pub fn sections_of(site: &Site) -> Vec<SectionInfo> {
    site.facts
        .get("sections")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    Some(SectionInfo {
                        id: s.get("id")?.as_str()?.to_owned(),
                        name: s.get("name")?.as_str()?.to_owned(),
                        about: s
                            .get("about")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A story as cards, lists and the story page show it.
#[derive(Debug, Clone, Serialize)]
pub struct Card {
    /// Slug.
    pub slug: String,
    /// `/2026/11/<slug>`.
    pub href: String,
    /// Section id.
    pub section: String,
    /// Section name.
    pub section_name: String,
    /// Kicker, or the section name.
    pub kicker: String,
    /// Headline.
    pub title: String,
    /// Standfirst.
    pub standfirst: String,
    /// Byline name.
    pub author: String,
    /// Byline role.
    pub author_role: String,
    /// "Thu 5 Nov", with the year when it isn't this year.
    pub date: String,
    /// "11:04".
    pub clock: String,
    /// "Thu 5 Nov 2026".
    pub date_long: String,
    /// RFC 3339, for `<time>`.
    pub iso: String,
    /// Subscriber-only.
    pub locked: bool,
    /// "16:20" (same day) or "Fri 6 Nov" when corrected, else empty.
    pub corrected: String,
    /// The running story it belongs to, or empty.
    pub topic: String,
    /// The weekly issue it ran in.
    pub issue: i64,
}

/// A step of a story so far.
#[derive(Debug, Clone, Serialize)]
pub struct Step {
    /// "Thu 5 Nov, 11:04", "January · scheduled".
    pub when: String,
    /// The line.
    pub line: String,
    /// The story it links to, or empty.
    pub href: String,
    /// `step`, `correction`, `scheduled`.
    pub kind: String,
}

/// The paper loaded once per request: published stories (newest first), bylines,
/// corrections and sections.
struct Paper {
    site: Site,
    off: i32,
    now: i64,
    stories: Vec<Story>,
    authors: HashMap<String, Author>,
    corrections: Vec<Correction>,
    sections: Vec<SectionInfo>,
    first_monday: Option<NaiveDate>,
}

impl Paper {
    async fn load(records: &Records, site: &Site) -> anyhow::Result<Self> {
        let now = time::now();
        let stories: Vec<Story> = records
            .stories(&site.site, None, 100_000)
            .await?
            .into_iter()
            .filter(|s| s.published_at <= now)
            .collect();
        let off = site.utc_offset_min;
        let first_monday = stories.last().map(|s| monday(s.published_at, off));
        Ok(Self {
            authors: records
                .authors(&site.site)
                .await?
                .into_iter()
                .map(|a| (a.author_id.clone(), a))
                .collect(),
            corrections: records
                .corrections(&site.site, None)
                .await?
                .into_iter()
                .filter(|c| c.at <= now)
                .collect(),
            sections: sections_of(site),
            site: site.clone(),
            off,
            now,
            stories,
            first_monday,
        })
    }

    fn href(&self, s: &Story) -> String {
        let l = time::local(s.published_at, self.off);
        format!("/{}/{:02}/{}", l.year(), l.month(), s.slug)
    }

    fn author(&self, s: &Story) -> String {
        self.authors
            .get(&s.author_id)
            .map(|a| a.name.clone())
            .unwrap_or_default()
    }

    fn section_name(&self, id: &str) -> String {
        self.sections
            .iter()
            .find(|s| s.id == id)
            .map_or_else(|| id.to_owned(), |s| s.name.clone())
    }

    /// "Thu 5 Nov", or "Tue 2 Sep 2025" when not this year.
    fn day(&self, ts: i64) -> String {
        if time::local(ts, self.off).year() == time::local(self.now, self.off).year() {
            time::fmt_day(ts, self.off)
        } else {
            time::fmt_date_long(ts, self.off)
        }
    }

    /// "Amani Otieno · Thu 5 Nov": the label a citation shows.
    fn label(&self, s: &Story) -> String {
        format!("{} \u{b7} {}", self.author(s), self.day(s.published_at))
    }

    fn correction(&self, slug: &str) -> Option<&Correction> {
        self.corrections.iter().find(|c| c.story_slug == slug)
    }

    fn issue(&self, ts: i64) -> i64 {
        self.first_monday
            .map_or(1, |first| (monday(ts, self.off) - first).num_days() / 7 + 1)
    }

    fn card(&self, s: &Story) -> Card {
        let corrected = self.correction(&s.slug).map_or_else(String::new, |c| {
            if time::day_of(c.at, self.off) == time::day_of(s.published_at, self.off) {
                time::fmt_clock(c.at, self.off)
            } else {
                self.day(c.at)
            }
        });
        Card {
            slug: s.slug.clone(),
            href: self.href(s),
            section: s.section.clone(),
            section_name: self.section_name(&s.section),
            kicker: s
                .kicker
                .clone()
                .unwrap_or_else(|| self.section_name(&s.section)),
            title: s.title.clone(),
            standfirst: s.standfirst.clone(),
            author: self.author(s),
            author_role: self
                .authors
                .get(&s.author_id)
                .map(|a| a.role.clone())
                .unwrap_or_default(),
            date: self.day(s.published_at),
            clock: time::fmt_clock(s.published_at, self.off),
            date_long: time::fmt_date_long(s.published_at, self.off),
            iso: time::local(s.published_at, self.off).to_rfc3339(),
            locked: s.locked(),
            corrected,
            topic: s.topic.clone().unwrap_or_default(),
            issue: self.issue(s.published_at),
        }
    }

    fn topic_title(&self, topic: &str) -> String {
        topic_fact(&self.site, topic, "title")
    }

    /// The topic as it reads mid-sentence: "the northern rail extension", "Ndovu market's move".
    fn topic_phrase(&self, topic: &str) -> String {
        let phrase = topic_fact(&self.site, topic, "phrase");
        if phrase.is_empty() {
            self.topic_title(topic)
        } else {
            phrase
        }
    }

    /// "07:30": the time a live answer read the records.
    fn clock(&self) -> String {
        time::fmt_clock(self.now, self.off)
    }

    fn source(&self, s: &Story, locked_for_reader: bool) -> Source {
        let mut src = Source::new(&self.label(s), &self.href(s));
        src.lock = locked_for_reader;
        src
    }
}

fn topic_fact(site: &Site, topic: &str, key: &str) -> String {
    site.facts
        .get("topics")
        .and_then(|t| t.get(topic))
        .and_then(|t| t.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn monday(ts: i64, off: i32) -> NaiveDate {
    let d = time::local(ts, off).date_naive();
    d - chrono::Duration::days(i64::from(d.weekday().num_days_from_monday()))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Whether the signed-in visitor holds an active subscription: by account id, or by the
/// customer row with the same email (seeded readers who sign in through W6's accounts).
async fn subscriber(records: &Records, ctx: &Ctx, now: i64) -> anyhow::Result<bool> {
    let Some(v) = &ctx.visitor else {
        return Ok(false);
    };
    if records
        .is_subscriber(&ctx.site.site, &v.account, now)
        .await?
    {
        return Ok(true);
    }
    match records.customer_by_email(&ctx.site.site, &v.email).await? {
        Some(c) if c.customer_id != v.account => {
            records
                .is_subscriber(&ctx.site.site, &c.customer_id, now)
                .await
        }
        _ => Ok(false),
    }
}

/// The subscription row for the signed-in visitor, if any.
async fn subscription_of(
    records: &Records,
    ctx: &Ctx,
) -> anyhow::Result<Option<agentrix_records::news::Subscriber>> {
    let Some(v) = &ctx.visitor else {
        return Ok(None);
    };
    let by_email = records
        .customer_by_email(&ctx.site.site, &v.email)
        .await?
        .map(|c| c.customer_id);
    Ok(records
        .subscribers(&ctx.site.site)
        .await?
        .into_iter()
        .find(|s| s.customer_id == v.account || Some(&s.customer_id) == by_email.as_ref()))
}

/// The view every Tana page starts from: the shell, today's date and the sections.
pub(crate) async fn view(
    app: &App,
    ctx: &Ctx,
    current: &str,
    title: &str,
    description: &str,
    extra: Value,
) -> anyhow::Result<Value> {
    let shell = app.shell(ctx, current, title, description).await?;
    let now = time::now();
    let mut v = json!({
        "shell": shell,
        "today": time::local(now, ctx.site.utc_offset_min).format("%A %-d %B %Y").to_string(),
        "sections": sections_of(&ctx.site),
        "tagline": ctx.site.fact("/tagline"),
    });
    if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
        o.extend(e);
    }
    Ok(v)
}

async fn steps(records: &Records, paper: &Paper, topic: &str) -> anyhow::Result<Vec<Step>> {
    let week_ago = paper.now - 7 * 86_400;
    let mut out = Vec::new();
    for t in records.timeline(&paper.site.site, topic).await? {
        let href = match t
            .story_slug
            .as_deref()
            .and_then(|slug| paper.stories.iter().find(|s| s.slug == slug))
        {
            Some(s) if t.kind == "correction" => format!("{}#correction", paper.href(s)),
            Some(s) => paper.href(s),
            None => String::new(),
        };
        if t.kind != "scheduled" && t.at > paper.now {
            continue;
        }
        let when = match t.kind.as_str() {
            "scheduled" => format!(
                "{} \u{b7} scheduled",
                time::local(t.at, paper.off).format("%B")
            ),
            "correction" => format!("{} \u{b7} correction", time::fmt_day_clock(t.at, paper.off)),
            _ if t.at >= week_ago => time::fmt_day_clock(t.at, paper.off),
            _ => paper.day(t.at),
        };
        out.push(Step {
            when,
            line: t.line,
            href,
            kind: t.kind,
        });
    }
    Ok(out)
}

fn digest_view(site: &Site, topics: &[String]) -> Value {
    let d = site.facts.get("digest").cloned().unwrap_or(Value::Null);
    let chosen = if topics.is_empty() {
        "front-page".to_owned()
    } else {
        topics.join(" and ").to_lowercase()
    };
    let rows: Vec<Value> = d
        .get("contract")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    let k = r.get(0)?.as_str()?;
                    let tpl = r.get(1)?.as_str()?;
                    Some(json!({ "k": k, "template": tpl, "text": tpl.replace("{topics}", &chosen) }))
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "name": d.get("name").and_then(Value::as_str).unwrap_or("Digest"),
        "rows": rows,
        "day": d.get("day").and_then(Value::as_str).unwrap_or(""),
        "time": d.get("time").and_then(Value::as_str).unwrap_or(""),
        "count": d.get("count").and_then(Value::as_i64).unwrap_or(5),
    })
}

fn house_rules(site: &Site) -> Vec<String> {
    site.facts
        .get("house_rules")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------- pages

async fn home(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let records = app.records().await?;
    let site = &ctx.site;
    let paper = Paper::load(records, site).await?;
    let sub = subscriber(records, &ctx, paper.now).await?;
    let lead_slug = site.fact("/front/lead");
    let lead = paper
        .stories
        .iter()
        .find(|s| s.slug == lead_slug)
        .or(paper.stories.first());
    let more: Vec<Card> = paper
        .stories
        .iter()
        .filter(|s| lead.is_none_or(|l| l.slug != s.slug))
        .take(6)
        .map(|s| paper.card(s))
        .collect();
    let topic = site.fact("/front/topic");
    let days = site
        .facts
        .pointer("/device_memory/sample/last_visit_days_ago")
        .and_then(Value::as_i64)
        .unwrap_or(5);
    let sample_last = time::day_start(site.anchor_at, paper.off) - days * 86_400 + 12 * 3600;
    let follows = site
        .facts
        .pointer("/device_memory/sample/follows")
        .cloned()
        .unwrap_or(Value::Null);
    let v = view(
        &app,
        &ctx,
        "newsroom",
        &site.name,
        "The valley's weekly paper: council, transport, farming, business and community news. A sample site.",
        json!({
            "lead": lead.map(|s| paper.card(s)),
            "more": more,
            "subscriber": sub,
            "topic": topic,
            "topic_title": paper.topic_title(&topic),
            "steps": steps(records, &paper, &topic).await?,
            "digest": digest_view(site, &[]),
            "house_rules": house_rules(site),
            "memory": { "sample_last": time::day_of(sample_last, paper.off), "sample_follows": follows },
        }),
    )
    .await?;
    Ok(html(app.render("tana/home.html", &v)?))
}

async fn section_page(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Path(section): Path<String>,
) -> Page {
    let Some(info) = sections_of(&ctx.site).into_iter().find(|s| s.id == section) else {
        return missing_page(&app, &ctx).await;
    };
    let records = app.records().await?;
    let paper = Paper::load(records, &ctx.site).await?;
    let sub = subscriber(records, &ctx, paper.now).await?;
    let cards: Vec<Card> = paper
        .stories
        .iter()
        .filter(|s| s.section == info.id)
        .map(|s| paper.card(s))
        .collect();
    let corrected = cards.iter().filter(|c| !c.corrected.is_empty()).count();
    let v = view(
        &app,
        &ctx,
        "",
        &info.name,
        &info.about,
        json!({ "section": info, "stories": cards, "corrected": corrected, "subscriber": sub }),
    )
    .await?;
    Ok(html(app.render("tana/section.html", &v)?))
}

async fn story_page(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Path((year, month, slug)): Path<(String, String, String)>,
) -> Page {
    let records = app.records().await?;
    let paper = Paper::load(records, &ctx.site).await?;
    let Some(s) = paper.stories.iter().find(|s| s.slug == slug) else {
        return missing_page(&app, &ctx).await;
    };
    let card = paper.card(s);
    if card.href != format!("/{year}/{month}/{slug}") {
        return Ok(see_other(&card.href));
    }
    let sub = subscriber(records, &ctx, paper.now).await?;
    let open = !s.locked() || sub;
    let paragraphs: Vec<Value> = if open {
        s.paragraphs()
            .into_iter()
            .enumerate()
            .map(|(i, p)| json!({ "id": format!("p{}", i + 1), "text": p }))
            .collect()
    } else {
        Vec::new()
    };
    let correction = paper.correction(&s.slug).map(
        |c| json!({ "when": time::fmt_day_clock(c.at, paper.off), "what": c.what, "note": c.note }),
    );
    let topic = s.topic.clone().unwrap_or_default();
    let topic_steps = if topic.is_empty() {
        Vec::new()
    } else {
        steps(records, &paper, &topic).await?
    };
    let more: Vec<Card> = paper
        .stories
        .iter()
        .filter(|o| o.section == s.section && o.slug != s.slug)
        .take(4)
        .map(|o| paper.card(o))
        .collect();
    // A subscriber story shows non-subscribers its headline and link only, here too.
    let description = if open {
        s.standfirst.clone()
    } else {
        format!(
            "A subscriber story in {}, by {}.",
            card.section_name, card.author
        )
    };
    let ld = json!({
        "@context": "https://schema.org",
        "@type": "NewsArticle",
        "headline": s.title,
        "description": description,
        "datePublished": card.iso,
        "dateModified": time::local(s.updated_at, paper.off).to_rfc3339(),
        "author": { "@type": "Person", "name": card.author },
        "publisher": { "@type": "Organization", "name": ctx.site.name },
        "articleSection": card.section_name,
        "isAccessibleForFree": !s.locked(),
    });
    let v = view(
        &app,
        &ctx,
        "",
        &s.title,
        &description,
        json!({
            "story": card,
            "subscriber": sub,
            "open": open,
            "paragraphs": paragraphs,
            "correction": correction,
            "topic_title": paper.topic_title(&topic),
            "steps": topic_steps,
            "more": more,
            "ld": ld,
            "sign_in": format!("/account/sign-in?why=save&next={}", urlencode(&ctx.path)),
            "bio": paper.authors.get(&s.author_id).map(|a| a.bio.clone()).unwrap_or_default(),
        }),
    )
    .await?;
    Ok(html(app.render("tana/story.html", &v)?))
}

async fn topic_page(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Path(topic): Path<String>,
) -> Page {
    let records = app.records().await?;
    let paper = Paper::load(records, &ctx.site).await?;
    let title = paper.topic_title(&topic);
    if title.is_empty() {
        return missing_page(&app, &ctx).await;
    }
    let cards: Vec<Card> = paper
        .stories
        .iter()
        .filter(|s| s.topic.as_deref() == Some(topic.as_str()))
        .map(|s| paper.card(s))
        .collect();
    let about = topic_fact(&ctx.site, &topic, "about");
    let v = view(
        &app,
        &ctx,
        "newsroom",
        &format!("Story so far: {title}"),
        &about,
        json!({
            "topic": topic,
            "title": title,
            "about": about,
            "steps": steps(records, &paper, &topic).await?,
            "stories": cards,
        }),
    )
    .await?;
    Ok(html(app.render("tana/topic.html", &v)?))
}

async fn corrections(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let records = app.records().await?;
    let paper = Paper::load(records, &ctx.site).await?;
    let list: Vec<Value> = paper
        .corrections
        .iter()
        .filter_map(|c| {
            let s = paper.stories.iter().find(|s| s.slug == c.story_slug)?;
            Some(json!({
                "id": format!("c-{}", c.story_slug),
                "when": time::fmt_day_clock(c.at, paper.off),
                "what": c.what,
                "note": c.note,
                "story": paper.card(s),
            }))
        })
        .collect();
    let v = view(
        &app,
        &ctx,
        "newsroom",
        "Corrections",
        "Every correction Tana Valley Times has published, newest first, with the note readers see on the story.",
        json!({ "corrections": list, "editor": ctx.site.fact("/owner/name"), "editor_email": ctx.site.fact("/owner/email") }),
    )
    .await?;
    Ok(html(app.render("tana/corrections.html", &v)?))
}

#[derive(Deserialize)]
struct ArchiveQuery {
    section: Option<String>,
}

async fn archive(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Query(q): Query<ArchiveQuery>,
) -> Page {
    let records = app.records().await?;
    let paper = Paper::load(records, &ctx.site).await?;
    let section = q
        .section
        .filter(|s| paper.sections.iter().any(|x| &x.id == s))
        .unwrap_or_default();
    let mut years: BTreeMap<i32, BTreeMap<u32, Vec<Card>>> = BTreeMap::new();
    let shown: Vec<&Story> = paper
        .stories
        .iter()
        .filter(|s| section.is_empty() || s.section == section)
        .collect();
    for s in &shown {
        let l = time::local(s.published_at, paper.off);
        years
            .entry(l.year())
            .or_default()
            .entry(l.month())
            .or_default()
            .push(paper.card(s));
    }
    let years: Vec<Value> = years
        .into_iter()
        .rev()
        .map(|(y, months)| {
            let count: usize = months.values().map(Vec::len).sum();
            let months: Vec<Value> = months
                .into_iter()
                .rev()
                .map(|(m, cards)| {
                    let name = MONTHS[(m - 1) as usize];
                    json!({
                        "id": format!("m-{y}-{m:02}"),
                        "name": format!("{}{} {y}", name[..1].to_uppercase(), &name[1..]),
                        "stories": cards,
                    })
                })
                .collect();
            json!({ "year": y, "id": format!("y-{y}"), "count": count, "months": months })
        })
        .collect();
    let issues: HashSet<i64> = shown.iter().map(|s| paper.issue(s.published_at)).collect();
    let section_name = if section.is_empty() {
        String::new()
    } else {
        paper.section_name(&section)
    };
    let v = view(
        &app,
        &ctx,
        "archive",
        "Archive",
        "Every story Tana Valley Times has published online, by year, month and section.",
        json!({
            "years": years,
            "section": section,
            "section_name": section_name,
            "total": shown.len(),
            "issues": issues.len(),
        }),
    )
    .await?;
    Ok(html(app.render("tana/archive.html", &v)?))
}

async fn digest(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let v = digest_page(
        &app,
        &ctx,
        json!({ "state": "form", "error": "", "email": "" }),
    )
    .await?;
    Ok(html(app.render("tana/digest.html", &v)?))
}

async fn digest_page(app: &App, ctx: &Ctx, extra: Value) -> anyhow::Result<Value> {
    let mut extra = extra;
    if let Some(o) = extra.as_object_mut() {
        o.insert("digest".into(), digest_view(&ctx.site, &[]));
        o.entry("sent").or_insert(Value::Null);
        o.entry("topics").or_insert(json!([]));
    }
    view(
        app,
        ctx,
        "subscriptions",
        "Your Friday five",
        "Choose topics for the Friday five: five published stories, two lines each, with links. We email you a link to confirm first. One tap stops it.",
        extra,
    )
    .await
}

async fn digest_start(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Form(form): Form<HashMap<String, String>>,
) -> Page {
    let email = form
        .get("email")
        .map(|e| e.trim().to_owned())
        .unwrap_or_default();
    let topics: Vec<String> = sections_of(&ctx.site)
        .into_iter()
        .filter(|s| form.contains_key(&format!("t-{}", s.id)))
        .map(|s| s.name)
        .collect();
    let valid = email.len() <= 200
        && email
            .split_once('@')
            .is_some_and(|(a, b)| !a.is_empty() && b.contains('.') && !b.starts_with('.'));
    if !valid {
        let v = digest_page(
            &app,
            &ctx,
            json!({ "state": "form", "error": "Write your email address, like name@example.com. Nothing is sent until you confirm it.", "email": email }),
        )
        .await?;
        return Ok(html(app.render("tana/digest.html", &v)?));
    }
    let records = app.records().await?;
    let (_, token) = records
        .start_digest(&ctx.site.site, &email, &topics, time::now())
        .await?;
    // Double opt-in: the confirmation message goes through W9's outbox (shown in the owner's
    // Messages app, Not sent until email is on); the page shows the same link as the Sample
    // mailbox.
    app.tell_digest_confirm(
        &ctx.site,
        &ctx.host.host,
        &email.to_lowercase(),
        &topics.join(", "),
        &format!("{}/digest/confirm?token={token}", ctx.host.origin),
    )
    .await;
    let v = digest_page(
        &app,
        &ctx,
        json!({
            "state": "sent",
            "error": "",
            "email": email.to_lowercase(),
            "topics": topics,
            "sent": { "confirm": format!("/digest/confirm?token={token}"), "stop": format!("/digest/stop?token={token}") },
        }),
    )
    .await?;
    Ok(html(app.render("tana/digest.html", &v)?))
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

async fn digest_confirm(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Query(q): Query<TokenQuery>,
) -> Page {
    digest_token(app, ctx, q.token.unwrap_or_default(), true).await
}

async fn digest_stop(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Query(q): Query<TokenQuery>,
) -> Page {
    digest_token(app, ctx, q.token.unwrap_or_default(), false).await
}

async fn digest_token(app: App, ctx: Ctx, token: String, confirm: bool) -> Page {
    let records = app.records().await?;
    let token: String = token
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(64)
        .collect();
    let found = if token.is_empty() {
        None
    } else {
        records
            .digest_by_token(&ctx.site.site, &token, confirm, time::now())
            .await?
    };
    if let (Some(d), false) = (&found, confirm)
        && d.state == "stopped"
    {
        app.tell_digest_stopped(
            &ctx.site,
            &ctx.host.host,
            &d.email,
            &format!("{}/digest", ctx.host.origin),
        )
        .await;
    }
    let extra = match found {
        Some(d) => {
            let topics: Vec<String> = serde_json::from_str(&d.topics).unwrap_or_default();
            json!({
                "state": d.state,
                "error": "",
                "email": d.email,
                "topics": topics,
                "sent": { "confirm": format!("/digest/confirm?token={token}"), "stop": format!("/digest/stop?token={token}") },
            })
        }
        None => json!({ "state": "invalid", "error": "", "email": "" }),
    };
    let v = digest_page(&app, &ctx, extra).await?;
    Ok(html(app.render("tana/digest.html", &v)?))
}

/// The subscription price as the subscribe page and the Subscriptions desk show it: the
/// offer's shillings, and the US dollars PayPal is asked for at the posted rate.
struct Price {
    /// `KSh 300`.
    local: String,
    /// The US cents PayPal is asked for.
    usd_cents: i64,
    /// `USD 2.33`.
    usd: String,
    /// `1 USD = KSh 129.00`.
    rate: String,
}

impl Price {
    fn of(site: &Site) -> Self {
        let offer = site.facts.get("subscription");
        let num = |k: &str| {
            offer
                .and_then(|o| o.get(k))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        };
        let local_minor = (num("price") * 100.0).round() as i64;
        let rate_minor = (num("usd_rate") * 100.0).round() as i64;
        let usd_cents = money::usd_cents(local_minor, rate_minor);
        Self {
            local: money::fmt(&site.currency, local_minor),
            usd_cents,
            usd: money::usd(usd_cents),
            rate: money::rate_line(rate_minor),
        }
    }
}

/// How a subscription's source reads to its reader.
fn source_words(source: &str) -> String {
    match source {
        "owner grant" => "Given by the editor".into(),
        "sandbox payment" => "Paid with PayPal (sandbox)".into(),
        "sample payment" => "Paid with a sample payment; no money moved".into(),
        other => other.to_owned(),
    }
}

#[derive(Deserialize)]
struct SubscribeQuery {
    paid: Option<String>,
    cancelled: Option<String>,
}

async fn subscribe(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Query(q): Query<SubscribeQuery>,
) -> Page {
    let flash = if q.paid.is_some() {
        "paid"
    } else if q.cancelled.is_some() {
        "cancelled"
    } else {
        ""
    };
    subscribe_view(&app, &ctx, flash, "").await
}

async fn subscribe_view(app: &App, ctx: &Ctx, flash: &str, error: &str) -> Page {
    let records = app.records().await?;
    let now = time::now();
    let off = ctx.site.utc_offset_min;
    let paper = Paper::load(records, &ctx.site).await?;
    let offer = ctx
        .site
        .facts
        .get("subscription")
        .cloned()
        .unwrap_or(Value::Null);
    let price = Price::of(&ctx.site);
    let active = subscriber(records, ctx, now).await?;
    let mine = subscription_of(records, ctx).await?.map(|s| {
        json!({
            "plan": s.plan,
            "state": s.state,
            "source": source_words(&s.source),
            "since": time::fmt_date_long(s.since, off),
            "until": s.until_at.map(|u| time::fmt_date_long(u, off)).unwrap_or_default(),
        })
    });
    let locked = paper.stories.iter().filter(|s| s.locked()).count();
    let v = view(
        app,
        ctx,
        "subscriptions",
        "Subscribe",
        &format!(
            "Subscribe to read every subscriber story in full. {}.",
            offer.get("note").and_then(Value::as_str).unwrap_or("")
        ),
        json!({
            "offer": offer,
            "price": price.local,
            "usd": price.usd,
            "rate": price.rate,
            "locked": locked,
            "active": active,
            "mine": mine,
            "flash": flash,
            "error": error,
            "stand_in": app.payments.is_stand_in(),
            "sign_in": "/account/sign-in?why=save&next=/subscribe",
        }),
    )
    .await?;
    Ok(html(app.render("tana/subscribe.html", &v)?))
}

async fn subscribe_pay(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let Some(visitor) = ctx.visitor.clone() else {
        return Ok(see_other("/account/sign-in?why=save&next=/subscribe"));
    };
    let records = app.records().await?;
    let now = time::now();
    if subscriber(records, &ctx, now).await? {
        return Ok(see_other("/subscribe"));
    }
    let offer = ctx
        .site
        .facts
        .get("subscription")
        .cloned()
        .unwrap_or(Value::Null);
    let plan = offer
        .get("plan")
        .and_then(Value::as_str)
        .unwrap_or("Supporter")
        .to_owned();
    let days = offer.get("days").and_then(Value::as_i64).unwrap_or(30);
    if app.payments.is_stand_in() {
        records
            .grant_subscription(
                &ctx.site.site,
                &visitor.account,
                &plan,
                "sample payment",
                Some(now + days * 86_400),
                now,
            )
            .await?;
        return Ok(see_other("/subscribe?paid=1"));
    }
    let usd = Price::of(&ctx.site).usd_cents;
    let n = records
        .next_number(&ctx.site.site, "subscription", 1001)
        .await?;
    let request = PaymentRequest {
        site: ctx.site.site.clone(),
        order_id: format!("{}-SUB-{n}", ctx.site.fact("/order_prefix")),
        amount_usd_cents: usd,
        description: format!(
            "{} {plan} subscription, one {} (Sample)",
            ctx.site.name,
            offer
                .get("period")
                .and_then(Value::as_str)
                .unwrap_or("month")
        ),
        customer: Some(visitor.account.clone()),
        return_url: format!("{}/subscribe/paid", ctx.host.origin),
        cancel_url: format!("{}/subscribe?cancelled=1", ctx.host.origin),
    };
    match app.payments.create(request).await? {
        PaymentStart::Redirect { approve_url, .. } => Ok(see_other(&approve_url)),
        PaymentStart::NotExecuted { words } => subscribe_view(&app, &ctx, "", &words).await,
    }
}

async fn subscribe_paid(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Query(q): Query<TokenQuery>,
) -> Page {
    let Some(visitor) = ctx.visitor.clone() else {
        return Ok(see_other("/account/sign-in?why=save&next=/subscribe"));
    };
    let Some(token) = q.token.filter(|t| !t.is_empty()) else {
        return Ok(see_other("/subscribe"));
    };
    let capture = app.payments.capture(&ctx.site.site, &token).await?;
    if capture.state != "captured" {
        let words = match capture.state.as_str() {
            "pending" => {
                "PayPal is still confirming the payment. Reload this page in a few minutes; your subscription starts once PayPal confirms it."
            }
            "cancelled" => "The payment was cancelled at PayPal. You weren't charged.",
            _ => {
                "PayPal didn't take the payment, so you weren't charged. Try again, or ask the Subscriptions desk."
            }
        };
        return subscribe_view(&app, &ctx, "", words).await;
    }
    let records = app.records().await?;
    let now = time::now();
    let offer = ctx
        .site
        .facts
        .get("subscription")
        .cloned()
        .unwrap_or(Value::Null);
    let plan = offer
        .get("plan")
        .and_then(Value::as_str)
        .unwrap_or("Supporter");
    let days = offer.get("days").and_then(Value::as_i64).unwrap_or(30);
    records
        .grant_subscription(
            &ctx.site.site,
            &visitor.account,
            plan,
            "sandbox payment",
            Some(now + days * 86_400),
            now,
        )
        .await?;
    Ok(see_other("/subscribe?paid=1"))
}

async fn missing(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    missing_page(&app, &ctx).await
}

async fn missing_page(app: &App, ctx: &Ctx) -> Page {
    let records = app.records().await?;
    let paper = Paper::load(records, &ctx.site).await?;
    let latest: Vec<Card> = paper
        .stories
        .iter()
        .take(5)
        .map(|s| paper.card(s))
        .collect();
    let v = view(
        app,
        ctx,
        "",
        "Page not found",
        "That page isn't on Tana Valley Times.",
        json!({ "latest": latest }),
    )
    .await?;
    let mut r = html(app.render("tana/missing.html", &v)?);
    *r.status_mut() = StatusCode::NOT_FOUND;
    Ok(r)
}

// ---------------------------------------------------------------------------- desk facts

/// Live facts for Tana's desks.
pub async fn desk_facts(
    records: &Records,
    site: &Site,
    now: i64,
) -> anyhow::Result<HashMap<String, DeskFacts>> {
    let off = site.utc_offset_min;
    let stories: Vec<Story> = records
        .stories(&site.site, None, 100_000)
        .await?
        .into_iter()
        .filter(|s| s.published_at <= now)
        .collect();
    let week_start = time::day_start(now, off)
        - i64::from(time::local(now, off).weekday().num_days_from_monday()) * 86_400;
    let this_week = stories
        .iter()
        .filter(|s| s.published_at >= week_start)
        .count();
    let latest = records
        .corrections(&site.site, None)
        .await?
        .into_iter()
        .find(|c| c.at <= now);
    let mut out = HashMap::new();
    out.insert(
        "newsroom".into(),
        DeskFacts {
            facts: vec![
                Fact {
                    k: "This week".into(),
                    v: plural(this_week, "story", "stories"),
                },
                Fact {
                    k: "Latest correction".into(),
                    v: latest.map_or_else(|| "none".into(), |c| time::fmt_day_clock(c.at, off)),
                },
            ],
            asof: asof(site, now),
        },
    );
    let weeks: HashSet<NaiveDate> = stories
        .iter()
        .map(|s| monday(s.published_at, off))
        .collect();
    let first_year = stories
        .last()
        .map(|s| time::local(s.published_at, off).year().to_string())
        .unwrap_or_default();
    out.insert(
        "archive".into(),
        DeskFacts {
            facts: vec![
                Fact {
                    k: "Issues online".into(),
                    v: weeks.len().to_string(),
                },
                Fact {
                    k: "Archive from".into(),
                    v: first_year,
                },
            ],
            asof: String::new(),
        },
    );
    let subscribers = records
        .subscribers(&site.site)
        .await?
        .into_iter()
        .filter(|s| s.state == "active" && s.until_at.is_none_or(|u| u > now))
        .count();
    let digest = records
        .digest_subs(&site.site)
        .await?
        .into_iter()
        .filter(|d| d.state == "confirmed")
        .count();
    out.insert(
        "subscriptions".into(),
        DeskFacts {
            facts: vec![
                Fact {
                    k: "Subscribers".into(),
                    v: subscribers.to_string(),
                },
                Fact {
                    k: "Friday five".into(),
                    v: plural(digest, "reader", "readers"),
                },
            ],
            asof: asof(site, now),
        },
    );
    Ok(out)
}

// ------------------------------------------------------------------------- instant answers

fn has(t: &str, words: &[&str]) -> bool {
    words.iter().any(|w| t.contains(w))
}

/// The running story a question is about, by its id or a distinctive word of its title.
fn topic_in(site: &Site, t: &str) -> Option<String> {
    let topics = site.facts.get("topics")?.as_object()?;
    topics
        .iter()
        .find(|(id, v)| {
            t.contains(id.as_str())
                || v.get("title").and_then(Value::as_str).is_some_and(|title| {
                    title
                        .to_lowercase()
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|w| w.len() >= 5 && !["county", "market"].contains(w))
                        .any(|w| t.contains(w))
                })
        })
        .map(|(id, _)| id.clone())
}

fn sections_in(site: &Site, t: &str) -> Vec<SectionInfo> {
    sections_of(site)
        .into_iter()
        .filter(|s| t.contains(&s.id) || t.contains(&s.name.to_lowercase()))
        .collect()
}

fn month_index(word: &str) -> Option<u32> {
    let w = word.get(..3)?;
    MONTHS
        .iter()
        .position(|m| m.starts_with(w))
        .map(|i| i as u32 + 1)
}

/// "since Tue 3 Nov 2026", "since 3 Nov", "since tuesday", "since yesterday", "since last week".
fn since_in(t: &str, now: i64, off: i32) -> Option<i64> {
    let rest = t.split("since ").nth(1)?;
    let today = time::local(now, off).date_naive();
    let start = time::day_start(now, off);
    if rest.starts_with("yesterday") {
        return Some(start - 86_400);
    }
    if rest.starts_with("last week") {
        return Some(start - 7 * 86_400);
    }
    let tokens: Vec<&str> = rest
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(5)
        .collect();
    let mut day = None;
    let mut month = None;
    let mut year = None;
    for w in &tokens {
        if let Ok(n) = w.parse::<i32>() {
            if (1..=31).contains(&n) && day.is_none() {
                day = Some(n as u32);
            } else if (2000..=2100).contains(&n) {
                year = Some(n);
            }
        } else if w.len() >= 3 && month.is_none() && !WEEKDAYS.iter().any(|d| d.starts_with(*w)) {
            month = month_index(w);
        }
    }
    if let (Some(d), Some(m)) = (day, month) {
        let y = year.unwrap_or(today.year());
        let mut date = NaiveDate::from_ymd_opt(y, m, d)?;
        if year.is_none() && date > today {
            date = NaiveDate::from_ymd_opt(y - 1, m, d)?;
        }
        return time::at_local(date, "00:00", off).ok();
    }
    let first = tokens.first()?;
    let wd = WEEKDAYS
        .iter()
        .position(|d| d.starts_with(*first) && first.len() >= 3)? as i64;
    let back = (i64::from(today.weekday().num_days_from_monday()) - wd + 7) % 7;
    Some(start - if back == 0 { 7 } else { back } * 86_400)
}

/// What Tana's capabilities answer instantly, without a model (06 §4.7): who has been reporting
/// a story (bylines), today's front page, and whether your subscription is active. Matched on
/// whole words. Everything else, Catch me up included, goes to the desks' model.
pub async fn instant(app: &App, ctx: &Ctx, q: &str) -> anyhow::Result<Option<Answer>> {
    let p = plain(q);
    let bylines = said(
        &p,
        &[
            "who has been reporting",
            "whos been reporting",
            "who reported",
            "who wrote",
            "who is reporting",
            "who covers",
            "byline",
            "bylines",
        ],
    );
    let front = said(
        &p,
        &["front page", "todays paper", "top stories", "headlines"],
    );
    let status = said(
        &p,
        &[
            "my subscription",
            "am i subscribed",
            "am i a subscriber",
            "subscription active",
        ],
    );
    if !(bylines || front || status) {
        return Ok(None);
    }
    let records = app.records().await?;
    let site = &ctx.site;
    let paper = Paper::load(records, site).await?;
    let sub = subscriber(records, ctx, paper.now).await?;
    let t = q.to_lowercase();
    if bylines {
        let topic = topic_in(site, &t).unwrap_or_else(|| site.fact("/front/topic"));
        return Ok(Some(who(&paper, &topic)));
    }
    if front {
        return Ok(Some(front_page(&paper, sub)));
    }
    Ok(Some(
        subscription_answer(records, ctx, &paper, &t, sub).await?,
    ))
}

/// What Tana's rules answer for a question (catch-ups, where a story stands, the archive,
/// subscriptions, corrections): the facts the desks' model writes from, or the answer itself
/// when no model can (it then carries no AI label).
pub async fn rules(app: &App, ctx: &Ctx, q: &str) -> anyhow::Result<Option<Answer>> {
    let records = app.records().await?;
    let site = &ctx.site;
    let t = q.to_lowercase();
    let paper = Paper::load(records, site).await?;
    let now = paper.now;
    let sub = subscriber(records, ctx, now).await?;
    let h = |w: &[&str]| has(&t, w);

    if h(&[
        "catch me up",
        "catch up",
        "what did i miss",
        "what have i missed",
    ]) {
        let (since, label) = match since_in(&t, now, paper.off) {
            Some(s) => (s, time::fmt_day(s, paper.off)),
            None if h(&["today"]) => (time::day_start(now, paper.off), "this morning".to_owned()),
            None => {
                let s = time::day_start(now, paper.off) - 7 * 86_400;
                (s, time::fmt_day(s, paper.off))
            }
        };
        let follows = t
            .split("following")
            .nth(1)
            .map(|f| sections_in(site, f))
            .unwrap_or_default();
        return Ok(Some(catch_up(&paper, since, &label, &follows, sub)));
    }
    if h(&["front page", "today's paper", "top stories", "headlines"]) {
        return Ok(Some(front_page(&paper, sub)));
    }
    if h(&["digest", "friday five", "newsletter"])
        || (h(&["subscri"]) && h(&["stop", "cancel", "unsubscribe"]))
    {
        return Ok(Some(digest_answer(&paper, &t)));
    }
    if h(&["subscri", "renew", "supporter", "paywall", "locked"]) {
        return Ok(Some(
            subscription_answer(records, ctx, &paper, &t, sub).await?,
        ));
    }
    let topic = topic_in(site, &t);
    if h(&[
        "good idea",
        "bad idea",
        "should ",
        "opinion",
        "do you think",
        "worth it",
        "agree",
        "your view",
        "right decision",
        "is it fair",
    ]) {
        return Ok(Some(opinion(
            &paper,
            topic.as_deref().unwrap_or(&site.fact("/front/topic")),
        )));
    }
    if h(&[
        "who has been reporting",
        "who reported",
        "who wrote",
        "who is reporting",
        "who covers",
        "who's been reporting",
        "who has reported",
        "reporting it",
        "byline",
    ]) {
        let topic = topic.unwrap_or_else(|| site.fact("/front/topic"));
        return Ok(Some(who(&paper, &topic)));
    }
    if let Some(topic) = &topic
        && h(&[
            "when did",
            "start",
            "first story",
            "begin",
            "began",
            "how did it",
        ])
    {
        return Ok(Some(first_step(records, &paper, topic).await?));
    }
    if let Some(a) = archive_answer(&paper, &t, sub) {
        return Ok(Some(a));
    }
    if h(&["correction", "corrected", "mistake", "got wrong"]) {
        return Ok(Some(corrections_answer(&paper)));
    }
    if let Some(topic) = &topic {
        if h(&[
            "stand",
            "latest on",
            "update on",
            "happening",
            "where are we",
            "status of",
            "news on",
            "progress",
            "what's new",
            "what is new",
        ]) {
            return Ok(Some(standing(records, &paper, topic).await?));
        }
        if h(&["show me", "stories about", "stories on", "list", "all the"]) {
            return Ok(Some(topic_list(&paper, topic, sub)));
        }
    }
    Ok(None)
}

fn catch_up(paper: &Paper, since: i64, label: &str, follows: &[SectionInfo], sub: bool) -> Answer {
    let mut a = Answer::new("newsroom");
    a.progress
        .push(format!("Finding stories published since {label}"));
    let names: Vec<String> = follows.iter().map(|s| s.name.clone()).collect();
    if !names.is_empty() {
        a.progress
            .push(format!("Keeping to {}", names.join(" and ")));
    }
    let window: Vec<&Story> = paper
        .stories
        .iter()
        .filter(|s| s.published_at >= since)
        .collect();
    let followed = |s: &Story| follows.is_empty() || follows.iter().any(|f| f.id == s.section);
    let mine: Vec<&Story> = window.iter().copied().filter(|s| followed(s)).collect();
    let others: Vec<&Story> = window.iter().copied().filter(|s| !followed(s)).collect();
    let in_topics = if names.is_empty() {
        String::new()
    } else {
        format!(" in {}", names.join(" and "))
    };
    if mine.is_empty() {
        a.say(format!("Nothing published since {label}{in_topics}."));
        if let Some(latest) = paper.stories.iter().find(|s| followed(s)) {
            a.say(format!(
                "The latest{in_topics} is **{}**, from {} [1].",
                latest.title,
                paper.day(latest.published_at)
            ));
            a.source(paper.source(latest, latest.locked() && !sub));
        }
        a.meta = "From the published stories' dates \u{b7} nothing was guessed".into();
        return a;
    }
    a.say(format!(
        "Since {label}, Tana Valley Times published {}{in_topics}{}",
        plural(mine.len(), "story", "stories"),
        if mine.len() > 5 {
            ". The five most recent:"
        } else {
            ":"
        }
    ));
    let mut locked_any = false;
    let mut listed = HashSet::new();
    for (i, s) in mine.iter().take(5).enumerate() {
        let n = a.sources.len() + 1;
        let locked = s.locked() && !sub;
        locked_any |= locked;
        if locked {
            a.say(format!(
                "{}. **{}** (subscriber story: headline and link only) [{n}]",
                i + 1,
                s.title
            ));
        } else {
            a.say(format!(
                "{}. **{}**. {} [{n}]",
                i + 1,
                s.title,
                s.standfirst
            ));
        }
        a.source(paper.source(s, locked));
        listed.insert(s.slug.clone());
        if let Some(c) = paper.correction(&s.slug).filter(|c| c.at >= since) {
            let m = a.sources.len() + 1;
            let when = time::fmt_day_clock(c.at, paper.off);
            a.say(format!(
                "**Correction:** {}. The story was corrected on {when} [{m}].",
                c.what
            ));
            a.source(Source::new(
                &format!("Correction \u{b7} {when}"),
                &format!("{}#correction", paper.href(s)),
            ));
        }
    }
    for c in paper
        .corrections
        .iter()
        .filter(|c| c.at >= since && !listed.contains(&c.story_slug))
        .take(2)
    {
        if let Some(s) = paper
            .stories
            .iter()
            .find(|s| s.slug == c.story_slug && followed(s))
        {
            let m = a.sources.len() + 1;
            let when = time::fmt_day_clock(c.at, paper.off);
            a.say(format!(
                "**Correction:** {} in **{}**, corrected on {when} [{m}].",
                c.what, s.title
            ));
            a.source(Source::new(
                &format!("Correction \u{b7} {when}"),
                &format!("{}#correction", paper.href(s)),
            ));
        }
    }
    if let Some(o) = others.first() {
        let n = a.sources.len() + 1;
        let locked = o.locked() && !sub;
        locked_any |= locked;
        let kind = if locked { "a subscriber story, " } else { "" };
        a.say(format!(
            "Outside your topics, {} more, including {kind}**{}** [{n}].",
            plural(others.len(), "story", "stories"),
            o.title
        ));
        a.source(paper.source(o, locked));
    }
    if let Some(topic) = mine.iter().find_map(|s| s.topic.clone()) {
        a.action(Action::link(
            "Follow the story so far",
            &format!("/stories/{topic}"),
            false,
        ));
    }
    a.action(Action::link("Get this every Friday", "/digest", false));
    a.meta = format!(
        "From {} \u{b7} every claim dated and bylined{}",
        plural(
            mine.len().min(5) + others.len().min(1),
            "published story",
            "published stories"
        ),
        if locked_any {
            " \u{b7} subscriber story: headline and link only"
        } else {
            ""
        }
    );
    a
}

fn front_page(paper: &Paper, sub: bool) -> Answer {
    let mut a = Answer::instant("newsroom");
    a.meta = format!("From today's front page, {}", paper.clock());
    let lead_slug = paper.site.fact("/front/lead");
    let lead = paper
        .stories
        .iter()
        .find(|s| s.slug == lead_slug)
        .or(paper.stories.first());
    let Some(lead) = lead else {
        a.say(
            "Nothing is published yet. Stories appear here the moment the editor publishes them."
                .into(),
        );
        return a;
    };
    let tag = |s: &Story| {
        if s.locked() && !sub {
            " (subscriber story)"
        } else {
            ""
        }
    };
    a.say(format!(
        "Today's front page leads with **{}**{} [1].",
        lead.title,
        tag(lead)
    ));
    a.source(paper.source(lead, lead.locked() && !sub));
    let next: Vec<&Story> = paper
        .stories
        .iter()
        .filter(|s| s.slug != lead.slug)
        .take(2)
        .collect();
    if next.len() == 2 {
        a.say(format!(
            "Also on it: **{}**{} [2] and **{}**{} [3].",
            next[0].title,
            tag(next[0]),
            next[1].title,
            tag(next[1])
        ));
        for s in next {
            a.source(paper.source(s, s.locked() && !sub));
        }
    }
    a
}

fn opinion(paper: &Paper, topic: &str) -> Answer {
    let mut a = Answer::instant("newsroom");
    let in_topic: Vec<&Story> = paper
        .stories
        .iter()
        .filter(|s| s.topic.as_deref() == Some(topic))
        .collect();
    let editorial = in_topic.iter().find(|s| {
        s.kicker.as_deref().is_some_and(|k| {
            k.eq_ignore_ascii_case("editorial") || k.eq_ignore_ascii_case("opinion")
        })
    });
    match editorial {
        Some(e) => {
            a.say(format!(
                "I don't give opinions of my own. The editorial {} published on it is **{}** [1].",
                paper.site.name, e.title
            ));
            a.source(paper.source(e, false));
        }
        None => {
            a.say(format!(
                "I don't give opinions. {} hasn't published an editorial on it.",
                paper.site.name
            ));
            let latest: Vec<&&Story> = in_topic.iter().take(2).collect();
            match latest.as_slice() {
                [one, two] => {
                    a.say(format!(
                        "Its latest reporting, newest first: **{}** [1] and **{}** [2].",
                        one.title, two.title
                    ));
                    a.source(paper.source(one, false));
                    a.source(paper.source(two, false));
                }
                [one] => {
                    a.say(format!("Its latest reporting: **{}** [1].", one.title));
                    a.source(paper.source(one, false));
                }
                _ => {}
            }
        }
    }
    a.meta = "From the published stories \u{b7} desks don't give opinions".into();
    a
}

fn who(paper: &Paper, topic: &str) -> Answer {
    let mut a = Answer::instant("newsroom");
    a.meta = "From the bylines on the published stories".into();
    let in_topic: Vec<&Story> = paper
        .stories
        .iter()
        .filter(|s| s.topic.as_deref() == Some(topic))
        .collect();
    let mut by: Vec<(String, Vec<&Story>)> = Vec::new();
    for s in &in_topic {
        match by.iter_mut().find(|(id, _)| *id == s.author_id) {
            Some((_, list)) => list.push(s),
            None => by.push((s.author_id.clone(), vec![s])),
        }
    }
    by.sort_by_key(|x| std::cmp::Reverse(x.1.len()));
    let title = paper.topic_phrase(topic);
    if by.is_empty() {
        a.say(format!("No stories on {title} are published yet."));
        return a;
    }
    for (i, (_, list)) in by.iter().enumerate() {
        let latest = list[0];
        let n = a.sources.len() + 1;
        let name = paper.author(latest);
        if i == 0 {
            a.say(format!(
                "**{name}** has written {} of the {} stories on {title}, most recently **{}** [{n}].",
                list.len(),
                in_topic.len(),
                latest.title
            ));
        } else {
            a.say(format!(
                "**{name}** wrote {}, including **{}** [{n}].",
                plural(list.len(), "story", "stories"),
                latest.title
            ));
        }
        a.source(paper.source(latest, false));
    }
    let n = a.sources.len() + 1;
    a.say(format!("Every step is on the story so far [{n}]."));
    a.source(Source::new("Story so far", &format!("/stories/{topic}")));
    a
}

async fn first_step(records: &Records, paper: &Paper, topic: &str) -> anyhow::Result<Answer> {
    let mut a = Answer::instant("archive");
    a.meta = "From the archive's dates".into();
    let timeline = records.timeline(&paper.site.site, topic).await?;
    let published: Vec<_> = timeline
        .iter()
        .filter(|t| t.at <= paper.now && t.kind == "step")
        .collect();
    let first = published.iter().find_map(|t| {
        paper
            .stories
            .iter()
            .find(|s| Some(&s.slug) == t.story_slug.as_ref())
    });
    let Some(first) = first else {
        a.say("The story so far has no published steps yet.".into());
        return Ok(a);
    };
    a.say(format!(
        "The story so far on {} starts on **{}** with **{}**, by {} [1].",
        paper.topic_phrase(topic),
        time::fmt_date_long(first.published_at, paper.off),
        first.title,
        paper.author(first)
    ));
    a.source(paper.source(first, false));
    if let Some(last) = published.last() {
        a.say(format!(
            "It has {} so far, the latest on {} [2].",
            plural(published.len(), "step", "steps"),
            paper.day(last.at)
        ));
        a.source(Source::new("Story so far", &format!("/stories/{topic}")));
    }
    Ok(a)
}

async fn standing(records: &Records, paper: &Paper, topic: &str) -> anyhow::Result<Answer> {
    let mut a = Answer::instant("newsroom");
    let timeline = records.timeline(&paper.site.site, topic).await?;
    let story = |slug: &Option<String>| {
        slug.as_ref()
            .and_then(|s| paper.stories.iter().find(|x| &x.slug == s))
    };
    let last = timeline
        .iter()
        .rev()
        .find(|t| t.kind == "step" && t.at <= paper.now);
    if let Some(step) = last {
        a.say(format!(
            "The latest step: **{}**, {} [1].",
            step.line,
            paper.day(step.at)
        ));
        match story(&step.story_slug) {
            Some(s) => a.source(paper.source(s, false)),
            None => a.source(Source::new("Story so far", &format!("/stories/{topic}"))),
        }
    }
    for c in timeline
        .iter()
        .filter(|t| t.kind == "correction" && t.at <= paper.now)
    {
        let n = a.sources.len() + 1;
        let when = time::fmt_day_clock(c.at, paper.off);
        a.say(format!("**Correction:** {}, {when} [{n}].", c.line));
        let href = story(&c.story_slug).map_or_else(
            || format!("/stories/{topic}"),
            |s| format!("{}#correction", paper.href(s)),
        );
        a.source(Source::new(&format!("Correction \u{b7} {when}"), &href));
    }
    if let Some(next) = timeline
        .iter()
        .find(|t| t.kind == "scheduled" && t.at > paper.now)
    {
        let n = a.sources.len() + 1;
        a.say(format!(
            "Next: {}, scheduled for {} [{n}].",
            next.line,
            time::local(next.at, paper.off).format("%B")
        ));
        a.source(Source::new("Story so far", &format!("/stories/{topic}")));
    }
    if a.phrases.is_empty() {
        a.say("The story so far has no published steps yet.".into());
    }
    a.action(Action::link(
        "Follow the story so far",
        &format!("/stories/{topic}"),
        true,
    ));
    a.meta = format!("From the story so far, {}", paper.clock());
    Ok(a)
}

fn topic_list(paper: &Paper, topic: &str, sub: bool) -> Answer {
    let mut a = Answer::instant("archive");
    let list: Vec<&Story> = paper
        .stories
        .iter()
        .filter(|s| s.topic.as_deref() == Some(topic))
        .collect();
    a.say(format!(
        "{} on {}, newest first:",
        plural(list.len(), "story", "stories"),
        paper.topic_phrase(topic)
    ));
    for s in list.iter().take(5) {
        let n = a.sources.len() + 1;
        a.say(format!(
            "**{}**, {} [{n}].",
            s.title,
            paper.day(s.published_at)
        ));
        a.source(paper.source(s, s.locked() && !sub));
    }
    a.action(Action::link(
        "Open the story so far",
        &format!("/stories/{topic}"),
        true,
    ));
    a.meta = format!("From the archive, {}", paper.clock());
    a
}

fn corrections_answer(paper: &Paper) -> Answer {
    let mut a = Answer::instant("newsroom");
    a.meta = "From the corrections page".into();
    if paper.corrections.is_empty() {
        a.say("No corrections published yet.".into());
        return a;
    }
    a.say(format!(
        "{} has published {}. The latest:",
        paper.site.name,
        plural(paper.corrections.len(), "correction", "corrections")
    ));
    for c in paper.corrections.iter().take(3) {
        if let Some(s) = paper.stories.iter().find(|s| s.slug == c.story_slug) {
            let n = a.sources.len() + 1;
            let when = time::fmt_day_clock(c.at, paper.off);
            a.say(format!(
                "**{}** in **{}**, corrected {when} [{n}].",
                c.what, s.title
            ));
            a.source(Source::new(
                &format!("Correction \u{b7} {when}"),
                &format!("{}#correction", paper.href(s)),
            ));
        }
    }
    a.action(Action::link("See every correction", "/corrections", false));
    a
}

/// "last month", "this month", "March 2025", "in 2024": the Archive desk's dated answers.
fn archive_answer(paper: &Paper, t: &str, sub: bool) -> Option<Answer> {
    let today = time::local(paper.now, paper.off).date_naive();
    let words: Vec<&str> = t
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let year_word = words
        .iter()
        .filter_map(|w| w.parse::<i32>().ok())
        .find(|y| (2000..=2100).contains(y));
    let month_word = words
        .iter()
        .filter(|w| w.len() >= 3 && MONTHS.iter().any(|m| m.starts_with(**w) || **w == *m))
        .find_map(|w| month_index(w));
    let (y, m) = if t.contains("last month") {
        if today.month() == 1 {
            (today.year() - 1, Some(12))
        } else {
            (today.year(), Some(today.month() - 1))
        }
    } else if t.contains("this month") {
        (today.year(), Some(today.month()))
    } else if let Some(m) = month_word.filter(|_| year_word.is_some() || t.contains(" in ")) {
        let y = year_word.unwrap_or_else(|| {
            if m > today.month() {
                today.year() - 1
            } else {
                today.year()
            }
        });
        (y, Some(m))
    } else if let Some(y) = year_word {
        (y, None)
    } else {
        return None;
    };
    let mut a = Answer::instant("archive");
    let in_range = |s: &&Story| {
        let l = time::local(s.published_at, paper.off);
        l.year() == y && m.is_none_or(|m| l.month() == m)
    };
    let list: Vec<&Story> = paper.stories.iter().filter(in_range).collect();
    let (name, anchor) = match m {
        Some(m) => {
            let n = MONTHS[(m - 1) as usize];
            (
                format!("{}{} {y}", n[..1].to_uppercase(), &n[1..]),
                format!("/archive#m-{y}-{m:02}"),
            )
        }
        None => (y.to_string(), format!("/archive#y-{y}")),
    };
    if list.is_empty() {
        let first = paper
            .stories
            .last()
            .map(|s| {
                time::local(s.published_at, paper.off)
                    .format("%B %Y")
                    .to_string()
            })
            .unwrap_or_default();
        a.say(format!(
            "No stories online from {name}. The archive starts in {first} [1]."
        ));
        a.source(Source::new("Archive", "/archive"));
        return Some(a);
    }
    let issues: HashSet<i64> = list.iter().map(|s| paper.issue(s.published_at)).collect();
    a.say(format!(
        "{} in {name}, across {} [1]:",
        plural(list.len(), "story", "stories"),
        plural(issues.len(), "issue", "issues")
    ));
    a.source(Source::new(&format!("Archive \u{b7} {name}"), &anchor));
    for s in list.iter().take(5) {
        let n = a.sources.len() + 1;
        a.say(format!(
            "**{}** ({}, {}) [{n}].",
            s.title,
            paper.section_name(&s.section),
            paper.day(s.published_at)
        ));
        a.source(paper.source(s, s.locked() && !sub));
    }
    if list.len() > 5 {
        a.say(format!("{} more in the archive [1].", list.len() - 5));
    }
    a.action(Action::link(
        &format!("Open {name} in the archive"),
        &anchor,
        true,
    ));
    a.meta = "From the archive's dates".into();
    Some(a)
}

fn digest_answer(paper: &Paper, t: &str) -> Answer {
    let mut a = Answer::instant("subscriptions");
    let d = digest_view(&paper.site, &[]);
    let name = d
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("digest")
        .to_owned();
    let row = |k: &str| {
        d.get("rows")
            .and_then(Value::as_array)
            .and_then(|r| {
                r.iter()
                    .find(|x| x.get("k").and_then(Value::as_str) == Some(k))
            })
            .and_then(|x| x.get("text").and_then(Value::as_str))
            .unwrap_or("")
            .to_owned()
    };
    if has(t, &["stop", "unsubscribe", "cancel"]) {
        a.say(format!("{} [1].", row("Stop")));
        a.say(format!(
            "Every {name} email ends with that stop link; you don't need to sign in."
        ));
        if has(t, &["subscri"]) && !has(t, &["digest", "friday five", "newsletter"]) {
            a.say("A subscription on this sample doesn't renew by itself, so there's nothing to cancel.".into());
        }
    } else {
        a.say(format!("The {name}: {} [1].", row("You get")));
        a.say(format!(
            "{} [1]. Nothing is sent until you confirm by email.",
            row("We keep")
        ));
        a.action(Action::link("Set up the Friday five", "/digest", true));
    }
    a.source(Source::new(
        "Friday five \u{b7} the contract",
        "/digest#contract",
    ));
    a.meta = "From the Friday five contract".into();
    a
}

async fn subscription_answer(
    records: &Records,
    ctx: &Ctx,
    paper: &Paper,
    t: &str,
    sub: bool,
) -> anyhow::Result<Answer> {
    let mut a = Answer::instant("subscriptions");
    let site = &paper.site;
    let offer = site
        .facts
        .get("subscription")
        .cloned()
        .unwrap_or(Value::Null);
    let s = |k: &str| {
        offer
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    if has(
        t,
        &["my ", "am i", "active", "still", "until", "expire", "renew"],
    ) {
        a.meta = format!("From your account on this site, {}", paper.clock());
        match &ctx.visitor {
            None => {
                a.meta = "From the subscribe page".into();
                a.say(
                    "Sign in and I can check. I never ask for a password or a code in this chat."
                        .into(),
                );
                a.action(Action::link(
                    "Sign in",
                    "/account/sign-in?why=save&next=/subscribe",
                    true,
                ));
            }
            Some(v) => {
                let row = subscription_of(records, ctx).await?;
                match row {
                    Some(r) if sub => {
                        let until = r.until_at.map_or_else(
                            || " with no end date".to_owned(),
                            |u| format!(" until **{}**", time::fmt_date_long(u, paper.off)),
                        );
                        a.say(format!(
                            "Yes. Your {} subscription is active{until} [1].",
                            r.plan
                        ));
                        if r.until_at.is_some() {
                            a.say(s("renewal"));
                        }
                    }
                    _ => {
                        a.say(format!(
                            "You're signed in as {}, without an active subscription [1].",
                            v.name
                        ));
                        a.action(Action::link("See the offer", "/subscribe", true));
                    }
                }
                a.source(Source::new(
                    "Subscribe \u{b7} your subscription",
                    "/subscribe#mine",
                ));
            }
        }
        return Ok(a);
    }
    let p = Price::of(site);
    a.say(format!("{} [1].", s("price_words")));
    a.say(format!(
        "On this sample site you pay **{}** to {} ({} at {}) with PayPal's sandbox, for one {}; no real money moves [1].",
        p.usd,
        site.name,
        p.local,
        p.rate,
        s("period")
    ));
    let locked = paper.stories.iter().filter(|s| s.locked()).count();
    a.say(format!(
        "It opens all {} in full.",
        plural(locked, "subscriber story", "subscriber stories")
    ));
    a.source(Source::new(
        "Subscribe \u{b7} the offer",
        "/subscribe#offer",
    ));
    a.action(Action::link("See the offer", "/subscribe", true));
    a.meta = "From the subscribe page".into();
    Ok(a)
}

// ----------------------------------------------------------------------------- retrieval

/// What Ask retrieves from on Tana: every free story's paragraphs with their anchors; for a
/// subscriber story, only the headline (unless the visitor subscribes); the
/// corrections, the house rules, the digest contract and the offer.
pub async fn sections(app: &App, ctx: &Ctx) -> anyhow::Result<Vec<Section>> {
    let records = app.records().await?;
    let site = &ctx.site;
    let paper = Paper::load(records, site).await?;
    let sub = subscriber(records, ctx, paper.now).await?;
    let mut out = Vec::new();
    for s in &paper.stories {
        let href = paper.href(s);
        let label = paper.label(s);
        if s.locked() && !sub {
            out.push(Section::new(
                &href,
                &format!("{label} \u{b7} subscriber"),
                &format!(
                    "{}. (Subscriber story: only the headline and link are shown to non-subscribers.)",
                    s.title
                ),
            ));
            continue;
        }
        out.push(Section::new(
            &href,
            &label,
            &format!("{}. {}", s.title, s.standfirst),
        ));
        for (i, p) in s.paragraphs().iter().enumerate() {
            out.push(Section::new(
                &format!("{href}#p{}", i + 1),
                &label,
                &format!("{}: {p}", s.title),
            ));
        }
    }
    for c in &paper.corrections {
        if let Some(s) = paper.stories.iter().find(|s| s.slug == c.story_slug) {
            out.push(Section::new(
                &format!("{}#correction", paper.href(s)),
                &format!("Correction \u{b7} {}", time::fmt_day_clock(c.at, paper.off)),
                &format!("Correction to \"{}\": {}", s.title, c.note),
            ));
        }
    }
    out.push(Section::new(
        "/#rules",
        "House rules for desks",
        &house_rules(site).join(" "),
    ));
    let d = digest_view(site, &[]);
    if let Some(rows) = d.get("rows").and_then(Value::as_array) {
        let text: Vec<String> = rows
            .iter()
            .map(|r| {
                format!(
                    "{}: {}.",
                    r.get("k").and_then(Value::as_str).unwrap_or(""),
                    r.get("text").and_then(Value::as_str).unwrap_or("")
                )
            })
            .collect();
        out.push(Section::new(
            "/digest#contract",
            "Friday five \u{b7} the contract",
            &text.join(" "),
        ));
    }
    let offer = site
        .facts
        .get("subscription")
        .cloned()
        .unwrap_or(Value::Null);
    out.push(Section::new(
        "/subscribe#offer",
        "Subscribe \u{b7} the offer",
        &format!(
            "{}. {}. {}",
            offer
                .get("price_words")
                .and_then(Value::as_str)
                .unwrap_or(""),
            offer.get("note").and_then(Value::as_str).unwrap_or(""),
            offer.get("renewal").and_then(Value::as_str).unwrap_or("")
        ),
    ));
    for s in &paper.sections {
        out.push(Section::new(&format!("/{}", s.id), &s.name, &s.about));
    }
    Ok(out)
}

/// Fridays from 07:00: each confirmed reader of a news sample gets that week's Friday five,
/// through W9's outbox, once per week, with their own stop link. Called hourly.
///
/// # Errors
///
/// On a database error.
pub async fn friday_digest(app: &App) -> anyhow::Result<()> {
    use chrono::Datelike as _;
    let records = app.records().await?;
    let now = time::now();
    for site in records.sites().await? {
        if site.archetype != "news" {
            continue;
        }
        let off = site.utc_offset_min;
        let local = time::local(now, off);
        if local.weekday() != chrono::Weekday::Fri {
            continue;
        }
        let Ok(edition_start) = time::at_local(local.date_naive(), "07:00", off) else {
            continue;
        };
        if now < edition_start {
            continue;
        }
        let due = records.digest_due(&site.site, edition_start).await?;
        if due.is_empty() {
            continue;
        }
        let week: Vec<Story> = records
            .stories_since(&site.site, now - 7 * 86_400)
            .await?
            .into_iter()
            .filter(|s| s.status == "published")
            .collect();
        let sections = sections_of(&site);
        let authors = records.authors(&site.site).await?;
        let (host, origin) = app.site_origin(&site);
        let date = time::fmt_day(now, off);
        for (sub, stop_token) in due {
            let topics: Vec<String> = serde_json::from_str(&sub.topics).unwrap_or_default();
            // The editor's rule: the week's stories in the reader's topics first, newest first,
            // then the rest, five in all.
            // Topics are kept as section names ("Transport"), as the sign-up form writes them.
            let wanted = |s: &Story| {
                let name = sections
                    .iter()
                    .find(|x| x.id == s.section)
                    .map_or(s.section.as_str(), |x| x.name.as_str());
                topics.iter().any(|t| t == name || *t == s.section)
            };
            let mut picks: Vec<&Story> = week.iter().filter(|s| wanted(s)).collect();
            picks.extend(week.iter().filter(|s| !wanted(s)));
            picks.truncate(5);
            let mut stories = Vec::new();
            for s in picks {
                let correction = records
                    .corrections(&site.site, Some(&s.slug))
                    .await?
                    .into_iter()
                    .next();
                let l = time::local(s.published_at, off);
                stories.push(agentrix_messages::orders::Story {
                    headline: s.title.clone(),
                    standfirst: if s.locked() {
                        String::new()
                    } else {
                        s.standfirst.clone()
                    },
                    url: format!("{origin}/{}/{:02}/{}", l.year(), l.month(), s.slug),
                    section: sections
                        .iter()
                        .find(|x| x.id == s.section)
                        .map_or_else(|| s.section.clone(), |x| x.name.clone()),
                    byline: authors
                        .iter()
                        .find(|a| a.author_id == s.author_id)
                        .map(|a| a.name.clone())
                        .unwrap_or_default(),
                    published: Some(time::fmt_day(s.published_at, off)),
                    corrected: correction.is_some(),
                    correction_note: correction.map(|c| c.note),
                });
            }
            let topic_words: Vec<String> = topics
                .iter()
                .map(|t| {
                    sections
                        .iter()
                        .find(|x| &x.id == t)
                        .map_or_else(|| t.clone(), |x| x.name.clone())
                })
                .collect();
            let topic_words = if topic_words.is_empty() {
                vec!["Every section".to_owned()]
            } else {
                topic_words
            };
            let out = agentrix_messages::orders::digest_weekly(
                &crate::notify::site_facts(&site, &host),
                &sub.email,
                &topic_words.join(", "),
                &date,
                &stories,
                &format!("{origin}/digest/stop?token={stop_token}"),
            );
            match app.messages.send(out).await {
                Ok(_) => {
                    records
                        .mark_digest_sent(&site.site, &sub.sub_id, now)
                        .await?
                }
                Err(e) => tracing::warn!(error = %e, site = site.site, "Friday five not recorded"),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sun 8 Nov 2026, 07:30 EAT.
    const NOW: i64 = 1_794_112_200;

    #[test]
    fn since_reads_the_dates_catch_me_up_sends() {
        let day = |t: i64| time::fmt_day(t, 180);
        assert_eq!(
            day(since_in("catch me up since tue 3 nov 2026", NOW, 180).unwrap()),
            "Tue 3 Nov"
        );
        assert_eq!(
            day(since_in("catch me up since 3 nov", NOW, 180).unwrap()),
            "Tue 3 Nov"
        );
        assert_eq!(
            day(since_in("catch me up since tuesday", NOW, 180).unwrap()),
            "Tue 3 Nov"
        );
        assert_eq!(
            day(since_in("catch me up since yesterday", NOW, 180).unwrap()),
            "Sat 7 Nov"
        );
        assert_eq!(
            day(since_in("since 20 dec", NOW, 180).unwrap()),
            "Sat 20 Dec"
        );
        assert!(since_in("catch me up", NOW, 180).is_none());
    }
}
