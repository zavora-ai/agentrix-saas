//! A small Telegram Bot API client: only the methods the channel uses.
//!
//! Every Bot API URL carries the token, so errors keep the method name and Telegram's own
//! description and drop the URL (`reqwest::Error::without_url`). Nothing here logs.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::Token;

/// Telegram's API base.
pub const API: &str = "https://api.telegram.org";

/// One update from `getUpdates` or a webhook.
#[derive(Debug, Clone, Deserialize)]
pub struct Update {
    /// Increasing id; the next poll asks for the one after.
    pub update_id: i64,
    /// A new message.
    #[serde(default)]
    pub message: Option<Message>,
    /// A tap on an inline button.
    #[serde(default)]
    pub callback_query: Option<CallbackQuery>,
}

/// A message.
#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    /// Its id in the chat.
    pub message_id: i64,
    /// The chat.
    pub chat: Chat,
    /// Who sent it.
    #[serde(default)]
    pub from: Option<User>,
    /// Its text, for text messages.
    #[serde(default)]
    pub text: Option<String>,
}

/// A chat.
#[derive(Debug, Clone, Deserialize)]
pub struct Chat {
    /// Its id.
    pub id: i64,
    /// `private`, `group`, `supergroup` or `channel`.
    #[serde(rename = "type")]
    pub kind: String,
}

/// A Telegram user.
#[derive(Debug, Clone, Deserialize)]
pub struct User {
    /// Their id.
    pub id: i64,
    /// Whether it's a bot.
    #[serde(default)]
    pub is_bot: bool,
    /// Their first name.
    #[serde(default)]
    pub first_name: String,
    /// Their @username, if they have one.
    #[serde(default)]
    pub username: Option<String>,
}

impl User {
    /// "@handle", else the first name.
    pub fn label(&self) -> String {
        match &self.username {
            Some(u) if !u.is_empty() => format!("@{u}"),
            _ => self.first_name.clone(),
        }
    }
}

/// A tap on an inline button.
#[derive(Debug, Clone, Deserialize)]
pub struct CallbackQuery {
    /// Answer it with this id.
    pub id: String,
    /// Who tapped.
    pub from: User,
    /// The message the button was on.
    #[serde(default)]
    pub message: Option<Message>,
    /// The button's data.
    #[serde(default)]
    pub data: Option<String>,
}

/// The bot itself (`getMe`).
#[derive(Debug, Clone, Deserialize)]
pub struct Me {
    /// The bot's id.
    pub id: i64,
    /// The bot's @username, without the @.
    pub username: String,
    /// Its display name.
    #[serde(default)]
    pub first_name: String,
}

/// A sent message (only what we keep).
#[derive(Debug, Clone, Deserialize)]
pub struct Sent {
    /// Its id in the chat.
    pub message_id: i64,
}

/// An inline button: a link or a callback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Button {
    /// Its words.
    pub text: String,
    /// A link it opens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Data sent back when tapped (at most 64 bytes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_data: Option<String>,
}

impl Button {
    /// A link button.
    pub fn link(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            url: Some(url.into()),
            callback_data: None,
        }
    }

    /// A callback button.
    pub fn callback(text: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            url: None,
            callback_data: Some(data.into()),
        }
    }
}

/// Rows of inline buttons under a message.
pub type Keyboard = Vec<Vec<Button>>;

/// What went wrong talking to Telegram. Never holds the token or a URL.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TgError {
    /// Telegram answered `ok: false`.
    #[error("Telegram refused {method}: {code} {description}")]
    Api {
        /// The method.
        method: &'static str,
        /// Its error code (400, 403, 409, 429…).
        code: i64,
        /// Telegram's words.
        description: String,
        /// Seconds to wait, for 429.
        retry_after: Option<u64>,
    },
    /// The call didn't complete.
    #[error("couldn't reach Telegram for {method}: {reason}")]
    Network {
        /// The method.
        method: &'static str,
        /// What happened, without the URL.
        reason: String,
    },
}

impl TgError {
    /// Telegram's error code, if it answered.
    pub fn code(&self) -> Option<i64> {
        match self {
            TgError::Api { code, .. } => Some(*code),
            TgError::Network { .. } => None,
        }
    }

    /// Whether Telegram refused a button's link (it won't take `localhost` or bare IPs).
    pub fn bad_button_url(&self) -> bool {
        match self {
            TgError::Api { description, .. } => {
                let d = description.to_ascii_uppercase();
                d.contains("BUTTON_URL_INVALID")
                    || d.contains("WRONG HTTP URL")
                    || d.contains("URL HOST IS EMPTY")
                    || d.contains("UNSUPPORTED URL PROTOCOL")
                    || (d.contains("URL") && d.contains("BUTTON"))
            }
            TgError::Network { .. } => false,
        }
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    ok: bool,
    #[serde(default = "none")]
    result: Option<T>,
    #[serde(default)]
    error_code: Option<i64>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    parameters: Option<Value>,
}

fn none<T>() -> Option<T> {
    None
}

/// The Bot API for one bot.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    token: Token,
    base: String,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client for `token` on Telegram's API.
    pub fn new(http: reqwest::Client, token: Token) -> Self {
        Self::with_base(http, token, API)
    }

    /// A client against another base (tests run a fake Bot API).
    pub fn with_base(http: reqwest::Client, token: Token, base: &str) -> Self {
        Self {
            http,
            token,
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    /// The token, for the webhook secret.
    pub fn token(&self) -> &Token {
        &self.token
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: &'static str,
        body: Value,
        timeout: Duration,
    ) -> Result<T, TgError> {
        let url = format!("{}/bot{}/{method}", self.base, self.token.expose());
        let network = |e: reqwest::Error| TgError::Network {
            method,
            reason: e.without_url().to_string(),
        };
        let response = self
            .http
            .post(url)
            .timeout(timeout)
            .json(&body)
            .send()
            .await
            .map_err(network)?;
        let status = response.status();
        let envelope: Envelope<T> = response.json().await.map_err(|e| TgError::Network {
            method,
            reason: format!("an answer we couldn't read ({status}): {}", e.without_url()),
        })?;
        match (envelope.ok, envelope.result) {
            (true, Some(result)) => Ok(result),
            _ => Err(TgError::Api {
                method,
                code: envelope.error_code.unwrap_or(i64::from(status.as_u16())),
                description: envelope.description.unwrap_or_default(),
                retry_after: envelope
                    .parameters
                    .as_ref()
                    .and_then(|p| p.get("retry_after"))
                    .and_then(Value::as_u64),
            }),
        }
    }

    /// The bot's own details.
    ///
    /// # Errors
    ///
    /// When Telegram can't be reached or refuses the token.
    pub async fn get_me(&self) -> Result<Me, TgError> {
        self.call("getMe", json!({}), Duration::from_secs(15)).await
    }

    /// Waits up to `wait` seconds for updates after `offset`.
    ///
    /// # Errors
    ///
    /// When Telegram can't be reached; 409 when another process polls or a webhook is set.
    pub async fn get_updates(&self, offset: i64, wait: u64) -> Result<Vec<Update>, TgError> {
        self.call(
            "getUpdates",
            json!({ "offset": offset, "timeout": wait, "allowed_updates": ["message", "callback_query"] }),
            Duration::from_secs(wait + 15),
        )
        .await
    }

    /// Sends `html` (Telegram's HTML subset) with optional buttons. Link previews are off.
    ///
    /// # Errors
    ///
    /// When Telegram refuses or can't be reached. 403 means the person blocked the bot.
    pub async fn send_message(
        &self,
        chat: i64,
        html: &str,
        keyboard: &Keyboard,
    ) -> Result<Sent, TgError> {
        let mut body = json!({
            "chat_id": chat,
            "text": html,
            "parse_mode": "HTML",
            "link_preview_options": { "is_disabled": true },
        });
        if !keyboard.is_empty() {
            body["reply_markup"] = json!({ "inline_keyboard": keyboard });
        }
        self.call("sendMessage", body, Duration::from_secs(20))
            .await
    }

    /// Replaces a message's text and buttons.
    ///
    /// # Errors
    ///
    /// When Telegram refuses (for example, the text didn't change) or can't be reached.
    pub async fn edit_message(
        &self,
        chat: i64,
        message: i64,
        html: &str,
        keyboard: &Keyboard,
    ) -> Result<Value, TgError> {
        let mut body = json!({
            "chat_id": chat,
            "message_id": message,
            "text": html,
            "parse_mode": "HTML",
            "link_preview_options": { "is_disabled": true },
        });
        body["reply_markup"] = json!({ "inline_keyboard": keyboard });
        self.call("editMessageText", body, Duration::from_secs(20))
            .await
    }

    /// Stops the button's spinner, with an optional short notice.
    ///
    /// # Errors
    ///
    /// When Telegram refuses or can't be reached.
    pub async fn answer_callback(&self, id: &str, text: Option<&str>) -> Result<bool, TgError> {
        let mut body = json!({ "callback_query_id": id });
        if let Some(t) = text {
            body["text"] = json!(t);
        }
        self.call("answerCallbackQuery", body, Duration::from_secs(15))
            .await
    }

    /// Sets the command menu for private chats.
    ///
    /// # Errors
    ///
    /// When Telegram refuses or can't be reached.
    pub async fn set_commands(&self, commands: &[(&str, &str)]) -> Result<bool, TgError> {
        let list: Vec<Value> = commands
            .iter()
            .map(|(c, d)| json!({ "command": c, "description": d }))
            .collect();
        self.call(
            "setMyCommands",
            json!({ "commands": list, "scope": { "type": "all_private_chats" } }),
            Duration::from_secs(15),
        )
        .await
    }

    /// Points Telegram at `url` with `secret` in each call's header.
    ///
    /// # Errors
    ///
    /// When Telegram refuses (it needs public HTTPS) or can't be reached.
    pub async fn set_webhook(&self, url: &str, secret: &str) -> Result<bool, TgError> {
        self.call(
            "setWebhook",
            json!({ "url": url, "secret_token": secret, "allowed_updates": ["message", "callback_query"], "drop_pending_updates": false }),
            Duration::from_secs(15),
        )
        .await
    }

    /// Removes any webhook, so polling works.
    ///
    /// # Errors
    ///
    /// When Telegram refuses or can't be reached.
    pub async fn delete_webhook(&self) -> Result<bool, TgError> {
        self.call(
            "deleteWebhook",
            json!({ "drop_pending_updates": false }),
            Duration::from_secs(15),
        )
        .await
    }
}

/// Escapes text for Telegram's HTML mode.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Turns W9's chat text (WhatsApp style: `*bold*` lines) and Ask's phrases (`**bold**`) into
/// Telegram HTML, escaping everything else.
pub fn chat_to_html(text: &str) -> String {
    let escaped = escape(text);
    let mut out = String::with_capacity(escaped.len());
    for (i, line) in escaped.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&bold_spans(line, "**"));
    }
    let mut final_out = String::with_capacity(out.len());
    for (i, line) in out.split('\n').enumerate() {
        if i > 0 {
            final_out.push('\n');
        }
        final_out.push_str(&bold_spans(line, "*"));
    }
    final_out
}

/// Replaces `{mark}x{mark}` pairs on one line with `<b>x</b>`; an unpaired mark stays as it is.
fn bold_spans(line: &str, mark: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        let Some(start) = rest.find(mark) else {
            out.push_str(rest);
            return out;
        };
        let after = &rest[start + mark.len()..];
        match after.find(mark) {
            Some(end)
                if end > 0 && !after[..end].starts_with(' ') && !after[..end].ends_with(' ') =>
            {
                out.push_str(&rest[..start]);
                out.push_str("<b>");
                out.push_str(&after[..end]);
                out.push_str("</b>");
                rest = &after[end + mark.len()..];
            }
            _ => {
                out.push_str(&rest[..start + mark.len()]);
                rest = after;
            }
        }
    }
}

/// Whether Telegram will accept `url` on a button: public http(s) hosts only, never localhost,
/// `*.localhost` or a bare IP address.
pub fn button_url_ok(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    if !matches!(u.scheme(), "https" | "http") {
        return false;
    }
    match u.host() {
        Some(url::Host::Domain(d)) => {
            let d = d.to_ascii_lowercase();
            d.contains('.')
                && d != "localhost"
                && !d.ends_with(".localhost")
                && !d.ends_with(".test")
        }
        _ => false,
    }
}

/// Keeps `keyboard`'s callback buttons and moves link buttons Telegram would refuse into the
/// text as plain addresses (local runs serve `localhost`). Returns the new text and keyboard.
pub fn fit_links(html: &str, keyboard: &Keyboard) -> (String, Keyboard) {
    let mut text = html.to_owned();
    let mut rows = Vec::new();
    for row in keyboard {
        let mut kept = Vec::new();
        for b in row {
            match &b.url {
                Some(url) if !button_url_ok(url) => {
                    text.push_str(&format!("\n\n{}: {}", escape(&b.text), escape(url)));
                }
                _ => kept.push(b.clone()),
            }
        }
        if !kept.is_empty() {
            rows.push(kept);
        }
    }
    (text, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whatsapp_and_ask_bold_become_html_and_the_rest_is_escaped() {
        assert_eq!(
            chat_to_html("*Good morning, Wanjiru.*\nA <b> & **KSh 4,500** [1]"),
            "<b>Good morning, Wanjiru.</b>\nA &lt;b&gt; &amp; <b>KSh 4,500</b> [1]"
        );
        assert_eq!(chat_to_html("2 * 3 = 6"), "2 * 3 = 6");
    }

    #[test]
    fn telegram_never_gets_a_localhost_button() {
        assert!(button_url_ok("https://agentrix.website/d/dec2_1"));
        assert!(!button_url_ok("http://localhost:18050/d/dec2_1"));
        assert!(!button_url_ok(
            "http://kilele-bakery-sample.localhost:18050/cakes"
        ));
        assert!(!button_url_ok("http://127.0.0.1:18050/"));
        let (text, kb) = fit_links(
            "Hi",
            &vec![vec![
                Button::link("Open in the console", "http://localhost:18050/d/dec2_1"),
                Button::callback("Decline", "dec:dec2_1"),
            ]],
        );
        assert!(text.contains("Open in the console: http://localhost:18050/d/dec2_1"));
        assert_eq!(kb, vec![vec![Button::callback("Decline", "dec:dec2_1")]]);
    }

    #[test]
    fn errors_never_carry_the_token() {
        let e = TgError::Api {
            method: "sendMessage",
            code: 400,
            description: "Bad Request: BUTTON_URL_INVALID".into(),
            retry_after: None,
        };
        assert!(e.bad_button_url());
        assert!(!e.to_string().contains("bot"));
    }
}
