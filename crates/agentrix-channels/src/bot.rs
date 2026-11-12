//! What a Telegram update does: link an owner, answer an owner, or answer a customer.
//!
//! A private chat linked to the bot's site is the owner's (13 §4.4); any other private chat is a
//! customer (13 §4.5). Groups get one line and nothing else (BotFather's join setting keeps the
//! bot out of them anyway).

use std::sync::Arc;

use agentrix_media::intent::{self, Intent};
use chrono::{Duration, Utc};
use serde_json::json;

use crate::rules::{self, Answer};
use crate::service::{Bot, CardView, Channels, TELEGRAM};
use crate::store::Link;
use crate::telegram::{
    Button, CallbackQuery, Keyboard, Message, TgError, Update, chat_to_html, escape, fit_links,
};

/// Messages a customer may send a minute.
pub const PER_MINUTE: i64 = 20;

/// The owner's command menu (and the customer's, which only uses `start`, `help` and `forget`).
pub const COMMANDS: &[(&str, &str)] = &[
    ("brief", "The latest brief from your Chief WebAgent"),
    ("decisions", "Decisions waiting for you"),
    ("freeze", "Freeze every WebAgent now"),
    ("pause", "Set one WebAgent to Watch: /pause Sales"),
    (
        "site",
        "Ask your site as a customer would: /site cakes for 10",
    ),
    ("help", "What you can do here"),
    ("forget", "Delete the messages saved from this chat"),
    ("stop", "Unlink this chat"),
];

/// Sends `html` with `keyboard`, moving link buttons Telegram won't take (local `localhost`
/// addresses) into the text. Returns the message id.
///
/// # Errors
///
/// What Telegram said (403 when the person blocked the bot).
pub async fn send_raw(
    bot: &Bot,
    chat: i64,
    html: &str,
    keyboard: &Keyboard,
) -> Result<i64, TgError> {
    let (text, kb) = fit_links(html, keyboard);
    match bot.client.send_message(chat, &text, &kb).await {
        Ok(sent) => Ok(sent.message_id),
        Err(e) if e.bad_button_url() => {
            let mut plain = text.clone();
            for b in kb.iter().flatten() {
                if let Some(u) = &b.url {
                    plain.push_str(&format!("\n\n{}: {}", escape(&b.text), escape(u)));
                }
            }
            let callbacks: Keyboard = kb
                .iter()
                .map(|r| {
                    r.iter()
                        .filter(|b| b.url.is_none())
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .filter(|r| !r.is_empty())
                .collect();
            bot.client
                .send_message(chat, &plain, &callbacks)
                .await
                .map(|s| s.message_id)
        }
        Err(e) => Err(e),
    }
}

/// [`send_raw`], logging a refusal instead of returning it.
pub async fn send(bot: &Bot, chat: i64, html: &str, keyboard: &Keyboard) -> Option<i64> {
    send_raw(bot, chat, html, keyboard)
        .await
        .map_err(|e| tracing::warn!(error = %e, "Telegram didn't take a message"))
        .ok()
}

/// Handles one update for `bot`. Errors are logged; the person always gets a line back.
pub async fn handle(channels: &Channels, bot: &Arc<Bot>, update: Update) {
    if let Some(cb) = update.callback_query {
        callback(channels, bot, cb).await;
        return;
    }
    let Some(msg) = update.message else {
        return;
    };
    if msg.from.as_ref().is_some_and(|u| u.is_bot) {
        return;
    }
    if msg.chat.kind != "private" {
        let _ = send(
            bot,
            msg.chat.id,
            "I only talk one to one. Open a private chat with me.",
            &Vec::new(),
        )
        .await;
        return;
    }
    let Some(text) = msg.text.clone() else {
        let _ = send(
            bot,
            msg.chat.id,
            "I can only read text. Send it as a typed message.",
            &Vec::new(),
        )
        .await;
        return;
    };
    let text = text.trim().to_owned();
    let chat = msg.chat.id.to_string();
    let link = channels
        .store()
        .link_for_chat(TELEGRAM, &bot.cfg.site, &chat)
        .await
        .unwrap_or(None);

    // `/start <code>` links an owner, whoever this chat was before.
    if let Some(rest) = command(&text, "start") {
        let code = rest.trim();
        if !code.is_empty() {
            link_owner(channels, bot, &msg, code).await;
            return;
        }
    }
    match link {
        Some(link) => owner_text(channels, bot, &link, &msg, &text).await,
        None => customer_text(channels, bot, &msg, &text).await,
    }
}

/// `/name rest` (or `/name@bot rest`) → `Some(rest)`.
fn command<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let t = text.strip_prefix('/')?;
    let (head, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
    let head = head.split('@').next().unwrap_or(head);
    head.eq_ignore_ascii_case(name).then_some(rest)
}

fn is_stop_word(text: &str) -> bool {
    let t: String = text
        .trim()
        .trim_end_matches(['.', '!'])
        .to_ascii_lowercase();
    t == "stop" || command(text, "freeze").is_some()
}

async fn link_owner(channels: &Channels, bot: &Arc<Bot>, msg: &Message, code: &str) {
    let now = Utc::now();
    let taken = channels
        .store()
        .take_link_code(TELEGRAM, code, now)
        .await
        .unwrap_or(None);
    let Some((account, site)) = taken.filter(|(_, s)| s == &bot.cfg.site) else {
        let _ = send(
            bot,
            msg.chat.id,
            "That link has expired or was already used. Make a new one in Settings → Channels.",
            &Vec::new(),
        )
        .await;
        return;
    };
    let label = msg.from.as_ref().map(|u| u.label()).unwrap_or_default();
    let link = match channels
        .store()
        .link(
            TELEGRAM,
            &bot.cfg.site,
            &msg.chat.id.to_string(),
            &account,
            &site,
            &label,
            now,
        )
        .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, "couldn't link a chat");
            let _ = send(bot, msg.chat.id, "I couldn't link this chat just now. Nothing changed. Make a new link in Settings → Channels and open it again.", &Vec::new()).await;
            return;
        }
    };
    channels
        .receipt_channel(&site, &account, "linked", "telegram")
        .await;
    let info = channels.site(&site).await;
    let first = channels.owner_first_name(&account).await;
    // Cards already waiting are listed by /decisions; alerts are for new ones.
    for c in channels.waiting_cards(&site).await {
        let _ = channels
            .store()
            .mark_alert(&link.id, "card", &c.id, now)
            .await;
    }
    let hello = format!(
        "<b>Linked{}.</b> This is now your owner chat for {}{}.\n\nYou'll get the morning brief, a note when a decision is waiting, and questions your desks pass on. From here you can decline, pause a WebAgent, or send <b>STOP</b> to freeze every WebAgent. To approve, open the console and use your passkey or authenticator code.\n\nUnlink any time with /stop or in Settings → Channels.",
        first
            .map(|n| format!(", {}", escape(&n)))
            .unwrap_or_default(),
        escape(&info.name),
        if info.sample { " (Sample)" } else { "" },
    );
    let _ = send(
        bot,
        msg.chat.id,
        &hello,
        &vec![vec![Button::link(
            "Open the console",
            channels.console_url(&site),
        )]],
    )
    .await;
    // The latest brief right away, and today's counts as sent.
    let today = crate::watch::local_day(Utc::now(), info.offset_min);
    let _ = channels
        .store()
        .mark_alert(&link.id, "brief", &today, now)
        .await;
    let brief = channels.brief_json(&site).await;
    if let Err(e) = channels
        .notify(
            &link,
            "owner.morning_brief",
            json!({ "brief": brief }),
            "You linked this Telegram chat; this is the latest brief",
            Some(format!("tg:{}:brief:link", link.id)),
        )
        .await
    {
        tracing::warn!(error = %format!("{e:#}"), "the first brief didn't go");
    }
}

async fn owner_text(channels: &Channels, bot: &Arc<Bot>, link: &Link, msg: &Message, text: &str) {
    let chat = msg.chat.id;
    let site = link.site_id.clone();
    let reply: (String, Keyboard) = if command(text, "stop").is_some() {
        unlink(channels, link, "chat").await;
        (
            "This chat is unlinked. Nothing more comes here, and your WebAgents keep working. Link again in Settings → Channels.".into(),
            Vec::new(),
        )
    } else if is_stop_word(text) || intent::read(text) == Intent::Stop {
        stop_all(channels, link).await
    } else if command(text, "brief").is_some()
        || (!text.starts_with('/') && intent::read(text) == Intent::Brief)
    {
        let brief = channels.brief_json(&site).await;
        if let Err(e) = channels
            .notify(
                link,
                "owner.morning_brief",
                json!({ "brief": brief }),
                "You asked for the brief in Telegram",
                None,
            )
            .await
        {
            tracing::warn!(error = %format!("{e:#}"), "the brief didn't go");
            (
                "I couldn't send the brief just now. Open the console to read it.".into(),
                Vec::new(),
            )
        } else {
            return;
        }
    } else if command(text, "decisions").is_some() || intent::read(text) == Intent::Next {
        decisions_list(channels, bot, link, chat).await;
        return;
    } else if let Some(rest) = command(text, "pause") {
        pause(channels, link, rest.trim()).await
    } else if let Some(rest) = command(text, "site") {
        if rest.trim().is_empty() {
            (
                "Add the question: /site do you have cakes for 10 people?".into(),
                Vec::new(),
            )
        } else {
            let info = channels.site(&site).await;
            let (html, kb, _) = crate::customer::answer(
                channels,
                bot,
                &info,
                rest.trim(),
                &format!("owner-{chat}"),
            )
            .await;
            (format!("<i>What a customer would see:</i>\n\n{html}"), kb)
        }
    } else if command(text, "help").is_some() || command(text, "start").is_some() {
        (owner_help(), Vec::new())
    } else if command(text, "forget").is_some() {
        let n = channels
            .store()
            .forget_chat(TELEGRAM, &bot.cfg.site, &chat.to_string())
            .await
            .unwrap_or(0);
        (
            format!("Deleted the messages saved from this chat: {n} of yours, with the replies."),
            Vec::new(),
        )
    } else if text.starts_with('/') {
        (
            format!("I don't know that command. {}", owner_help()),
            Vec::new(),
        )
    } else {
        match intent::read(text) {
            Intent::AllowOnce => approve_elsewhere(channels, link).await,
            Intent::Deny => decline_by_words(channels, link, text).await,
            Intent::Cancel => (
                "There's nothing to cancel here. To turn down a decision, send /decisions and tap Decline.".into(),
                Vec::new(),
            ),
            _ => {
                let answer = crate::chief::answer(channels, &site, text).await;
                (escape(&answer), Vec::new())
            }
        }
    };
    let out = send(bot, chat, &reply.0, &reply.1).await;
    let _ = channels
        .store()
        .log_turn(
            TELEGRAM,
            &bot.cfg.site,
            &chat.to_string(),
            "owner",
            None,
            text,
            &reply.0,
            Some(msg.message_id),
            out,
            Utc::now(),
        )
        .await;
}

fn owner_help() -> String {
    "Here you can:\n• /brief: the latest brief\n• /decisions: what's waiting, with buttons to open or decline each one\n• STOP or /freeze: freeze every WebAgent now\n• /pause Sales: set one WebAgent to Watch\n• /site and a question: see what a customer would get\n• anything else: ask your Chief WebAgent\n\nTo approve, open the console and use your passkey or authenticator code. /stop unlinks this chat.".into()
}

async fn unlink(channels: &Channels, link: &Link, how: &str) {
    if channels
        .store()
        .unlink(&link.id, how, Utc::now())
        .await
        .unwrap_or(false)
    {
        channels
            .receipt_channel(&link.site_id, &link.account_id, "unlinked", how)
            .await;
    }
}

async fn stop_all(channels: &Channels, link: &Link) -> (String, Keyboard) {
    if !rules::chat_may_freeze() {
        return (
            "This chat can't stop everything. Use Freeze in the console.".into(),
            Vec::new(),
        );
    }
    match channels.freeze(&link.site_id, &link.account_id).await {
        Ok(true) => (
            "<b>Every WebAgent is frozen.</b> Your site keeps serving its signed release; nothing is sent, published or paid. Unfreezing needs your passkey or authenticator code in the console.".into(),
            vec![vec![Button::link("Open the console", channels.console_url(&link.site_id))]],
        ),
        Ok(false) => (
            "Freeze isn't switched on for this site yet, so nothing was stopped. To stop one WebAgent now, send /pause and its name.".into(),
            vec![vec![Button::link("Open the console", channels.console_url(&link.site_id))]],
        ),
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "freeze from chat failed");
            ("I couldn't freeze your WebAgents just now. Use Freeze in the console.".into(), vec![vec![Button::link("Open the console", channels.console_url(&link.site_id))]])
        }
    }
}

async fn approve_elsewhere(channels: &Channels, link: &Link) -> (String, Keyboard) {
    let cards = channels.waiting_cards(&link.site_id).await;
    match cards.first() {
        None => ("Nothing is waiting for you to decide.".into(), Vec::new()),
        Some(c) => (
            format!(
                "Approving needs your passkey or an authenticator code, so it happens in the console, never in chat. <b>{}</b> opens there:",
                escape(&c.title)
            ),
            vec![vec![Button::link(
                "Open in the console",
                channels.card_url(&c.id),
            )]],
        ),
    }
}

async fn decline_by_words(channels: &Channels, link: &Link, heard: &str) -> (String, Keyboard) {
    let cards: Vec<CardView> = channels
        .waiting_cards(&link.site_id)
        .await
        .into_iter()
        .filter(|c| c.v2)
        .collect();
    match cards.as_slice() {
        [] => ("Nothing is waiting that I can decline here.".into(), Vec::new()),
        [only] => decline_card(channels, link, &only.id, heard).await,
        _ => (
            "More than one decision is waiting. Tap Decline under the one you mean (/decisions lists them).".into(),
            Vec::new(),
        ),
    }
}

async fn decline_card(
    channels: &Channels,
    link: &Link,
    card: &str,
    heard: &str,
) -> (String, Keyboard) {
    match channels
        .decline(&link.site_id, &link.account_id, card, heard)
        .await
    {
        Ok(receipt) => (
            format!(
                "Declined. Nothing changed. Receipt {}.",
                escape(&short(&receipt))
            ),
            Vec::new(),
        ),
        Err(words) => (escape(&words), Vec::new()),
    }
}

fn short(receipt: &str) -> String {
    let hex = receipt.trim_start_matches("sha256:");
    if hex.len() > 10 {
        format!("r-{}…{}", &hex[..4], &hex[hex.len() - 2..])
    } else {
        hex.to_owned()
    }
}

/// The buttons a chat may show for a card (the rule table).
pub fn card_buttons(channels: &Channels, card: &CardView) -> Keyboard {
    let mut row = Vec::new();
    for a in rules::chat_answers(card.powers) {
        match a {
            Answer::Open => row.push(Button::link(
                "Open in the console",
                channels.card_url(&card.id),
            )),
            Answer::Decline if card.v2 => {
                row.push(Button::callback("Decline", format!("dec:{}", card.id)))
            }
            _ => {}
        }
    }
    vec![row]
}

async fn decisions_list(channels: &Channels, bot: &Arc<Bot>, link: &Link, chat: i64) {
    let cards = channels.waiting_cards(&link.site_id).await;
    if cards.is_empty() {
        let _ = send(
            bot,
            chat,
            "Nothing is waiting for you to decide.",
            &Vec::new(),
        )
        .await;
        return;
    }
    let info = channels.site(&link.site_id).await;
    let n = cards.len();
    let _ = send(
        bot,
        chat,
        &format!(
            "{} waiting:",
            if n == 1 {
                "1 decision is".to_owned()
            } else {
                format!("{n} decisions are")
            }
        ),
        &Vec::new(),
    )
    .await;
    for c in cards.iter().take(5) {
        let why = if c.money {
            "It moves money, so it needs the console."
        } else if c.risk == "high" {
            "It's high risk, so it needs the console."
        } else {
            "Approving it needs your passkey or an authenticator code, in the console."
        };
        let html = format!(
            "<b>{}</b>\n{} · {} risk · since {}\n{}",
            escape(&c.title),
            escape(&c.agent),
            capital(&c.risk),
            crate::service::clock(c.created_at, info.offset_min),
            why
        );
        let _ = send(bot, chat, &html, &card_buttons(channels, c)).await;
    }
    if n > 5 {
        let _ = send(
            bot,
            chat,
            &format!("{} more in the console.", n - 5),
            &vec![vec![Button::link(
                "Open the console",
                channels.console_url(&link.site_id),
            )]],
        )
        .await;
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

async fn pause(channels: &Channels, link: &Link, who: &str) -> (String, Keyboard) {
    if who.is_empty() {
        return ("Say which WebAgent: /pause Sales".into(), Vec::new());
    }
    let ctx = channels.ctx();
    let rosters = agentrix_kernel::Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
    let roster = match rosters.roster(&link.site_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                "This site has no WebAgents on record yet, so there's no one to pause.".into(),
                Vec::new(),
            );
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "no roster for pause");
            return (
                "I couldn't read your WebAgents just now. Pause one from the console.".into(),
                Vec::new(),
            );
        }
    };
    let wanted = who.trim_start_matches("the ").trim().to_ascii_lowercase();
    let Some(member) = roster.members.iter().find(|m| {
        m.spec.id.eq_ignore_ascii_case(&wanted) || m.spec.name.to_ascii_lowercase() == wanted
    }) else {
        let names: Vec<String> = roster.members.iter().map(|m| m.spec.name.clone()).collect();
        return (
            format!(
                "I can't find a WebAgent called {}. Your WebAgents: {}.",
                escape(who),
                escape(&names.join(", "))
            ),
            Vec::new(),
        );
    };
    let Ok(account) = uuid::Uuid::parse_str(&link.account_id) else {
        return (
            "I couldn't tell who you are. Pause from the console.".into(),
            Vec::new(),
        );
    };
    let dials =
        agentrix_kernel::Dials::new(rosters.clone(), ctx.receipts.clone(), ctx.approvals.clone());
    let mut lowered = 0;
    let mut already = 0;
    for job in &member.spec.jobs {
        match dials
            .lower(
                &link.site_id,
                account,
                &member.spec.id,
                &job.id,
                agentrix_kernel::Level::Watch,
            )
            .await
        {
            Ok(_) => lowered += 1,
            Err(agentrix_kernel::DialError::NotAllowed(_)) => already += 1,
            Err(e) => tracing::warn!(error = %e, job = %job.id, "a dial didn't lower"),
        }
    }
    let name = escape(&member.spec.name);
    if lowered == 0 && already > 0 {
        (
            format!("{name} is already at Watch: it reads and reports, and changes nothing."),
            Vec::new(),
        )
    } else if lowered == 0 {
        (
            format!("I couldn't pause {name} just now. Try the console."),
            Vec::new(),
        )
    } else {
        (
            format!(
                "{name} is at Watch now: it reads and reports, and changes nothing. Raising it again needs your approval in the console."
            ),
            vec![vec![Button::link(
                "Open the console",
                channels.console_url(&link.site_id),
            )]],
        )
    }
}

async fn customer_text(channels: &Channels, bot: &Arc<Bot>, msg: &Message, text: &str) {
    let chat = msg.chat.id;
    let info = channels.site(&bot.cfg.site).await;
    let site_word = format!(
        "{}{}",
        info.name,
        if info.sample { " (Sample)" } else { "" }
    );
    if command(text, "start").is_some() || command(text, "help").is_some() {
        let html = format!(
            "<b>{}</b>\nYou're talking to AI. It never guesses prices, dates or allergens.\n\nThe site's desks answer from its own pages, and the Orders desk can tell you where your order is. Ask a question, or send your order number.\n\n/forget deletes the messages saved from this chat.",
            escape(&site_word)
        );
        let _ = send(
            bot,
            chat,
            &html,
            &vec![vec![Button::link(
                "Open the site",
                channels.site_origin(&info.host),
            )]],
        )
        .await;
        return;
    }
    if command(text, "forget").is_some() {
        let n = channels
            .store()
            .forget_chat(TELEGRAM, &bot.cfg.site, &chat.to_string())
            .await
            .unwrap_or(0);
        let _ = send(
            bot,
            chat,
            &format!("Deleted the messages saved from this chat: {n} of yours, with the replies."),
            &Vec::new(),
        )
        .await;
        return;
    }
    if command(text, "stop").is_some() {
        let _ = send(bot, chat, "There's nothing to stop. This chat only replies when you write to it. To block it, use Telegram's chat menu.", &Vec::new()).await;
        return;
    }
    let since = Utc::now() - Duration::minutes(1);
    let recent = channels
        .store()
        .turns_since(TELEGRAM, &bot.cfg.site, &chat.to_string(), since)
        .await
        .unwrap_or(0);
    if recent >= PER_MINUTE {
        let _ = send(
            bot,
            chat,
            "You've sent more messages than this chat answers in a minute. Wait a minute, then ask again.",
            &Vec::new(),
        )
        .await;
        return;
    }
    let q: String = text.chars().take(400).collect();
    let key = format!(
        "tg-{}",
        crate::store::hash_code(&format!("{}:{chat}", bot.cfg.site))
            .get(..16)
            .unwrap_or("x")
    );
    let (html, kb, desk) = crate::customer::answer(channels, bot, &info, &q, &key).await;
    let out = send(bot, chat, &html, &kb).await;
    let _ = channels
        .store()
        .log_turn(
            TELEGRAM,
            &bot.cfg.site,
            &chat.to_string(),
            "customer",
            desk.as_deref(),
            &q,
            &html,
            Some(msg.message_id),
            out,
            Utc::now(),
        )
        .await;
}

async fn callback(channels: &Channels, bot: &Arc<Bot>, cb: CallbackQuery) {
    let data = cb.data.clone().unwrap_or_default();
    let Some(msg) = cb.message.clone() else {
        let _ = bot.client.answer_callback(&cb.id, None).await;
        return;
    };
    let chat = msg.chat.id.to_string();
    if let Some(card) = data.strip_prefix("dec:") {
        let link = channels
            .store()
            .link_for_chat(TELEGRAM, &bot.cfg.site, &chat)
            .await
            .unwrap_or(None);
        let Some(link) = link.filter(|l| cb.from.id.to_string() == l.chat_id) else {
            let _ = bot
                .client
                .answer_callback(&cb.id, Some("Only the linked owner can decline."))
                .await;
            return;
        };
        let (words, _) = decline_card(channels, &link, card, "Decline (button)").await;
        let _ = bot.client.answer_callback(&cb.id, None).await;
        let original = msg.text.clone().unwrap_or_default();
        let edited = format!("{}\n\n{}", escape(&original), words);
        let kept: Keyboard = vec![vec![Button::link(
            "Open in the console",
            channels.card_url(card),
        )]];
        let (edited, kept) = fit_links(&edited, &kept);
        if bot
            .client
            .edit_message(msg.chat.id, msg.message_id, &edited, &kept)
            .await
            .is_err()
        {
            let _ = send(bot, msg.chat.id, &words, &Vec::new()).await;
        }
        let _ = channels
            .store()
            .log_turn(
                TELEGRAM,
                &bot.cfg.site,
                &chat,
                "owner",
                None,
                "Decline (button)",
                &words,
                Some(msg.message_id),
                None,
                Utc::now(),
            )
            .await;
        return;
    }
    if let Some(turn) = data.strip_prefix("pass:") {
        let words = crate::customer::pass(channels, bot, &chat, turn).await;
        let _ = bot.client.answer_callback(&cb.id, None).await;
        let _ = send(bot, msg.chat.id, &escape(&words), &Vec::new()).await;
        return;
    }
    let _ = bot
        .client
        .answer_callback(&cb.id, Some("That button has expired."))
        .await;
}

/// The keyboard for a W9 chat message (the transport): the link button, then the replies the rule
/// table allows. "Approve" and "Allow once" become the link; "Decline" becomes a callback when the
/// link names a card.
pub fn keyboard_for(link: Option<&(String, String)>, replies: &[String]) -> Keyboard {
    let card = link.and_then(|(_, url)| {
        url::Url::parse(url)
            .ok()
            .and_then(|u| u.path().strip_prefix("/d/").map(str::to_owned))
            .filter(|c| !c.is_empty() && !c.contains('/'))
    });
    let mut row = Vec::new();
    if let Some((label, url)) = link {
        let label = if card.is_some() {
            "Open in the console".to_owned()
        } else {
            label.clone()
        };
        row.push(Button::link(label, url.clone()));
    }
    for r in replies {
        let lower = r.to_ascii_lowercase();
        if lower.contains("decline") || lower == "deny" {
            if let Some(c) = &card {
                row.push(Button::callback("Decline", format!("dec:{c}")));
            }
        }
        // Approve, Allow once and "Ask me anything" add no button: approving opens the console
        // (already the link) and a question is just a message.
    }
    if row.is_empty() {
        Vec::new()
    } else {
        vec![row]
    }
}

/// Telegram HTML for a W9 chat text.
pub fn html_for(text: &str) -> String {
    chat_to_html(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_and_stop_words() {
        assert_eq!(command("/start abc", "start"), Some("abc"));
        assert_eq!(command("/start@KileleBot abc", "start"), Some("abc"));
        assert_eq!(command("/brief", "brief"), Some(""));
        assert_eq!(command("brief", "brief"), None);
        assert!(is_stop_word("STOP"));
        assert!(is_stop_word("stop."));
        assert!(is_stop_word("/freeze"));
        assert!(!is_stop_word("/stop"));
        assert!(!is_stop_word("stop the cake order"));
    }

    #[test]
    fn a_decision_message_offers_open_and_decline_never_approve() {
        let kb = keyboard_for(
            Some(&(
                "Open in the console".into(),
                "https://agentrix.website/d/dec2_abc".into(),
            )),
            &["Approve".into(), "Decline".into()],
        );
        assert_eq!(
            kb,
            vec![vec![
                Button::link("Open in the console", "https://agentrix.website/d/dec2_abc"),
                Button::callback("Decline", "dec:dec2_abc"),
            ]]
        );
        let brief = keyboard_for(
            Some(&(
                "Open the console".into(),
                "https://agentrix.website/console/k".into(),
            )),
            &["Ask me anything".into()],
        );
        assert_eq!(
            brief,
            vec![vec![Button::link(
                "Open the console",
                "https://agentrix.website/console/k"
            )]]
        );
    }

    #[test]
    fn the_bot_s_own_words_pass_the_copy_check() {
        let words = [
            owner_help(),
            "That link has expired or was already used. Make a new one in Settings → Channels.".into(),
            "This chat is unlinked. Nothing more comes here, and your WebAgents keep working. Link again in Settings → Channels.".into(),
            "Freeze isn't switched on for this site yet, so nothing was stopped. To stop one WebAgent now, send /pause and its name.".into(),
            "Every WebAgent is frozen. Your site keeps serving its signed release; nothing is sent, published or paid. Unfreezing needs your passkey or authenticator code in the console.".into(),
            "Approving needs your passkey or an authenticator code, so it happens in the console, never in chat.".into(),
            "More than one decision is waiting. Tap Decline under the one you mean (/decisions lists them).".into(),
            "You've sent more messages than this chat answers in a minute. Wait a minute, then ask again.".into(),
            "You're talking to AI. It never guesses prices, dates or allergens.\n\nThe site's desks answer from its own pages, and the Orders desk can tell you where your order is. Ask a question, or send your order number.".into(),
            "There's nothing to stop. This chat only replies when you write to it. To block it, use Telegram's chat menu.".into(),
        ];
        for w in words {
            let found = agentrix_messages::lint::check(&w);
            assert!(found.is_empty(), "{w}: {found:?}");
        }
    }

    #[test]
    fn receipts_read_short() {
        assert_eq!(short("sha256:4c19aabbccddeeff00e7"), "r-4c19…e7");
    }
}
