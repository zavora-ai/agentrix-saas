//! Rendering a catalogue message into what each channel carries.
//!
//! Templates are minijinja in semi-strict mode: printing a missing value is an error, so a
//! message can never go out with a blank where a fact should be. Every printed value is wrapped
//! in sentinels and cleaned of line breaks before the body is parsed (see [`crate::markup`]).

use std::sync::OnceLock;

use minijinja::value::Value;
use minijinja::{Environment, Error, ErrorKind, UndefinedBehavior};
use serde::Serialize;
use serde_json::Value as Json;

use crate::catalogue::{self, Audience, Channel, Kind, MessageDef};
use crate::markup::{self, Block, CLOSE, OPEN, Palette};

/// What a secret value looks like in every stored copy of a message.
pub const REDACTED: &str = "••••••";

/// An email, ready for a transport.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Email {
    /// The name in the From line ("Kilele Bakery (sample)", "Agentrix").
    pub from_name: String,
    /// The subject line.
    pub subject: String,
    /// The hidden preview line most inboxes show after the subject.
    pub preheader: String,
    /// The plain-text version.
    pub text: String,
    /// The HTML version.
    pub html: String,
}

/// A web push notification: the doorbell, never the letter.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Push {
    /// The title ("Kilele · Agentrix").
    pub title: String,
    /// One line that says something is waiting, without details.
    pub body: String,
    /// Where tapping it opens (inside the console, after unlocking).
    pub url: String,
}

/// A chat message (WhatsApp first).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Chat {
    /// The message text.
    pub text: String,
    /// Quick replies the person can tap ("Decline"). At most three.
    pub replies: Vec<String>,
    /// One link button, when the answer needs the app.
    pub link: Option<(String, String)>,
}

/// A message rendered for every channel it supports.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Rendered {
    /// The catalogue id.
    pub id: &'static str,
    /// Present when the message has an email version.
    pub email: Option<Email>,
    /// Present when the message has a push version.
    pub push: Option<Push>,
    /// Present when the message has a chat version.
    pub chat: Option<Chat>,
}

impl Rendered {
    /// One line for lists: the subject, else the push line, else the chat's first line.
    pub fn summary(&self) -> String {
        if let Some(e) = &self.email {
            return e.subject.clone();
        }
        if let Some(p) = &self.push {
            return p.body.clone();
        }
        if let Some(c) = &self.chat {
            return c.text.lines().next().unwrap_or_default().to_string();
        }
        String::new()
    }
}

/// Why a message could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// No message has this id.
    #[error("no message called `{0}` in the catalogue")]
    Unknown(String),
    /// The template failed, usually a missing value.
    #[error("`{id}`: {source}")]
    Template {
        /// The message id.
        id: String,
        /// What minijinja said.
        #[source]
        source: Error,
    },
    /// The rendered body broke the line format.
    #[error("`{id}` {block}: {source}")]
    Markup {
        /// The message id.
        id: String,
        /// Which block.
        block: &'static str,
        /// What was wrong.
        #[source]
        source: markup::MarkupError,
    },
    /// A rule the catalogue promises was broken (for example an email that approves).
    #[error("`{id}`: {problem}")]
    Rule {
        /// The message id.
        id: String,
        /// What was wrong.
        problem: String,
    },
}

fn clean_value(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .filter(|c| !c.is_control() && *c != OPEN && *c != CLOSE)
        .collect()
}

/// `{{ 4500|money("KES") }}` → `KSh 4,500`; `{{ 19.77|money("USD") }}` → `USD 19.77`.
fn money(value: Value, currency: Option<String>) -> Result<String, Error> {
    let amount: f64 = if let Some(n) = value.as_i64() {
        n as f64
    } else if let Ok(f) = f64::try_from(value.clone()) {
        f
    } else if let Some(s) = value.as_str() {
        // Already written with its currency ("KSh 2,550", "USD 19.77"): print as given.
        if s.chars().any(|c| c.is_alphabetic()) {
            return Ok(s.trim().to_string());
        }
        s.trim().parse::<f64>().map_err(|_| {
            Error::new(
                ErrorKind::InvalidOperation,
                format!("money: `{s}` isn't a number"),
            )
        })?
    } else {
        return Err(Error::new(
            ErrorKind::InvalidOperation,
            "money: needs a number",
        ));
    };
    let currency = currency
        .unwrap_or_else(|| "KES".to_string())
        .to_ascii_uppercase();
    let negative = amount < 0.0;
    let cents = (amount.abs() * 100.0).round() as u64;
    let whole = cents / 100;
    let frac = cents % 100;
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let number = if frac == 0 && currency == "KES" {
        grouped
    } else {
        format!("{grouped}.{frac:02}")
    };
    let symbol = match currency.as_str() {
        "KES" => "KSh".to_string(),
        other => other.to_string(),
    };
    Ok(format!(
        "{}{symbol} {number}",
        if negative { "-" } else { "" }
    ))
}

/// `{{ n|plural("decision", "decisions") }}` → `1 decision`, `2 decisions`.
fn plural(n: i64, one: String, many: String) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn message_env() -> &'static Environment<'static> {
    static ENV: OnceLock<Environment<'static>> = OnceLock::new();
    ENV.get_or_init(|| {
        let mut env = Environment::new();
        env.set_undefined_behavior(UndefinedBehavior::SemiStrict);
        env.set_trim_blocks(true);
        env.set_lstrip_blocks(true);
        env.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
        env.set_formatter(|out, _state, value| {
            if value.is_undefined() {
                return Err(Error::new(ErrorKind::UndefinedError, "a value is missing"));
            }
            if value.is_none() {
                return Err(Error::new(
                    ErrorKind::UndefinedError,
                    "a value is null; test it with `if` before printing",
                ));
            }
            let s = clean_value(&value.to_string());
            write!(out, "{OPEN}{s}{CLOSE}").map_err(Error::from)
        });
        env.add_filter("money", money);
        env.add_filter("plural", plural);
        env.add_filter("code", |s: String| markup::spaced_code(&s));
        for def in catalogue::all() {
            env.add_template(def.id, def.source)
                .unwrap_or_else(|e| panic!("template {} doesn't parse: {e}", def.id));
        }
        for (name, source) in LAYOUT_PARTS {
            env.add_template(name, source)
                .unwrap_or_else(|e| panic!("layout part {name} doesn't parse: {e}"));
        }
        env
    })
}

fn layout_env() -> &'static Environment<'static> {
    static ENV: OnceLock<Environment<'static>> = OnceLock::new();
    ENV.get_or_init(|| {
        let mut env = Environment::new();
        env.set_undefined_behavior(UndefinedBehavior::Strict);
        env.set_trim_blocks(true);
        env.set_lstrip_blocks(true);
        env.add_template(
            "email.html",
            include_str!("../templates/_layout/email.html"),
        )
        .expect("email layout parses");
        env
    })
}

/// Shared parts rendered around every message, written in the same line format.
const LAYOUT_PARTS: [(&str, &str); 4] = [
    (
        "_layout/footer_customer",
        include_str!("../templates/_layout/footer_customer.txt"),
    ),
    (
        "_layout/footer_owner",
        include_str!("../templates/_layout/footer_owner.txt"),
    ),
    (
        "_layout/footer_subscriber",
        include_str!("../templates/_layout/footer_subscriber.txt"),
    ),
    (
        "_layout/footer_prospect",
        include_str!("../templates/_layout/footer_prospect.txt"),
    ),
];

/// The Sample band, in the exact words every surface uses (W7's `sample.band`).
fn sample_band(ctx: &Json) -> Vec<Block> {
    let name = site_name(ctx).unwrap_or_else(|| "This site".to_string());
    let words = agentrix_ui::word("en", "sample.band").unwrap_or(
        "{site} isn't a real business. It's a sample Agentrix runs to show the platform.",
    );
    vec![Block::Para(format!(
        "Sample: {}",
        words.replace("{site}", &name)
    ))]
}

fn template_err(id: &str, source: Error) -> RenderError {
    RenderError::Template {
        id: id.to_string(),
        source,
    }
}

fn block(
    state: &mut minijinja::State<'_, '_>,
    id: &str,
    name: &str,
) -> Result<Option<String>, RenderError> {
    match state.render_block(name) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == ErrorKind::UnknownBlock => Ok(None),
        Err(e) => Err(template_err(id, e)),
    }
}

fn one_line(s: &str) -> String {
    markup::plain(s)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Replaces the secret values a message declares with [`REDACTED`], for every stored copy.
pub fn redact(def: &MessageDef, ctx: &Json) -> Json {
    let mut ctx = ctx.clone();
    for path in def.secret {
        let mut cur = &mut ctx;
        let parts: Vec<&str> = path.split('.').collect();
        for (i, part) in parts.iter().enumerate() {
            let Some(obj) = cur.as_object_mut() else {
                break;
            };
            if i + 1 == parts.len() {
                if obj.contains_key(*part) {
                    obj.insert((*part).to_string(), Json::String(REDACTED.to_string()));
                }
                break;
            }
            match obj.get_mut(*part) {
                Some(next) => cur = next,
                None => break,
            }
        }
    }
    ctx
}

fn palette_for(def: &MessageDef, ctx: &Json) -> Palette {
    let mut p = Palette::default();
    if matches!(def.audience, Audience::Customer | Audience::Subscriber)
        && let Some(accent) = ctx.pointer("/site/accent").and_then(Json::as_str)
        && accent.len() == 7
        && accent.starts_with('#')
        && accent[1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        p.accent = accent.to_string();
    }
    p
}

fn site_name(ctx: &Json) -> Option<String> {
    ctx.pointer("/site/name")
        .and_then(Json::as_str)
        .map(clean_value)
}

fn is_sample(ctx: &Json) -> bool {
    ctx.pointer("/site/sample")
        .and_then(Json::as_bool)
        .unwrap_or(false)
}

/// Whether `url` is on `host` itself.
fn on_host(url: &str, host: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.eq_ignore_ascii_case(host)))
        .unwrap_or(false)
}

/// The From name: the site for its customers, Agentrix for owners.
fn from_name(def: &MessageDef, ctx: &Json) -> String {
    match def.audience {
        Audience::Customer | Audience::Subscriber => match site_name(ctx) {
            Some(name) if is_sample(ctx) => format!("{name} (sample)"),
            Some(name) => name,
            None => "Agentrix".to_string(),
        },
        Audience::Owner | Audience::Teammate | Audience::Prospect => "Agentrix".to_string(),
    }
}

/// The push title, as the lock screen in `concept/console/phone.html` draws it.
fn push_title(ctx: &Json) -> String {
    match site_name(ctx) {
        Some(name) => {
            let short = name.split_whitespace().next().unwrap_or(&name).to_string();
            format!("{short} · Agentrix")
        }
        None => "Agentrix".to_string(),
    }
}

#[derive(Serialize)]
struct LayoutCtx<'a> {
    subject: &'a str,
    preheader: &'a str,
    brand: &'a str,
    brand_sub: &'a str,
    accent: &'a str,
    sample_band: Option<String>,
    body: Value,
    footer: Value,
    lang: &'a str,
}

fn render_part(name: &str, ctx: &Json, id: &str) -> Result<Vec<Block>, RenderError> {
    let env = message_env();
    let tmpl = env.get_template(name).map_err(|e| template_err(id, e))?;
    let text = tmpl.render(ctx).map_err(|e| template_err(id, e))?;
    markup::parse(&text).map_err(|source| RenderError::Markup {
        id: id.to_string(),
        block: "layout",
        source,
    })
}

/// Renders a message with its context. Secret values print as given: use [`render_stored`]
/// for any copy that is kept.
pub fn render(id: &str, ctx: &Json) -> Result<Rendered, RenderError> {
    let def = catalogue::get(id).ok_or_else(|| RenderError::Unknown(id.to_string()))?;
    render_def(def, ctx)
}

/// Renders the copy the outbox keeps: secret values replaced by [`REDACTED`].
pub fn render_stored(id: &str, ctx: &Json) -> Result<Rendered, RenderError> {
    let def = catalogue::get(id).ok_or_else(|| RenderError::Unknown(id.to_string()))?;
    render_def(def, &redact(def, ctx))
}

/// Fills in the keys every template may test: `person` defaults to `{"name": null}`.
fn normalize(ctx: &Json) -> Json {
    let mut ctx = ctx.clone();
    if let Some(obj) = ctx.as_object_mut() {
        obj.entry("person")
            .or_insert_with(|| serde_json::json!({ "name": null }));
    }
    ctx
}

pub(crate) fn render_def(def: &MessageDef, ctx: &Json) -> Result<Rendered, RenderError> {
    let ctx = &normalize(ctx);
    let id = def.id;
    let env = message_env();
    let tmpl = env.get_template(id).map_err(|e| template_err(id, e))?;
    let mut captured = tmpl.render_captured(ctx).map_err(|e| template_err(id, e))?;
    let mut get = |name: &str| captured.with_state_mut(|state| block(state, id, name));

    let mut rendered = Rendered {
        id: def.id,
        email: None,
        push: None,
        chat: None,
    };

    if def.channels.contains(&Channel::Email) {
        let subject = get("subject")?.ok_or_else(|| RenderError::Rule {
            id: id.into(),
            problem: "an email needs a `subject` block".into(),
        })?;
        let body = get("body")?.ok_or_else(|| RenderError::Rule {
            id: id.into(),
            problem: "an email needs a `body` block".into(),
        })?;
        let preheader = get("preheader")?.unwrap_or_default();
        let blocks = markup::parse(&body).map_err(|source| RenderError::Markup {
            id: id.into(),
            block: "body",
            source,
        })?;
        let footer_name = match def.audience {
            Audience::Customer => "_layout/footer_customer",
            Audience::Subscriber => "_layout/footer_subscriber",
            Audience::Owner | Audience::Teammate => "_layout/footer_owner",
            Audience::Prospect => "_layout/footer_prospect",
        };
        let footer = render_part(footer_name, ctx, id)?;
        let band = if is_sample(ctx)
            && matches!(def.audience, Audience::Customer | Audience::Subscriber)
        {
            Some(sample_band(ctx))
        } else {
            None
        };

        if matches!(def.audience, Audience::Customer | Audience::Subscriber)
            && let Some(host) = ctx.pointer("/site/host").and_then(Json::as_str)
        {
            for b in blocks.iter().chain(footer.iter()) {
                if let Block::Button { url, .. } | Block::Link { url, .. } = b
                    && !on_host(url, host)
                {
                    return Err(RenderError::Rule {
                        id: id.into(),
                        problem: format!(
                            "a customer email links only to {host} (the footer promises it), but one link goes to {url}"
                        ),
                    });
                }
            }
        }
        let palette = palette_for(def, ctx);
        let subject_line = one_line(&subject);
        let preheader_line = one_line(&preheader);
        let from = from_name(def, ctx);

        let mut text = String::new();
        if let Some(band) = &band {
            text.push_str(&markup::to_text(band));
            text.push_str("\n\n");
        }
        text.push_str(&markup::to_text(&blocks));
        text.push_str("\n\n--\n");
        text.push_str(&markup::to_text(&footer));
        text.push('\n');

        let footer_palette = Palette {
            accent: palette.muted.clone(),
            ..palette.clone()
        };
        let brand_sub = match def.audience {
            Audience::Customer | Audience::Subscriber => ctx
                .pointer("/site/host")
                .and_then(Json::as_str)
                .map(clean_value)
                .unwrap_or_default(),
            Audience::Owner | Audience::Teammate => match site_name(ctx) {
                Some(name) => format!("For {name}"),
                None => String::new(),
            },
            Audience::Prospect => "agentrix.website".to_string(),
        };
        let brand = match def.audience {
            Audience::Customer | Audience::Subscriber => {
                site_name(ctx).unwrap_or_else(|| "Agentrix".into())
            }
            Audience::Owner | Audience::Teammate | Audience::Prospect => "Agentrix".to_string(),
        };
        let layout = LayoutCtx {
            subject: &subject_line,
            preheader: &preheader_line,
            brand: &brand,
            brand_sub: &brand_sub,
            accent: &palette.accent,
            sample_band: band.as_ref().map(|b| markup::plain(&markup::to_text(b))),
            body: Value::from_safe_string(markup::to_html(&blocks, &palette)),
            footer: Value::from_safe_string(markup::to_html(&footer, &footer_palette)),
            lang: "en",
        };
        let html = layout_env()
            .get_template("email.html")
            .and_then(|t| t.render(&layout))
            .map_err(|e| template_err(id, e))?;

        if def.kind == Kind::Security || def.audience == Audience::Owner {
            // Email approves nothing (ADR-014): no reply-to-approve wording.
            let lower = text.to_lowercase();
            for banned in [
                "reply yes",
                "reply approve",
                "reply \"yes\"",
                "approve by email",
            ] {
                if lower.contains(banned) {
                    return Err(RenderError::Rule {
                        id: id.into(),
                        problem: format!("an email can't approve anything, but it says `{banned}`"),
                    });
                }
            }
        }

        rendered.email = Some(Email {
            from_name: from,
            subject: subject_line,
            preheader: preheader_line,
            text,
            html,
        });
    }

    if def.channels.contains(&Channel::Push) {
        let body = get("push")?.ok_or_else(|| RenderError::Rule {
            id: id.into(),
            problem: "a push needs a `push` block".into(),
        })?;
        let url = get("push_url")?
            .map(|u| one_line(&u))
            .unwrap_or_else(|| "/os".to_string());
        rendered.push = Some(Push {
            title: push_title(ctx),
            body: one_line(&body),
            url,
        });
    }

    if def.channels.contains(&Channel::Chat) {
        let body = get("chat")?.ok_or_else(|| RenderError::Rule {
            id: id.into(),
            problem: "a chat message needs a `chat` block".into(),
        })?;
        rendered.chat = Some(chat_from(
            id,
            &body,
            matches!(def.audience, Audience::Owner | Audience::Teammate),
        )?);
    }

    Ok(rendered)
}

/// Chat bodies use the same line format plus `::reply Label` for quick replies.
/// `decisions` is true for owner and teammate messages, where a reply could stand for an
/// approval: those replies may decline, ask, pause or stop, never approve.
fn chat_from(id: &str, body: &str, decisions: bool) -> Result<Chat, RenderError> {
    let mut replies = Vec::new();
    let mut rest = String::new();
    for line in body.lines() {
        let t = line.trim();
        if let Some(label) = t.strip_prefix("::reply ") {
            replies.push(one_line(label));
        } else {
            rest.push_str(line);
            rest.push('\n');
        }
    }
    let blocks = markup::parse(&rest).map_err(|source| RenderError::Markup {
        id: id.into(),
        block: "chat",
        source,
    })?;
    let mut link = None;
    let mut parts = Vec::new();
    for b in &blocks {
        match b {
            Block::Button { label, url } if link.is_none() => {
                link = Some((markup::plain(label), url.clone()));
            }
            Block::Heading(t) => parts.push(format!("*{}*", markup::plain(t))),
            other => parts.push(markup::to_text(std::slice::from_ref(other))),
        }
    }
    if replies.len() > 3 {
        return Err(RenderError::Rule {
            id: id.into(),
            problem: "a chat message can offer at most three replies".into(),
        });
    }
    for r in &replies {
        let lower = r.to_lowercase();
        // 07 principle 9: chat can ask, decline, pause and stop; approving needs a screen.
        if decisions
            && ["always", "allow", "approve", "accept", "confirm", "yes"]
                .iter()
                .any(|w| lower.contains(w))
        {
            return Err(RenderError::Rule {
                id: id.into(),
                problem: format!("chat never approves, but a reply says `{r}`"),
            });
        }
    }
    Ok(Chat {
        text: parts.join("\n\n"),
        replies,
        link,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_reads_like_the_concepts() {
        assert_eq!(
            money(Value::from(4500), Some("KES".into())).unwrap(),
            "KSh 4,500"
        );
        assert_eq!(money(Value::from(1234567), None).unwrap(), "KSh 1,234,567");
        assert_eq!(
            money(Value::from(19.77), Some("usd".into())).unwrap(),
            "USD 19.77"
        );
        assert_eq!(
            money(Value::from(29), Some("USD".into())).unwrap(),
            "USD 29.00"
        );
        assert_eq!(
            money(Value::from(12.5), Some("KES".into())).unwrap(),
            "KSh 12.50"
        );
    }

    #[test]
    fn plural_counts() {
        assert_eq!(
            plural(1, "decision".into(), "decisions".into()),
            "1 decision"
        );
        assert_eq!(
            plural(3, "decision".into(), "decisions".into()),
            "3 decisions"
        );
    }

    #[test]
    fn values_lose_line_breaks() {
        assert_eq!(clean_value("Amina\n::button x"), "Amina ::button x");
    }
}
