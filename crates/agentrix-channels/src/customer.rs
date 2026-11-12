//! Customers in chat get the site's own Ask: the same desks, citations, refusals and order-status
//! answers as the page, because the question goes to the site's `POST /ask` on node's loopback
//! listener with the site's host (13 §4.5). Horizon 2 swaps this for an in-process port from W2.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::service::{Bot, Channels, SiteInfo, TELEGRAM};
use crate::telegram::{Button, Keyboard, button_url_ok, chat_to_html, escape};

#[derive(Debug, Clone, Deserialize, Default)]
struct Desk {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Source {
    n: usize,
    label: String,
    href: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Action {
    label: String,
    kind: String,
    #[serde(default)]
    href: Option<String>,
    #[serde(default)]
    question: Option<String>,
}

/// Ask's `answer` event.
#[derive(Debug, Clone, Deserialize)]
struct AskAnswer {
    #[serde(default)]
    desk: Desk,
    #[serde(default)]
    handover: Option<String>,
    #[serde(default)]
    phrases: Vec<String>,
    #[serde(default)]
    sources: Vec<Source>,
    #[serde(default)]
    actions: Vec<Action>,
    #[serde(default)]
    meta: String,
}

/// Where node listens on loopback, else the platform origin.
fn loopback(channels: &Channels) -> String {
    channels
        .settings()
        .loopback
        .clone()
        .unwrap_or_else(|| channels.origin())
}

/// Reads an SSE body into `(event, data)` pairs.
fn events(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let (mut event, mut data) = (String::new(), String::new());
    for line in body.lines() {
        if line.is_empty() {
            if !data.is_empty() {
                out.push((std::mem::take(&mut event), std::mem::take(&mut data)));
            }
            event.clear();
            continue;
        }
        if let Some(e) = line.strip_prefix("event:") {
            event = e.trim().to_owned();
        } else if let Some(d) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(d.strip_prefix(' ').unwrap_or(d));
        }
    }
    if !data.is_empty() {
        out.push((event, data));
    }
    out
}

async fn ask_site(
    channels: &Channels,
    site: &SiteInfo,
    q: &str,
    client_key: &str,
) -> anyhow::Result<AskAnswer> {
    let body = channels
        .ctx()
        .http
        .post(format!("{}/ask", loopback(channels)))
        .header(reqwest::header::HOST, site.host.as_str())
        .header("x-forwarded-for", client_key)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .timeout(Duration::from_secs(30))
        .json(&json!({ "q": q }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Ask didn't answer: {}", e.without_url()))?
        .error_for_status()
        .map_err(|e| anyhow::anyhow!("Ask refused: {}", e.without_url()))?
        .text()
        .await?;
    let answer = events(&body)
        .into_iter()
        .find(|(e, _)| e == "answer")
        .ok_or_else(|| anyhow::anyhow!("Ask sent no answer"))?;
    Ok(serde_json::from_str(&answer.1)?)
}

/// The site's answer to `q`, as Telegram HTML and buttons, and the desk that answered.
pub async fn answer(
    channels: &Channels,
    bot: &Arc<Bot>,
    site: &SiteInfo,
    q: &str,
    client_key: &str,
) -> (String, Keyboard, Option<String>) {
    let origin = channels.site_origin(&site.host);
    let site_word = format!(
        "{}{}",
        site.name,
        if site.sample { " (Sample)" } else { "" }
    );
    let a = match ask_site(channels, site, q, client_key).await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "Ask failed for a chat");
            return (
                format!(
                    "Answers are paused right now. The site still works: {}",
                    escape(&origin)
                ),
                if button_url_ok(&origin) {
                    vec![vec![Button::link("Open the site", origin)]]
                } else {
                    Vec::new()
                },
                None,
            );
        }
    };
    let desk = if a.desk.name.is_empty() {
        "Front desk".to_owned()
    } else {
        a.desk.name.clone()
    };
    let mut html = format!(
        "<b>{}</b> · WebAgent · {}",
        escape(&desk),
        escape(&site_word)
    );
    if let Some(h) = a.handover.as_deref().filter(|h| !h.is_empty()) {
        html.push_str(&format!("\n<i>{}</i>", escape(h)));
    }
    html.push_str("\n\n");
    html.push_str(&chat_to_html(&a.phrases.join(" ")));
    if !a.sources.is_empty() {
        html.push('\n');
        for s in &a.sources {
            let url = format!("{origin}{}", s.href);
            if button_url_ok(&url) {
                html.push_str(&format!(
                    "\n[{}] <a href=\"{}\">{}</a>",
                    s.n,
                    escape(&url),
                    escape(&s.label)
                ));
            } else {
                html.push_str(&format!(
                    "\n[{}] {} ({})",
                    s.n,
                    escape(&s.label),
                    escape(&url)
                ));
            }
        }
    }
    if !a.meta.is_empty() {
        html.push_str(&format!("\n\n<i>{}</i>", escape(&a.meta)));
    }
    let mut row = Vec::new();
    for act in a.actions.iter().take(3) {
        match act.kind.as_str() {
            "link" => {
                if let Some(h) = &act.href {
                    let url = if h.starts_with("http") {
                        h.clone()
                    } else {
                        format!("{origin}{h}")
                    };
                    row.push(Button::link(act.label.clone(), url));
                }
            }
            // The question is the chat's last turn; the button names it.
            "pass" if act.question.is_some() => {
                row.push(Button::callback(act.label.clone(), "pass:last"));
            }
            _ => {}
        }
    }
    let _ = bot;
    let kb = if row.is_empty() {
        Vec::new()
    } else {
        vec![row]
    };
    (html, kb, Some(a.desk.id).filter(|d| !d.is_empty()))
}

/// Passes the chat's last question to the owner through the site's own pass route, so it lands
/// where the page's questions land. Returns the site's words.
pub async fn pass(channels: &Channels, bot: &Arc<Bot>, chat: &str, _turn: &str) -> String {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT asked, desk FROM channel_turns WHERE channel = ? AND bot = ? AND chat_id = ? AND role = 'customer' ORDER BY id DESC LIMIT 1",
    )
    .bind(TELEGRAM)
    .bind(&bot.cfg.site)
    .bind(chat)
    .fetch_optional(channels.store().pool())
    .await
    .unwrap_or(None);
    let Some((question, desk)) = row else {
        return "I couldn't find the question to pass on. Ask it again, then tap the button."
            .into();
    };
    let site = channels.site(&bot.cfg.site).await;
    let key = format!(
        "tg-{}",
        crate::store::hash_code(&format!("{}:{chat}", bot.cfg.site))
            .get(..16)
            .unwrap_or("x")
    );
    let sent = channels
        .ctx()
        .http
        .post(format!("{}/ask/pass", loopback(channels)))
        .header(reqwest::header::HOST, site.host.as_str())
        .header("x-forwarded-for", key)
        .timeout(Duration::from_secs(15))
        .json(&json!({ "question": question, "desk": desk, "page": "Telegram" }))
        .send()
        .await;
    match sent {
        Ok(r) => {
            let v: Value = r.json().await.unwrap_or(Value::Null);
            v.get("words")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| "Passed to the owner.".into())
        }
        Err(_) => "I couldn't pass it on just now. Try again in a minute.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_events_are_read() {
        let body = "event: progress\ndata: {\"label\":\"Searching\"}\n\nevent: answer\ndata: {\"phrases\":[\"Yes.\"]}\n\nevent: done\ndata: {}\n\n";
        let ev = events(body);
        assert_eq!(ev.len(), 3);
        assert_eq!(ev[1].0, "answer");
        let a: AskAnswer = serde_json::from_str(&ev[1].1).unwrap();
        assert_eq!(a.phrases, vec!["Yes."]);
    }
}
