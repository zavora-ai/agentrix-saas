//! Settings → Channels (contract C9): a console app where the owner links Telegram with a one-time
//! deep link, unlinks in one tap, sees WhatsApp's and email's state, and reads the one rule table
//! that decides what each place may do.

use std::collections::HashMap;
use std::sync::Mutex;

use agentrix_shell::apps::{AppContext, AppPage, AppReply, AppSpec, ConsoleApp};
use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};

use crate::service::{Channels, TELEGRAM, clock};
use crate::store::{LINK_CODE_TTL, PAIR_CODE_TTL};
use crate::telegram::escape;

/// Deep links waiting to be opened: (account, site) → (code, until).
type Pending = HashMap<(String, String), (String, DateTime<Utc>)>;

/// The Channels app.
pub struct ChannelsApp {
    channels: Channels,
    /// Deep links made in the last 15 minutes, by (account, site), shown once on the page. Only
    /// the code's hash is in the database; this copy lives in memory and is lost on restart.
    pending: Mutex<Pending>,
    /// Phone pairing codes, the same way.
    pairing: Mutex<Pending>,
}

impl ChannelsApp {
    /// Over the channels service.
    pub fn new(channels: Channels) -> Self {
        Self {
            channels,
            pending: Mutex::new(HashMap::new()),
            pairing: Mutex::new(HashMap::new()),
        }
    }

    fn pending_pair(&self, account: &str, site: &str) -> Option<(String, DateTime<Utc>)> {
        let mut map = self.pairing.lock().unwrap_or_else(|e| e.into_inner());
        let now = Utc::now();
        map.retain(|_, (_, until)| *until > now);
        map.get(&(account.to_owned(), site.to_owned())).cloned()
    }

    fn pending_link(&self, account: &str, site: &str) -> Option<(String, DateTime<Utc>)> {
        let mut map = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let now = Utc::now();
        map.retain(|_, (_, until)| *until > now);
        map.get(&(account.to_owned(), site.to_owned())).cloned()
    }
}

/// The channel ids other pages link here with (`?connect=`), W4's post-go-live step among them.
pub const CHANNEL_IDS: [&str; 8] = [
    "app", "device", "email", "telegram", "whatsapp", "slack", "discord", "matrix",
];

/// A `back` address the page may link to: a path on this origin, never another site.
fn safe_back(back: Option<&String>) -> Option<String> {
    back.map(|b| b.trim().to_owned()).filter(|b| {
        b.starts_with('/')
            && !b.starts_with("//")
            && !b.contains('\\')
            && b.len() <= 300
            && b.chars().all(|c| !c.is_control())
    })
}

fn with_back(path: &str, back: Option<&str>) -> String {
    match back {
        Some(b) => {
            let enc: String = url::form_urlencoded::byte_serialize(b.as_bytes()).collect();
            let sep = if path.contains('?') { '&' } else { '?' };
            format!("{path}{sep}back={enc}")
        }
        None => path.to_owned(),
    }
}

fn day_time(at: DateTime<Utc>, offset: i32) -> String {
    let o = chrono::FixedOffset::east_opt(offset * 60)
        .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).expect("zero offset"));
    at.with_timezone(&o).format("%a %-d %b, %H:%M").to_string()
}

fn qr_svg(text: &str) -> String {
    match qrcode::QrCode::new(text.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::svg::Color<'_>>()
            .min_dimensions(176, 176)
            .quiet_zone(true)
            .dark_color(qrcode::render::svg::Color("#1f1d3d"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .build(),
        Err(_) => String::new(),
    }
}

#[async_trait]
impl ConsoleApp for ChannelsApp {
    fn spec(&self) -> AppSpec {
        AppSpec {
            id: "channels".into(),
            name: "Channels".into(),
            title: "Channels: Telegram, WhatsApp and email, and what each may decide".into(),
            icon: "bell".into(),
            webagent: None,
            order: 92,
        }
    }

    async fn page(&self, ctx: &AppContext, path: &str, query: &str) -> anyhow::Result<AppPage> {
        if !path.is_empty() {
            return Ok(AppPage::NotFound);
        }
        let q: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        let ch = &self.channels;
        let bot = ch.bot_for(&ctx.site_id);
        let link = ch
            .store()
            .link_for_owner(TELEGRAM, &ctx.site_id, &ctx.account)
            .await?;
        let offset = ctx.utc_offset_minutes;
        let base = escape(&ctx.base);
        // W4's Move page links here with ?connect=telegram&back=/sites/{id}/…: the Telegram card
        // takes focus, and a link leads back once the owner is done.
        let back = safe_back(q.get("back"));
        let back_field = back
            .as_deref()
            .map(|b| {
                format!(
                    "<input type=\"hidden\" name=\"back\" value=\"{}\">",
                    escape(b)
                )
            })
            .unwrap_or_default();
        // ?connect=<id> (W4's ids: device, email, telegram, whatsapp, slack, discord, matrix)
        // brings that channel's card forward; an id we don't know is ignored.
        let wanted = q
            .get("connect")
            .map(String::as_str)
            .filter(|c| CHANNEL_IDS.contains(c));
        let connect = wanted == Some("telegram");
        let focus = |id: &str| {
            if wanted == Some(id) {
                " tabindex=\"-1\" data-focus"
            } else {
                ""
            }
        };

        let mut html = String::new();
        html.push_str(&format!(
            "<div class=\"field-head\"><div><h2>Channels</h2><div class=\"sub\">Where {}'s WebAgents reach you, and what each place may decide.</div></div></div>",
            escape(&ctx.site_name)
        ));
        let note = match q.get("done").map(String::as_str) {
            Some("unlinked") => Some("Unlinked. Nothing more goes to that chat."),
            Some("linked") => Some("Linked."),
            Some("removed") => Some("Removed. That phone can't open your site any more."),
            _ => None,
        };
        if let Some(n) = note {
            html.push_str(&format!(
                "<p class=\"note ok-note\" role=\"status\"><span>{n}</span></p>"
            ));
        }
        if let Some(b) = &back {
            html.push_str(&format!(
                "<p class=\"small\"><a href=\"{}\">Back to where you were</a></p>",
                escape(b)
            ));
        }
        html.push_str("<div class=\"cards\">");

        // Telegram.
        html.push_str(&format!(
            "<article class=\"fcard span2\" id=\"telegram\" aria-labelledby=\"tg-h\"{}><div class=\"fcard-head\"><div class=\"who\"><b id=\"tg-h\">Telegram</b>",
            focus("telegram")
        ));
        match (&bot, &link) {
            (None, _) => {
                html.push_str(
                    "<span class=\"state-line\">Not connected for this site yet</span></div></div>",
                );
                html.push_str("<p>Once this site has its Telegram bot, you can link your Telegram here: the morning brief, a note when a decision is waiting and questions your desks pass on, with Decline and STOP in the chat. Approving always opens the console.</p>");
            }
            (Some(b), Some(l)) => {
                let handle = b
                    .username()
                    .map(|u| format!(" through @{u}"))
                    .unwrap_or_default();
                // Just linked from W4's Move page: say so, then go back there by itself.
                if let Some(b) = back
                    .as_deref()
                    .filter(|_| Utc::now() - l.linked_at < chrono::Duration::minutes(3))
                {
                    html.push_str(&format!(
                        "<p class=\"small\" role=\"status\">Linked. Taking you back…</p><script>setTimeout(function(){{location.assign({})}},2500)</script>",
                        // JSON, with `<` escaped so a path can't close the script.
                        serde_json::to_string(b)
                            .unwrap_or_else(|_| "\"/\"".into())
                            .replace('<', "\\u003c")
                    ));
                }
                html.push_str(&format!(
                    "<span class=\"state-line\" data-tg-linked>Linked to {} since {}{}</span></div><span class=\"chip chip-ok\">On</span></div>",
                    escape(&l.label),
                    escape(&day_time(l.linked_at, offset)),
                    escape(&handle)
                ));
                html.push_str("<p>Your Telegram chat gets the morning brief when quiet hours end, a note when a decision is waiting, and questions your desks pass on. From it you can decline, pause a WebAgent with /pause, or send STOP to freeze every WebAgent. It can't approve anything: approving opens the decision here, where you use your passkey or authenticator code.</p>");
                html.push_str(&format!(
                    "<form method=\"post\" action=\"{base}/telegram/unlink\" class=\"verbs\">{back_field}<button class=\"btn btn-sm\" type=\"submit\">Unlink</button></form>"
                ));
            }
            (Some(b), None) => {
                let pending = self.pending_link(&ctx.account, &ctx.site_id);
                match (pending, b.username()) {
                    (Some((code, until)), Some(_)) => {
                        let url = b.deep_link(&code).unwrap_or_default();
                        html.push_str(&format!(
                            "<span class=\"state-line\">Waiting for you to open the link, until {}</span></div><span class=\"chip chip-wait\">Waiting</span></div>",
                            escape(&clock(until, offset))
                        ));
                        html.push_str(&format!(
                            "<p>Open this on the phone that has your Telegram, and tap Start. The link works once, within 15 minutes. Keep it to yourself: the chat that opens it gets your owner messages.</p><div class=\"verbs\"><a class=\"btn btn-primary btn-sm\" href=\"{}\" rel=\"noopener\">Open Telegram</a></div><div class=\"qr\" aria-label=\"QR code for the Telegram link\" style=\"margin-top:12px;max-width:192px\">{}</div><p class=\"small muted\">Or scan it with your phone's camera.</p><p class=\"small\" role=\"status\">Waiting for you in Telegram. This page changes by itself once you tap Start.</p>",
                            escape(&url),
                            qr_svg(&url)
                        ));
                        // Checks this page every 3 seconds (for up to 15 minutes, the code's
                        // life) and reloads once the chat is linked, or the code has run out and
                        // a fresh one is needed.
                        html.push_str("<script>(function(){var n=0;function tick(){if(++n>300)return;fetch(location.href,{credentials:'same-origin',cache:'no-store'}).then(function(r){return r.text()}).then(function(t){if(t.indexOf('data-tg-linked')>=0||t.indexOf('data-tg-wait')<0){location.reload()}else{setTimeout(tick,3000)}}).catch(function(){setTimeout(tick,5000)})}setTimeout(tick,3000)})();</script><span hidden data-tg-wait></span>");
                    }
                    (_, None) => {
                        html.push_str("<span class=\"state-line\">Reaching Telegram</span></div></div><p>The bot hasn't answered yet. Reload in a moment.</p>");
                    }
                    (None, Some(u)) => {
                        html.push_str(&format!(
                            "<span class=\"state-line\">Not linked · @{}</span></div></div>",
                            escape(&u)
                        ));
                        html.push_str("<p>Get the morning brief, a note when a decision is waiting and questions your desks pass on, in Telegram. From the chat you can decline, pause a WebAgent or send STOP to freeze every WebAgent. Approving always opens the console.</p>");
                        html.push_str(&format!(
                            "<form method=\"post\" action=\"{base}/telegram/link\" class=\"verbs\">{back_field}<button class=\"btn btn-primary btn-sm\" type=\"submit\"{}>Link Telegram</button></form>",
                            if connect { " autofocus" } else { "" }
                        ));
                    }
                }
            }
        }
        if let Some(b) = &bot
            && let Some(u) = b.username()
        {
            html.push_str(&format!(
                "<p class=\"small muted\">Customers can ask your desks at t.me/{} and get the same answers as on the site.</p>",
                escape(&u)
            ));
        }
        html.push_str("</article>");

        // The owner app on a phone.
        let devices = ch.store().devices(&ctx.account, &ctx.site_id).await?;
        html.push_str(&format!(
            "<article class=\"fcard span2\" id=\"app\" aria-labelledby=\"app-h\"{}><div class=\"fcard-head\"><div class=\"who\"><b id=\"app-h\">Agentrix on your phone</b><span class=\"state-line\">{}</span></div></div>",
            focus("app"),
            match devices.len() {
                0 => "No phone paired".to_owned(),
                1 => "1 phone paired".to_owned(),
                n => format!("{n} phones paired"),
            }
        ));
        html.push_str("<p>The Agentrix app shows your brief, the decisions waiting for you and your WebAgents. You can approve in the app with a code from your authenticator app; the code approves exactly the decision on screen. To use a passkey, the app opens the decision here. A phone you remove can't open your site any more.</p>");
        if let Some((code, until)) = self.pending_pair(&ctx.account, &ctx.site_id) {
            let link = format!(
                "agentrix://pair?origin={}&code={}",
                url::form_urlencoded::byte_serialize(ch.origin().as_bytes()).collect::<String>(),
                code
            );
            html.push_str(&format!(
                "<p>In the app, tap Pair and type this code, or scan it. It works once, until {}.</p><p style=\"font:600 28px/1.2 var(--font-mono, monospace);letter-spacing:.12em\" aria-label=\"Pairing code\">{}</p><div class=\"qr\" aria-label=\"QR code for pairing\" style=\"max-width:192px\">{}</div>",
                escape(&clock(until, offset)),
                escape(&code),
                qr_svg(&link)
            ));
        } else {
            html.push_str(&format!(
                "<form method=\"post\" action=\"{base}/app/pair\" class=\"verbs\">{back_field}<button class=\"btn btn-primary btn-sm\" type=\"submit\">Pair a phone</button></form>"
            ));
        }
        if !devices.is_empty() {
            html.push_str("<ul class=\"procs\" style=\"margin-top:12px\">");
            for d in &devices {
                html.push_str(&format!(
                    "<li><b>{}</b> · paired {} · last used {} <form method=\"post\" action=\"{base}/app/remove\" style=\"display:inline\"><input type=\"hidden\" name=\"device\" value=\"{}\">{back_field}<button class=\"btn btn-ghost btn-sm\" type=\"submit\">Remove</button></form></li>",
                    escape(&d.name),
                    escape(&day_time(d.created_at, offset)),
                    escape(&day_time(d.last_used_at, offset)),
                    escape(&d.id)
                ));
            }
            html.push_str("</ul>");
        }
        html.push_str("</article>");

        // WhatsApp and email.
        html.push_str(&format!("<article class=\"fcard\" id=\"whatsapp\" aria-labelledby=\"wa-h\"{}><div class=\"fcard-head\"><div class=\"who\"><b id=\"wa-h\">WhatsApp</b><span class=\"state-line\">Not connected yet</span></div></div><p>Once your business's WhatsApp number is verified, it gets the same brief and decisions as Telegram. You can decline there; approving opens the console. STOP freezes every WebAgent.</p></article>", focus("whatsapp")));
        html.push_str(&format!("<article class=\"fcard\" id=\"device\" aria-labelledby=\"dv-h\"{}><div class=\"fcard-head\"><div class=\"who\"><b id=\"dv-h\">Phone notifications</b><span class=\"state-line\">Not switched on yet</span></div></div><p>A notification only says that something is waiting, like \"A decision is waiting\". The details show in the console once you're past the lock screen. Until they're switched on, Telegram above tells you when a decision is waiting.</p></article>", focus("device")));
        for (id, name, place) in [
            ("slack", "Slack", "channel"),
            ("discord", "Discord", "server"),
            ("matrix", "Matrix", "room"),
        ] {
            html.push_str(&format!(
                "<article class=\"fcard\" id=\"{id}\" aria-labelledby=\"{id}-h\"{}><div class=\"fcard-head\"><div class=\"who\"><b id=\"{id}-h\">{name}</b><span class=\"state-line\">Not built yet</span></div></div><p>Your brief and the decisions waiting for you, posted in your team's {place}, with the same limits as Telegram. For now, Telegram above carries the same messages.</p></article>",
                focus(id)
            ));
        }
        html.push_str(&format!(
            "<article class=\"fcard\" id=\"email\" aria-labelledby=\"em-h\"><div class=\"fcard-head\"><div class=\"who\"><b id=\"em-h\">Email</b><span class=\"state-line\">The brief and weekly review, with links</span></div></div><p>Email never approves anything, because email is easy to forge and forward. Its links open the decision here.</p><div class=\"verbs\"><a class=\"btn btn-sm\" href=\"{}/apps/messages/notifications\">Choose what's emailed</a></div></article>",
            escape(&ctx.console)
        ));

        // The rule table.
        html.push_str("<article class=\"fcard span2\" aria-labelledby=\"rules-h\"><div class=\"fcard-head\"><div class=\"who\"><b id=\"rules-h\">What each place may decide</b><span class=\"state-line\">One table for every place, so nothing is approved where the checks are weaker</span></div></div><table class=\"surface-table\"><thead><tr><th scope=\"col\">Where</th><th scope=\"col\">May decide</th><th scope=\"col\">Never</th></tr></thead><tbody>");
        for row in agentrix_decisions::surface::TABLE {
            html.push_str(&format!(
                "<tr><td><b>{}</b></td><td>{}</td><td>{}</td></tr>",
                escape(row.surface),
                escape(row.may),
                escape(row.never)
            ));
        }
        html.push_str("<tr><td><b>Telegram</b></td><td>Decline any card or pause a WebAgent. STOP freezes every WebAgent</td><td>Approve anything or unfreeze: approving opens the card here</td></tr>");
        html.push_str("</tbody></table></article></div>");
        Ok(AppPage::Field {
            title: "Channels".into(),
            html,
        })
    }

    async fn post(
        &self,
        ctx: &AppContext,
        path: &str,
        _headers: &HeaderMap,
        body: Bytes,
    ) -> anyhow::Result<AppReply> {
        let ch = &self.channels;
        let form: HashMap<String, String> =
            url::form_urlencoded::parse(&body).into_owned().collect();
        let back = safe_back(form.get("back"));
        match path {
            "telegram/link" => {
                if ch.bot_for(&ctx.site_id).is_none() {
                    return Ok(AppReply::Error(
                        axum::http::StatusCode::CONFLICT,
                        "Telegram isn't connected for this site yet.".into(),
                    ));
                }
                let now = Utc::now();
                let code = ch
                    .store()
                    .new_link_code(TELEGRAM, &ctx.account, &ctx.site_id, LINK_CODE_TTL, now)
                    .await?;
                self.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(
                        (ctx.account.clone(), ctx.site_id.clone()),
                        (code, now + LINK_CODE_TTL),
                    );
                Ok(AppReply::Redirect(with_back(&ctx.base, back.as_deref())))
            }
            "telegram/unlink" => {
                if let Some(link) = ch
                    .store()
                    .link_for_owner(TELEGRAM, &ctx.site_id, &ctx.account)
                    .await?
                    && ch.store().unlink(&link.id, "settings", Utc::now()).await?
                {
                    ch.receipt_channel(&ctx.site_id, &ctx.account, "unlinked", "settings")
                        .await;
                    if let (Some(bot), Ok(chat)) =
                        (ch.bot_for(&ctx.site_id), link.chat_id.parse::<i64>())
                    {
                        let _ = crate::bot::send(
                            bot,
                            chat,
                            "You unlinked this chat in Settings → Channels. Nothing more comes here.",
                            &Vec::new(),
                        )
                        .await;
                    }
                }
                Ok(AppReply::Redirect(with_back(
                    &format!("{}?done=unlinked", ctx.base),
                    back.as_deref(),
                )))
            }
            "app/pair" => {
                let now = Utc::now();
                let code = ch
                    .store()
                    .new_link_code("app", &ctx.account, &ctx.site_id, PAIR_CODE_TTL, now)
                    .await?;
                self.pairing
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(
                        (ctx.account.clone(), ctx.site_id.clone()),
                        (code, now + PAIR_CODE_TTL),
                    );
                Ok(AppReply::Redirect(with_back(
                    &format!("{}?connect=app", ctx.base),
                    back.as_deref(),
                )))
            }
            "app/remove" => {
                if let Some(id) = form.get("device") {
                    ch.store()
                        .remove_device(id, &ctx.account, Utc::now())
                        .await?;
                }
                Ok(AppReply::Redirect(with_back(
                    &format!("{}?done=removed", ctx.base),
                    back.as_deref(),
                )))
            }
            _ => Ok(AppReply::NotFound),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_is_a_path_on_this_origin_only() {
        let b = |s: &str| safe_back(Some(&s.to_owned()));
        assert_eq!(b("/sites/s1/move"), Some("/sites/s1/move".into()));
        assert_eq!(b("//evil.example/x"), None);
        assert_eq!(b("https://evil.example/"), None);
        assert_eq!(b("/a\\b"), None);
        assert_eq!(
            with_back(
                "/console/s/apps/channels?done=unlinked",
                Some("/sites/s1/move")
            ),
            "/console/s/apps/channels?done=unlinked&back=%2Fsites%2Fs1%2Fmove"
        );
    }
}
