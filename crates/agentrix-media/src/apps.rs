//! The console apps (W5's contract C9): **Pictures**, the media library, and **Talk**, the
//! voice panel. The shell puts them in the dock, signs the owner in, checks they own the site
//! and wraps each page in the console.

use std::sync::Arc;

use agentrix_host::NodeContext;
use agentrix_shell::apps::{AppContext, AppPage, AppReply, AppSpec, ConsoleApp};
use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};

use crate::library::{self, DecideError, Image, Request};
use crate::pipeline::{Ask, MakeError};
use crate::policy::Kind;
use crate::routes::{MediaState, draw_and_keep, is_sample};

/// A short hash of the panel's assets, so a new release never runs with a cached old script.
pub fn asset_v() -> &'static str {
    use std::hash::{Hash as _, Hasher as _};
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for asset in [
            include_str!("../static/voice.js"),
            include_str!("../static/voice.css"),
            include_str!("../static/media.css"),
            include_str!("../static/pcm-capture.js"),
        ] {
            asset.hash(&mut h);
        }
        format!("{:012x}", h.finish() & 0xffff_ffff_ffff)
    })
}

/// Escapes text for HTML.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Both apps, for node to hand to the shell.
pub fn console_apps(ctx: &NodeContext) -> Vec<Arc<dyn ConsoleApp>> {
    let state = MediaState::new(ctx);
    vec![
        Arc::new(PicturesApp {
            state: state.clone(),
        }),
        Arc::new(TalkApp),
    ]
}

/// Talk to your Chief WebAgent.
pub struct TalkApp;

/// The voice panel's HTML for a site (concept/console/live.html `#voice`, adk-surface Voice).
pub fn talk_html(site: &str) -> String {
    let api = format!("/api/voice/{}", esc(site));
    let v = asset_v();
    format!(
        r#"<link rel="stylesheet" href="/static/media/voice.css?v={v}">
<div class="field-head"><h2>Talk to your Chief WebAgent</h2><p class="lead">Hold the button and speak. Your words appear as you say them, and the Chief answers out loud.</p></div>
<section class="voice-panel" data-voice data-api="{api}" data-mode="idle" aria-label="Voice">
  <div class="voice-ctl"><span class="voice-level" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i><i></i></span><div class="vstate" data-vstate>Ready<small>Hold to talk, or read your brief</small></div></div>
  <div class="vrow"><button class="btn btn-primary" type="button" data-talk aria-pressed="false">Hold to talk</button><button class="btn" type="button" data-brief>Read my brief</button><button class="btn btn-ghost" type="button" data-hush hidden>Stop speaking</button></div>
  <form class="vtype" data-vtype autocomplete="off"><label class="sr-only" for="vtype-said">Or type what you'd say</label><input id="vtype-said" name="said" maxlength="300" placeholder="Or type it: deny, read my brief, stop everything"><button class="btn btn-sm" type="submit">Say it</button></form>
  <p class="verror" role="alert" data-verror></p>
  <div class="captions" aria-live="polite" data-caps></div>
  <section class="vdecision" aria-label="Voice decision" data-vdecision hidden>
    <div class="vtop"><span data-vheard></span><span class="vcount" data-vcount></span></div>
    <h3 data-vtitle></h3>
    <div class="vfoot"><span class="small muted" data-vmeta></span><button class="btn" type="button" data-vcancel>Cancel</button></div>
  </section>
  <p class="small muted">By voice you can say "allow once" or "deny" to the decision at the top of your lane, and "stop everything", which acts at once. Approving opens the sheet on your screen for your passkey: voice never approves. We keep a note of your answer, never the recording.</p>
</section>
<script type="module" src="/static/media/voice.js?v={v}"></script>"#
    )
}

#[async_trait]
impl ConsoleApp for TalkApp {
    fn spec(&self) -> AppSpec {
        AppSpec {
            id: "talk".into(),
            name: "Talk".into(),
            title: "Talk to your Chief WebAgent".into(),
            icon: "voice".into(),
            webagent: Some("Chief WebAgent".into()),
            order: 5,
        }
    }

    async fn page(&self, ctx: &AppContext, path: &str, _query: &str) -> anyhow::Result<AppPage> {
        if !path.is_empty() {
            return Ok(AppPage::NotFound);
        }
        Ok(AppPage::Field {
            title: "Talk to your Chief WebAgent".into(),
            html: talk_html(&ctx.site_id),
        })
    }
}

/// The media library.
pub struct PicturesApp {
    state: MediaState,
}

fn slot_words(slot: &str) -> String {
    let (kind, name) = slot.split_once(':').unwrap_or((slot, ""));
    let name = name.replace(['-', '_'], " ");
    match kind {
        "hero" => "Home page, top".into(),
        "product" => format!("Product: {name}"),
        "section" => format!("Section: {name}"),
        "post" => format!("Post: {name}"),
        "story" => format!("Story: {name}"),
        "part" => format!("Sketch part: {name}"),
        "page" => format!("Page: {name}"),
        _ => slot.to_owned(),
    }
}

fn status_words(image: &Image) -> &'static str {
    match image.status.as_str() {
        "draft" => "Waiting for you",
        "approved" if image.asked_by == "sample" => "Shown · sample picture",
        "approved" => "Shown on your site",
        "replaced" => "Replaced",
        "declined" => "Declined",
        "blocked" => "Stopped by the safety check",
        _ => "",
    }
}

fn dollars(micro: i64) -> String {
    if micro <= 0 {
        "no charge".into()
    } else if micro < 10_000 {
        "under $0.01".into()
    } else {
        format!("${:.2}", micro as f64 / 1e6)
    }
}

fn card_html(base: &str, site: &str, image: &Image) -> String {
    let id = esc(&image.image_id);
    let picture = match image.status.as_str() {
        "blocked" => r#"<div class="media-blocked">No picture kept</div>"#.to_owned(),
        "approved" => format!(
            r#"<img src="{}" alt="{}" loading="lazy"{}>"#,
            esc(&image.url()),
            esc(&image.alt),
            size_attrs(image)
        ),
        _ => format!(
            r#"<img src="/api/media/{}/{id}/file" alt="{}" loading="lazy"{}>"#,
            esc(site),
            esc(&image.alt),
            size_attrs(image)
        ),
    };
    let label = image
        .label
        .as_deref()
        .map(|l| format!(r#"<span class="rtag">{}</span>"#, esc(l)))
        .unwrap_or_default();
    let check = if image.owner_check.trim().is_empty() {
        String::new()
    } else {
        format!(
            r#"<p class="small"><b>Check:</b> {}</p>"#,
            esc(&image.owner_check)
        )
    };
    let made = if image.model.is_empty() {
        String::new()
    } else {
        format!(
            r#"<p class="small muted">Drawn with {} · {}</p>"#,
            esc(&image.model),
            dollars(image.cost_micro_usd)
        )
    };
    let verbs = match image.status.as_str() {
        "draft" => format!(
            r#"<div class="verbs"><form method="post" action="{base}/approve/{id}"><button class="btn btn-primary btn-sm" type="submit">Approve</button></form><form method="post" action="{base}/again/{id}"><button class="btn btn-sm" type="submit">Draw another</button></form><form method="post" action="{base}/decline/{id}"><button class="btn btn-ghost btn-sm" type="submit">Decline</button></form></div>"#
        ),
        "approved" => format!(
            r#"<div class="verbs"><form method="post" action="{base}/again/{id}"><button class="btn btn-sm" type="submit">Replace</button></form></div>"#
        ),
        _ => String::new(),
    };
    let alt = if image.alt.is_empty() {
        format!("<p>{}</p>", esc(&image.brief))
    } else {
        format!("<p>{}</p>", esc(&image.alt))
    };
    format!(
        r#"<article class="fcard media-card" data-status="{}"><div class="media-pic">{picture}{label}</div><div class="fcard-head"><div class="who"><b>{}</b><span class="state-line">{}</span></div></div>{alt}{check}{made}{verbs}</article>"#,
        esc(&image.status),
        esc(&slot_words(&image.slot)),
        status_words(image),
    )
}

fn size_attrs(image: &Image) -> String {
    match (image.width, image.height) {
        (Some(w), Some(h)) => format!(r#" width="{w}" height="{h}""#),
        _ => String::new(),
    }
}

const KINDS: [(&str, &str); 5] = [
    ("product", "A product"),
    ("hero", "The top of the home page"),
    ("section", "A page section"),
    ("post", "A post"),
    ("news-illustration", "A drawn illustration"),
];

fn pictures_html(
    ctx: &AppContext,
    images: &[Image],
    spend: &serde_json::Value,
    flash: &str,
) -> String {
    let base = esc(&ctx.base);
    let waiting = images.iter().filter(|i| i.status == "draft").count();
    let head_line = if waiting == 0 {
        "Nothing waits for you. New pictures appear here as drafts; none reaches your site until you approve it.".to_owned()
    } else {
        format!(
            "{waiting} picture{} wait{} for you. None reaches your site until you approve it.",
            if waiting == 1 { "" } else { "s" },
            if waiting == 1 { "s" } else { "" }
        )
    };
    let release_line = if ctx.sample {
        "This is a sample site: approved pictures show on it at once."
    } else {
        "Approved pictures go out with your next release, which you sign."
    };
    let made = spend["made"].as_i64().unwrap_or(0);
    let cost = dollars(spend["cost_micro_usd"].as_i64().unwrap_or(0));
    let spend_line = if made == 0 {
        "No pictures drawn this month.".to_owned()
    } else {
        format!(
            "{made} picture{} drawn this month, {cost}.",
            if made == 1 { "" } else { "s" }
        )
    };
    let kinds: String = KINDS
        .iter()
        .map(|(v, w)| format!(r#"<option value="{v}">{w}</option>"#))
        .collect();
    let cards: String = images
        .iter()
        .filter(|i| i.status != "replaced" && i.status != "declined")
        .map(|i| card_html(&ctx.base, &ctx.site_id, i))
        .collect();
    let empty = if cards.is_empty() {
        r#"<p class="quiet">No pictures yet. Describe one below and the Editor draws it.</p>"#
    } else {
        ""
    };
    let v = asset_v();
    let flash = if flash.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="banner" role="status">{}</p>"#, esc(flash))
    };
    format!(
        r#"<link rel="stylesheet" href="/static/media/media.css?v={v}">
<div class="field-head"><h2>Pictures</h2><p class="lead">{head_line} {release_line}</p><p class="small muted">{spend_line}</p></div>
{flash}
<div class="media-grid">{cards}</div>{empty}
<section class="fcard media-draw" aria-labelledby="draw-h">
  <h3 id="draw-h">Draw a new picture</h3>
  <form method="post" action="{base}/draw" class="media-form">
    <label>Where it goes <select name="kind">{kinds}</select></label>
    <label>Name it <input name="slot" required maxlength="80" placeholder="product:sourdough"></label>
    <label class="wide">What it shows <textarea name="brief" required maxlength="600" rows="3" placeholder="A round sourdough loaf with a dark crust on a wooden board"></textarea></label>
    <p class="small muted wide">Pictures never show people, real events or logos. Each is checked for safety, gets alt text, and is labelled as made with AI wherever someone could take it for a photo.</p>
    <div class="verbs wide"><button class="btn btn-primary" type="submit">Draw it</button></div>
  </form>
</section>"#
    )
}

fn form(body: &Bytes) -> Vec<(String, String)> {
    url::form_urlencoded::parse(body).into_owned().collect()
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> &'a str {
    fields
        .iter()
        .find(|(k, _)| k == name)
        .map_or("", |(_, v)| v.as_str())
}

#[async_trait]
impl ConsoleApp for PicturesApp {
    fn spec(&self) -> AppSpec {
        AppSpec {
            id: "pictures".into(),
            name: "Pictures".into(),
            title: "Pictures: what your site shows, and drafts waiting for you".into(),
            icon: "media".into(),
            webagent: Some("Editor".into()),
            order: 30,
        }
    }

    async fn badge(&self, ctx: &AppContext) -> anyhow::Result<u32> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM media_images WHERE site_id = ? AND status = 'draft'",
        )
        .bind(&ctx.site_id)
        .fetch_one(&self.state.ctx().pool)
        .await?;
        Ok(u32::try_from(n).unwrap_or(0))
    }

    async fn page(&self, ctx: &AppContext, path: &str, query: &str) -> anyhow::Result<AppPage> {
        if !path.is_empty() {
            return Ok(AppPage::NotFound);
        }
        let pool = &self.state.ctx().pool;
        let images = library::list(pool, &ctx.site_id).await?;
        let spend = library::month_spend(pool, &ctx.site_id).await?;
        let flash: String = url::form_urlencoded::parse(query.as_bytes())
            .find(|(k, _)| k == "said")
            .map(|(_, v)| v.chars().take(300).collect())
            .unwrap_or_default();
        Ok(AppPage::Field {
            title: "Pictures".into(),
            html: pictures_html(ctx, &images, &spend, &flash),
        })
    }

    async fn post(
        &self,
        ctx: &AppContext,
        path: &str,
        _headers: &HeaderMap,
        body: Bytes,
    ) -> anyhow::Result<AppReply> {
        let back = |said: &str| {
            let q: String = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("said", said)
                .finish();
            AppReply::Redirect(format!("{}?{q}", ctx.base))
        };
        let pool = &self.state.ctx().pool;
        let who = format!("owner:{}", ctx.account);
        let (verb, id) = path.split_once('/').unwrap_or((path, ""));
        let decided = |r: anyhow::Result<Image>, done: &str| match r {
            Ok(_) => Ok(back(done)),
            Err(err) => match err.downcast::<DecideError>() {
                Ok(e) => Ok(back(&e.to_string())),
                Err(err) => Err(err),
            },
        };
        match verb {
            "approve" => decided(
                library::approve(pool, &self.state.ctx().receipts, &ctx.site_id, id, &who).await,
                "Approved. A receipt records it.",
            ),
            "decline" => decided(
                library::decline(pool, &ctx.site_id, id, &who).await,
                "Declined. It won't be shown.",
            ),
            "again" | "draw" => {
                let fields = form(&body);
                let (slot, kind, brief, replaces) = if verb == "again" {
                    let Some(old) = library::get(pool, &ctx.site_id, id).await? else {
                        return Ok(back(&DecideError::NotFound.to_string()));
                    };
                    let kind: Kind = serde_json::from_value(serde_json::json!(old.kind))
                        .unwrap_or(Kind::Section);
                    (
                        old.slot.clone(),
                        kind,
                        old.brief.clone(),
                        Some(old.image_id.clone()),
                    )
                } else {
                    let kind: Kind =
                        serde_json::from_value(serde_json::json!(field(&fields, "kind")))
                            .unwrap_or(Kind::Product);
                    let slot = field(&fields, "slot").trim().to_owned();
                    let slot = if slot.contains(':') || slot == "hero" || slot.is_empty() {
                        slot
                    } else {
                        let prefix = match kind {
                            Kind::Product => "product",
                            Kind::Post => "post",
                            Kind::NewsIllustration => "story",
                            _ => "section",
                        };
                        format!("{prefix}:{}", slot.to_lowercase().replace(' ', "-"))
                    };
                    (slot, kind, field(&fields, "brief").trim().to_owned(), None)
                };
                if kind == Kind::Film {
                    return Ok(back(
                        "Pictures for your site never show people. Choose a product, a section or a post.",
                    ));
                }
                if kind == Kind::CakeIdea {
                    return Ok(back(crate::routes::IDEAS_ARE_FOR_VISITORS));
                }
                if slot.is_empty()
                    || !slot
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'))
                {
                    return Ok(back(
                        "Name the picture with letters, digits and dashes, for example product:sourdough.",
                    ));
                }
                let request = Request {
                    site: &ctx.site_id,
                    slot: &slot,
                    kind,
                    brief: &brief,
                    asked_by: "owner",
                    replaces: replaces.as_deref(),
                };
                let ask = Ask {
                    kind,
                    brief: brief.clone(),
                    look: String::new(),
                    context: format!("A picture for {} ({})", ctx.site_name, slot_words(&slot)),
                    n: 1,
                    seed: None,
                    tier: "fast",
                    sample: is_sample(&ctx.site_id),
                };
                match draw_and_keep(&self.state, &request, &ask).await {
                    Ok((made, blocked)) if made.is_empty() && !blocked.is_empty() => Ok(back(
                        "The safety check stopped that picture, so nothing was kept. Try describing it another way.",
                    )),
                    Ok(_) => Ok(back("Drawn. It waits for you below.")),
                    Err(MakeError::Refused(r)) => Ok(back(&r.0)),
                    Err(MakeError::Gateway(e)) => Ok(back(&e.message)),
                }
            }
            _ => Ok(AppReply::Error(
                StatusCode::NOT_FOUND,
                "No such action.".into(),
            )),
        }
    }
}
