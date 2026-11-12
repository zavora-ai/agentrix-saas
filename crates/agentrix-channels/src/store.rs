//! The channel tables in `control.db` (migrations 1300–1301): one-time codes, linked chats,
//! poll cursors, what was announced, and chat turns.

use base64::Engine as _;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rand::RngCore as _;
use sha2::{Digest as _, Sha256};
use sqlx::{Row, SqlitePool};

/// How long a link code works.
pub const LINK_CODE_TTL: Duration = Duration::minutes(15);

/// How long chat turns are kept.
pub const TURNS_KEPT_DAYS: i64 = 30;

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn parse(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// SHA-256 of a code, hex: what the table keeps.
pub fn hash_code(code: &str) -> String {
    hex::encode(Sha256::digest(code.trim().as_bytes()))
}

/// A fresh code: 128 random bits, URL-safe (Telegram's start parameter allows `A-Za-z0-9_-`).
pub fn new_code() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// How long a phone pairing code works.
pub const PAIR_CODE_TTL: Duration = Duration::minutes(10);

/// How long a paired phone may go unused before its token stops working.
pub const DEVICE_IDLE_DAYS: i64 = 30;

/// The letters a pairing code uses: no 0/O, 1/I/L or U, so it reads aloud and types cleanly.
const PAIR_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A pairing code a person types: eight letters and digits, shown as `ABCD-EFGH` (about 39 bits,
/// single use, ten minutes).
pub fn new_pair_code() -> String {
    let mut rng = rand::thread_rng();
    let raw: String = (0..8)
        .map(|_| {
            let i = (rng.next_u32() as usize) % PAIR_ALPHABET.len();
            PAIR_ALPHABET[i] as char
        })
        .collect();
    format!("{}-{}", &raw[..4], &raw[4..])
}

/// A code as typed, in the form its hash was taken of: pairing codes ignore case, spaces and
/// dashes; link codes are exact.
pub fn normalise(purpose: &str, code: &str) -> String {
    if purpose == "app" {
        code.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_uppercase())
            .collect()
    } else {
        code.trim().to_owned()
    }
}

/// A paired phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// `dev_…`.
    pub id: String,
    /// The owner's account.
    pub account_id: String,
    /// The site it was paired for.
    pub site_id: String,
    /// The name the app gave ("Wanjiru's iPhone").
    pub name: String,
    /// When it was paired.
    pub created_at: DateTime<Utc>,
    /// When it last called.
    pub last_used_at: DateTime<Utc>,
}

fn device_from(r: &sqlx::sqlite::SqliteRow) -> Result<Device, sqlx::Error> {
    Ok(Device {
        id: r.try_get("id")?,
        account_id: r.try_get("account_id")?,
        site_id: r.try_get("site_id")?,
        name: r.try_get("name")?,
        created_at: parse(&r.try_get::<String, _>("created_at")?).unwrap_or_default(),
        last_used_at: parse(&r.try_get::<String, _>("last_used_at")?).unwrap_or_default(),
    })
}

/// A linked chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// `lnk_…`.
    pub id: String,
    /// `telegram`.
    pub channel: String,
    /// The bot's site key.
    pub bot: String,
    /// The chat's id at the channel.
    pub chat_id: String,
    /// The owner's account.
    pub account_id: String,
    /// The site.
    pub site_id: String,
    /// "@handle" or a first name.
    pub label: String,
    /// When it was linked.
    pub linked_at: DateTime<Utc>,
}

fn link_from(r: &sqlx::sqlite::SqliteRow) -> Result<Link, sqlx::Error> {
    Ok(Link {
        id: r.try_get("id")?,
        channel: r.try_get("channel")?,
        bot: r.try_get("bot")?,
        chat_id: r.try_get("chat_id")?,
        account_id: r.try_get("account_id")?,
        site_id: r.try_get("site_id")?,
        label: r.try_get("label")?,
        linked_at: parse(&r.try_get::<String, _>("linked_at")?).unwrap_or_default(),
    })
}

/// A `channel_links` query: [`link_from`]'s columns, then the rest. sqlx takes only static SQL.
macro_rules! select_links {
    ($rest:literal) => {
        concat!(
            "SELECT id, channel, bot, chat_id, account_id, site_id, label, linked_at FROM channel_links ",
            $rest
        )
    };
}

/// The channel tables.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Over `control.db`.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Makes a one-time code for `purpose` (`telegram` or `app`) bound to the owner and the site,
    /// replacing any unused one, and returns it. Only its hash is stored.
    pub async fn new_link_code(
        &self,
        purpose: &str,
        account: &str,
        site: &str,
        ttl: Duration,
        now: DateTime<Utc>,
    ) -> Result<String, sqlx::Error> {
        let code = if purpose == "app" {
            new_pair_code()
        } else {
            new_code()
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM channel_link_codes WHERE account_id = ? AND site_id = ? AND purpose = ? AND used_at IS NULL",
        )
        .bind(account)
        .bind(site)
        .bind(purpose)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO channel_link_codes (code_hash, purpose, account_id, site_id, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(hash_code(&normalise(purpose, &code)))
        .bind(purpose)
        .bind(account)
        .bind(site)
        .bind(ts(now))
        .bind(ts(now + ttl))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(code)
    }

    /// Uses a code once: `Some((account, site))` when it exists, is unused, unexpired and for
    /// `purpose`; it can't be used again.
    pub async fn take_link_code(
        &self,
        purpose: &str,
        code: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<(String, String)>, sqlx::Error> {
        let row = sqlx::query(
            "UPDATE channel_link_codes SET used_at = ?
             WHERE code_hash = ? AND purpose = ? AND used_at IS NULL AND expires_at > ?
             RETURNING account_id, site_id",
        )
        .bind(ts(now))
        .bind(hash_code(&normalise(purpose, code)))
        .bind(purpose)
        .bind(ts(now))
        .fetch_optional(&self.pool)
        .await?;
        Ok(match row {
            Some(r) => Some((r.try_get("account_id")?, r.try_get("site_id")?)),
            None => None,
        })
    }

    /// Whether the owner has an unused, unexpired code (the Channels page says "waiting for you
    /// to open the link").
    pub async fn pending_code(
        &self,
        purpose: &str,
        account: &str,
        site: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT expires_at FROM channel_link_codes WHERE account_id = ? AND site_id = ? AND purpose = ? AND used_at IS NULL AND expires_at > ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(account)
        .bind(site)
        .bind(purpose)
        .bind(ts(now))
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(t,)| parse(&t)))
    }

    /// Links `chat` to the owner's site, ending any earlier link of that owner on this channel and
    /// any link this chat had on the bot. Returns the new link.
    #[allow(clippy::too_many_arguments)]
    pub async fn link(
        &self,
        channel: &str,
        bot: &str,
        chat: &str,
        account: &str,
        site: &str,
        label: &str,
        now: DateTime<Utc>,
    ) -> Result<Link, sqlx::Error> {
        let id = format!("lnk_{}", uuid::Uuid::now_v7().simple());
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE channel_links SET unlinked_at = ?, unlinked_how = 'replaced'
             WHERE unlinked_at IS NULL AND channel = ? AND ((site_id = ? AND account_id = ?) OR (bot = ? AND chat_id = ?))",
        )
        .bind(ts(now))
        .bind(channel)
        .bind(site)
        .bind(account)
        .bind(bot)
        .bind(chat)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO channel_links (id, channel, bot, chat_id, account_id, site_id, label, linked_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(channel)
        .bind(bot)
        .bind(chat)
        .bind(account)
        .bind(site)
        .bind(label)
        .bind(ts(now))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Link {
            id,
            channel: channel.into(),
            bot: bot.into(),
            chat_id: chat.into(),
            account_id: account.into(),
            site_id: site.into(),
            label: label.into(),
            linked_at: now,
        })
    }

    /// The active link for a chat on a bot.
    pub async fn link_for_chat(
        &self,
        channel: &str,
        bot: &str,
        chat: &str,
    ) -> Result<Option<Link>, sqlx::Error> {
        let row = sqlx::query(select_links!(
            "WHERE channel = ? AND bot = ? AND chat_id = ? AND unlinked_at IS NULL"
        ))
        .bind(channel)
        .bind(bot)
        .bind(chat)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(link_from).transpose()
    }

    /// The owner's active link on a site and channel.
    pub async fn link_for_owner(
        &self,
        channel: &str,
        site: &str,
        account: &str,
    ) -> Result<Option<Link>, sqlx::Error> {
        let row = sqlx::query(select_links!(
            "WHERE channel = ? AND site_id = ? AND account_id = ? AND unlinked_at IS NULL"
        ))
        .bind(channel)
        .bind(site)
        .bind(account)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(link_from).transpose()
    }

    /// Every active link on a site.
    pub async fn links_for_site(&self, site: &str) -> Result<Vec<Link>, sqlx::Error> {
        let rows = sqlx::query(select_links!(
            "WHERE site_id = ? AND unlinked_at IS NULL ORDER BY linked_at"
        ))
        .bind(site)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(link_from).collect()
    }

    /// Ends a link. False if it had already ended.
    pub async fn unlink(
        &self,
        id: &str,
        how: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE channel_links SET unlinked_at = ?, unlinked_how = ? WHERE id = ? AND unlinked_at IS NULL",
        )
        .bind(ts(now))
        .bind(how)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// The next update id to ask a bot for.
    pub async fn cursor(&self, bot: &str) -> Result<i64, sqlx::Error> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT next FROM channel_cursors WHERE bot = ?")
            .bind(bot)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map_or(0, |(n,)| n))
    }

    /// Moves a bot's cursor forward (never back).
    pub async fn set_cursor(
        &self,
        bot: &str,
        next: i64,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO channel_cursors (bot, next, updated_at) VALUES (?, ?, ?)
             ON CONFLICT (bot) DO UPDATE SET next = MAX(next, excluded.next), updated_at = excluded.updated_at",
        )
        .bind(bot)
        .bind(next)
        .bind(ts(now))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Records that `subject` of `kind` was announced on `link`. True the first time only.
    pub async fn mark_alert(
        &self,
        link: &str,
        kind: &str,
        subject: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(
            "INSERT OR IGNORE INTO channel_alerts (link_id, kind, subject, at) VALUES (?, ?, ?, ?)",
        )
        .bind(link)
        .bind(kind)
        .bind(subject)
        .bind(ts(now))
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Forgets an announcement (the send failed before anything left), so the next tick retries.
    pub async fn unmark_alert(
        &self,
        link: &str,
        kind: &str,
        subject: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM channel_alerts WHERE link_id = ? AND kind = ? AND subject = ?")
            .bind(link)
            .bind(kind)
            .bind(subject)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Keeps the outbox row an announcement became.
    pub async fn alert_outbox(
        &self,
        link: &str,
        kind: &str,
        subject: &str,
        outbox: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE channel_alerts SET outbox_id = ? WHERE link_id = ? AND kind = ? AND subject = ?")
            .bind(outbox)
            .bind(link)
            .bind(kind)
            .bind(subject)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Logs a chat turn. The question is cut to 200 characters.
    #[allow(clippy::too_many_arguments)]
    pub async fn log_turn(
        &self,
        channel: &str,
        bot: &str,
        chat: &str,
        role: &str,
        desk: Option<&str>,
        asked: &str,
        answered: &str,
        in_message: Option<i64>,
        out_message: Option<i64>,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO channel_turns (channel, bot, chat_id, role, desk, asked, answered, in_message_id, out_message_id, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(channel)
        .bind(bot)
        .bind(chat)
        .bind(role)
        .bind(desk)
        .bind(asked.chars().take(200).collect::<String>())
        .bind(answered.chars().take(2000).collect::<String>())
        .bind(in_message)
        .bind(out_message)
        .bind(ts(now))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// How many turns a chat had since `since` (the rate limit).
    pub async fn turns_since(
        &self,
        channel: &str,
        bot: &str,
        chat: &str,
        since: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM channel_turns WHERE channel = ? AND bot = ? AND chat_id = ? AND at >= ?",
        )
        .bind(channel)
        .bind(bot)
        .bind(chat)
        .bind(ts(since))
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    /// Deletes a chat's turns (`/forget`). Returns how many.
    pub async fn forget_chat(
        &self,
        channel: &str,
        bot: &str,
        chat: &str,
    ) -> Result<u64, sqlx::Error> {
        let done =
            sqlx::query("DELETE FROM channel_turns WHERE channel = ? AND bot = ? AND chat_id = ?")
                .bind(channel)
                .bind(bot)
                .bind(chat)
                .execute(&self.pool)
                .await?;
        Ok(done.rows_affected())
    }

    /// Pairs a phone: a new device with a fresh token (256 bits), returned once; only its hash is
    /// kept.
    pub async fn add_device(
        &self,
        account: &str,
        site: &str,
        name: &str,
        now: DateTime<Utc>,
    ) -> Result<(Device, String), sqlx::Error> {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token = format!(
            "agxm_{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
        );
        let id = format!("dev_{}", uuid::Uuid::now_v7().simple());
        let name: String = name.trim().chars().take(60).collect();
        let name = if name.is_empty() {
            "A phone".to_owned()
        } else {
            name
        };
        sqlx::query(
            "INSERT INTO channel_devices (id, token_hash, account_id, site_id, name, created_at, last_used_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(hash_code(&token))
        .bind(account)
        .bind(site)
        .bind(&name)
        .bind(ts(now))
        .bind(ts(now))
        .execute(&self.pool)
        .await?;
        Ok((
            Device {
                id,
                account_id: account.into(),
                site_id: site.into(),
                name,
                created_at: now,
                last_used_at: now,
            },
            token,
        ))
    }

    /// The device a bearer token belongs to, if it's still paired and was used in the last
    /// [`DEVICE_IDLE_DAYS`]; records the use.
    pub async fn device_for_token(
        &self,
        token: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<Device>, sqlx::Error> {
        let row = sqlx::query(
            "UPDATE channel_devices SET last_used_at = ?
             WHERE token_hash = ? AND removed_at IS NULL AND last_used_at > ?
             RETURNING id, account_id, site_id, name, created_at, last_used_at",
        )
        .bind(ts(now))
        .bind(hash_code(token))
        .bind(ts(now - Duration::days(DEVICE_IDLE_DAYS)))
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(device_from).transpose()
    }

    /// The owner's paired phones on a site, newest first.
    pub async fn devices(&self, account: &str, site: &str) -> Result<Vec<Device>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, account_id, site_id, name, created_at, last_used_at FROM channel_devices
             WHERE account_id = ? AND site_id = ? AND removed_at IS NULL ORDER BY created_at DESC",
        )
        .bind(account)
        .bind(site)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(device_from).collect()
    }

    /// Removes one of the owner's phones. False if it wasn't theirs or was already removed.
    pub async fn remove_device(
        &self,
        id: &str,
        account: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE channel_devices SET removed_at = ? WHERE id = ? AND account_id = ? AND removed_at IS NULL",
        )
        .bind(ts(now))
        .bind(id)
        .bind(account)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Deletes turns older than [`TURNS_KEPT_DAYS`] and spent codes older than a day.
    pub async fn tidy(&self, now: DateTime<Utc>) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM channel_turns WHERE at < ?")
            .bind(ts(now - Duration::days(TURNS_KEPT_DAYS)))
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM channel_link_codes WHERE expires_at < ?")
            .bind(ts(now - Duration::days(1)))
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::migrate(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn a_code_works_once_for_its_purpose_and_not_after_it_expires() {
        let s = Store::new(pool().await);
        let now = Utc::now();
        let code = s
            .new_link_code("telegram", "acc", "site", LINK_CODE_TTL, now)
            .await
            .unwrap();
        assert!(s.take_link_code("app", &code, now).await.unwrap().is_none());
        assert_eq!(
            s.take_link_code("telegram", &code, now).await.unwrap(),
            Some(("acc".into(), "site".into()))
        );
        assert!(
            s.take_link_code("telegram", &code, now)
                .await
                .unwrap()
                .is_none()
        );

        let late = s
            .new_link_code("telegram", "acc", "site", LINK_CODE_TTL, now)
            .await
            .unwrap();
        let after = now + LINK_CODE_TTL + Duration::seconds(1);
        assert!(
            s.take_link_code("telegram", &late, after)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_new_code_replaces_the_unused_one_and_only_hashes_are_kept() {
        let s = Store::new(pool().await);
        let now = Utc::now();
        let first = s
            .new_link_code("telegram", "acc", "site", LINK_CODE_TTL, now)
            .await
            .unwrap();
        let second = s
            .new_link_code("telegram", "acc", "site", LINK_CODE_TTL, now)
            .await
            .unwrap();
        assert!(
            s.take_link_code("telegram", &first, now)
                .await
                .unwrap()
                .is_none()
        );
        let kept: Vec<(String,)> = sqlx::query_as("SELECT code_hash FROM channel_link_codes")
            .fetch_all(s.pool())
            .await
            .unwrap();
        assert!(kept.iter().all(|(h,)| h != &second && h.len() == 64));
    }

    #[tokio::test]
    async fn linking_again_moves_the_link_and_alerts_go_once() {
        let s = Store::new(pool().await);
        let now = Utc::now();
        let a = s
            .link("telegram", "kb", "100", "acc", "site", "@w", now)
            .await
            .unwrap();
        let b = s
            .link("telegram", "kb", "200", "acc", "site", "@w", now)
            .await
            .unwrap();
        assert!(
            s.link_for_chat("telegram", "kb", "100")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            s.link_for_chat("telegram", "kb", "200")
                .await
                .unwrap()
                .unwrap()
                .id,
            b.id
        );
        assert_eq!(s.links_for_site("site").await.unwrap().len(), 1);
        assert!(!s.unlink(&a.id, "chat", now).await.unwrap());
        assert!(s.mark_alert(&b.id, "card", "dec2_1", now).await.unwrap());
        assert!(!s.mark_alert(&b.id, "card", "dec2_1", now).await.unwrap());
        assert!(s.unlink(&b.id, "chat", now).await.unwrap());
        assert!(s.links_for_site("site").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_pairing_code_ignores_case_and_dashes_and_a_token_is_kept_only_as_a_hash() {
        let s = Store::new(pool().await);
        let now = Utc::now();
        let code = s
            .new_link_code("app", "acc", "site", PAIR_CODE_TTL, now)
            .await
            .unwrap();
        assert_eq!(code.len(), 9);
        let typed = code.replace('-', " ").to_lowercase();
        assert_eq!(
            s.take_link_code("app", &typed, now).await.unwrap(),
            Some(("acc".into(), "site".into()))
        );
        let (dev, token) = s
            .add_device("acc", "site", "Wanjiru's iPhone", now)
            .await
            .unwrap();
        assert!(token.starts_with("agxm_") && token.len() > 40);
        assert_eq!(
            s.device_for_token(&token, now).await.unwrap().unwrap().id,
            dev.id
        );
        assert!(
            s.device_for_token("agxm_wrong", now)
                .await
                .unwrap()
                .is_none()
        );
        let late = now + Duration::days(DEVICE_IDLE_DAYS + 1);
        assert!(s.device_for_token(&token, late).await.unwrap().is_none());
        assert!(!s.remove_device(&dev.id, "someone-else", now).await.unwrap());
        assert!(s.remove_device(&dev.id, "acc", now).await.unwrap());
        assert!(s.device_for_token(&token, now).await.unwrap().is_none());
        let kept: Vec<(String,)> = sqlx::query_as("SELECT token_hash FROM channel_devices")
            .fetch_all(s.pool())
            .await
            .unwrap();
        assert!(kept.iter().all(|(h,)| h != &token));
    }

    #[tokio::test]
    async fn the_cursor_only_moves_forward() {
        let s = Store::new(pool().await);
        let now = Utc::now();
        assert_eq!(s.cursor("kb").await.unwrap(), 0);
        s.set_cursor("kb", 10, now).await.unwrap();
        s.set_cursor("kb", 5, now).await.unwrap();
        assert_eq!(s.cursor("kb").await.unwrap(), 10);
    }
}
