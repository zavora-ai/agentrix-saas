//! Every site's roster, from what is recorded (`control.db` in Horizon 1).
//!
//! - A **sample** site has a `webagent_sites` row, written by [`Rosters::seed_sample`] when W2
//!   seeds it: the concept's look, the sample's team and its section desks.
//! - A **moved the first release site** has no row. Its look comes from the `--wa-*` tokens in its live
//!   release's home page (the Move extracted them), and its team from charter v1: the Chief,
//!   the Concierge, the Editor and the Guardian on duty, the Mover handed over.
//! - The site's newest **charter** overlays the starting levels, so the dials show what the
//!   owner signed.
//! - The newest owner-approved **spec version** of a WebAgent replaces its template.
//!
//! The first release's tables (`sites`, `site_pointer`, `charters`) are read only, by id, and a missing
//! table reads as "nothing recorded".

use std::sync::Arc;

use agentrix_host::Blobs;
use agentrix_types::Charter;
use anyhow::Context;
use futures::future::BoxFuture;
use sqlx::SqlitePool;

use crate::looks::{self, SiteLook, Tokens};
use crate::roster::{self, DeskDraft, MOVED_SITE_ON_DUTY, Roster, Status};
use crate::spec::WebAgentSpec;

/// Runs agentrix-kernel's migrations (range 0100–0199) with their own table.
pub fn migrate(pool: &SqlitePool) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let mut m = sqlx::migrate!("./migrations");
        m.set_ignore_missing(true);
        m.dangerous_set_table_name("_sqlx_migrations_agentrix_kernel");
        m.run(pool).await.context("agentrix-kernel migrations")?;
        Ok(())
    })
}

/// Reads and writes rosters.
#[derive(Clone)]
pub struct Rosters {
    pool: SqlitePool,
    blobs: Arc<dyn Blobs>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Whether an error is "no such table", which reads as nothing recorded.
fn missing_table(e: &sqlx::Error) -> bool {
    e.to_string().contains("no such table")
}

impl Rosters {
    /// Over node's database and content store.
    pub fn new(pool: SqlitePool, blobs: Arc<dyn Blobs>) -> Self {
        Rosters { pool, blobs }
    }

    /// The database it reads.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Records a sample site's team: the concept's look, its WebAgents and its section desks.
    /// Run by the sample seed (W2); safe to run again.
    pub async fn seed_sample(&self, site_id: &str, slug: &str) -> anyhow::Result<()> {
        let look = looks::sample(slug).with_context(|| format!("no sample called {slug}"))?;
        let (on_duty, desks) = roster::sample_team(slug).context("sample team")?;
        self.set_team(site_id, &look, &on_duty, &desks).await
    }

    /// Records a site's look, who is on duty and its section desks.
    pub async fn set_team(
        &self,
        site_id: &str,
        look: &SiteLook,
        on_duty: &[&str],
        desks: &[DeskDraft],
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO webagent_sites (site_id, look, on_duty, desks, updated_at) VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (site_id) DO UPDATE SET look = excluded.look, on_duty = excluded.on_duty, \
             desks = excluded.desks, updated_at = excluded.updated_at",
        )
        .bind(site_id)
        .bind(serde_json::to_string(look)?)
        .bind(serde_json::to_string(on_duty)?)
        .bind(serde_json::to_string(desks)?)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Saves an owner-approved version of one WebAgent's spec and returns its version.
    pub async fn save_spec(
        &self,
        site_id: &str,
        spec: &WebAgentSpec,
        reason: &str,
        decision_id: Option<&str>,
    ) -> anyhow::Result<u32> {
        spec.mark.validate()?;
        let mut tx = self.pool.begin().await?;
        let (latest,): (Option<i64>,) = sqlx::query_as(
            "SELECT MAX(version) FROM webagent_specs WHERE site_id = ? AND webagent = ?",
        )
        .bind(site_id)
        .bind(&spec.id)
        .fetch_one(&mut *tx)
        .await?;
        let version = latest.unwrap_or(0) + 1;
        sqlx::query(
            "INSERT INTO webagent_specs (site_id, webagent, version, body, reason, decision_id, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(site_id)
        .bind(&spec.id)
        .bind(version)
        .bind(serde_json::to_string(spec)?)
        .bind(reason)
        .bind(decision_id)
        .bind(now())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(u32::try_from(version)?)
    }

    /// Whether a team is recorded for the site (a seeded sample, or a team the owner changed).
    pub async fn has_team(&self, site_id: &str) -> anyhow::Result<bool> {
        match sqlx::query_as::<_, (i64,)>("SELECT 1 FROM webagent_sites WHERE site_id = ?")
            .bind(site_id)
            .fetch_optional(&self.pool)
            .await
        {
            Ok(r) => Ok(r.is_some()),
            Err(e) if missing_table(&e) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Records who owns a site that isn't in the first release's `sites` table (a sample).
    pub async fn set_owner(&self, site_id: &str, account: uuid::Uuid) -> anyhow::Result<()> {
        let done = sqlx::query(
            "UPDATE webagent_sites SET owner_account = ?, updated_at = ? WHERE site_id = ?",
        )
        .bind(account.to_string())
        .bind(now())
        .bind(site_id)
        .execute(&self.pool)
        .await?;
        anyhow::ensure!(
            done.rows_affected() == 1,
            "no team recorded for {site_id}; seed it first"
        );
        Ok(())
    }

    /// Whether `account` owns `site_id`: the first release's `sites.owner_account_id`, or the owner
    /// recorded with the site's team.
    pub async fn owns(&self, site_id: &str, account: uuid::Uuid) -> anyhow::Result<bool> {
        let account = account.to_string();
        for sql in [
            "SELECT 1 FROM sites WHERE site_id = ? AND owner_account_id = ?",
            "SELECT 1 FROM webagent_sites WHERE site_id = ? AND owner_account = ?",
        ] {
            match sqlx::query_as::<_, (i64,)>(sql)
                .bind(site_id)
                .bind(&account)
                .fetch_optional(&self.pool)
                .await
            {
                Ok(Some(_)) => return Ok(true),
                Ok(None) => {}
                Err(e) if missing_table(&e) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(false)
    }

    /// Every site the kernel can build a roster for: the first release's sites and the seeded samples.
    pub async fn site_ids(&self) -> anyhow::Result<Vec<String>> {
        let mut ids: Vec<String> = Vec::new();
        for sql in [
            "SELECT site_id FROM sites ORDER BY site_id",
            "SELECT site_id FROM webagent_sites ORDER BY site_id",
        ] {
            match sqlx::query_as::<_, (String,)>(sql)
                .fetch_all(&self.pool)
                .await
            {
                Ok(rows) => ids.extend(rows.into_iter().map(|(id,)| id)),
                Err(e) if missing_table(&e) => {}
                Err(e) => return Err(e.into()),
            }
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// A site's roster, or `None` when nothing is recorded for the site.
    pub async fn roster(&self, site_id: &str) -> anyhow::Result<Option<Roster>> {
        let team: Option<(String, String, String)> = match sqlx::query_as(
            "SELECT look, on_duty, desks FROM webagent_sites WHERE site_id = ?",
        )
        .bind(site_id)
        .fetch_optional(&self.pool)
        .await
        {
            Ok(row) => row,
            Err(e) if missing_table(&e) => None,
            Err(e) => return Err(e.into()),
        };
        let mut roster = if let Some((look, on_duty, desks)) = team {
            let look: SiteLook = serde_json::from_str(&look).context("a stored look")?;
            let on_duty: Vec<String> = serde_json::from_str(&on_duty).context("a stored team")?;
            let desks: Vec<DeskDraft> = serde_json::from_str(&desks).context("stored desks")?;
            let on_duty: Vec<&str> = on_duty.iter().map(String::as_str).collect();
            Roster::build(site_id, look, &on_duty, &desks)
        } else {
            let Some((name, live)) = self.first_release_site(site_id).await? else {
                return Ok(None);
            };
            let tokens = match &live {
                Some(manifest) => self.live_tokens(manifest).await.unwrap_or_else(|e| {
                    tracing_warn(site_id, &e);
                    None
                }),
                None => None,
            };
            let look = SiteLook::from_tokens(site_id, &name, &tokens.unwrap_or_default());
            let mut r = Roster::build(site_id, look, &MOVED_SITE_ON_DUTY, &[]);
            if live.is_none() {
                // Still moving: the Mover is the one at work.
                if let Some(m) = r.members.iter_mut().find(|m| m.spec.id == "mover") {
                    m.status = Status::OnDuty;
                }
            }
            r
        };
        if let Some(charter) = self.charter(site_id).await? {
            roster.apply_charter(&charter);
        }
        for (id, body) in self.saved_specs(site_id).await? {
            let spec: WebAgentSpec = serde_json::from_str(&body).context("a saved spec")?;
            if let Some(m) = roster.members.iter_mut().find(|m| m.spec.id == id) {
                m.spec = spec;
            }
        }
        Ok(Some(roster))
    }

    /// The first release's record of a site: its name (or domain) and its live manifest, if live.
    async fn first_release_site(
        &self,
        site_id: &str,
    ) -> anyhow::Result<Option<(String, Option<String>)>> {
        let row: Option<(String, String)> =
            match sqlx::query_as("SELECT name, domain FROM sites WHERE site_id = ?")
                .bind(site_id)
                .fetch_optional(&self.pool)
                .await
            {
                Ok(r) => r,
                Err(e) if missing_table(&e) => None,
                Err(e) => return Err(e.into()),
            };
        let Some((name, domain)) = row else {
            return Ok(None);
        };
        let live: Option<(String,)> =
            match sqlx::query_as("SELECT manifest_digest FROM site_pointer WHERE site_id = ?")
                .bind(site_id)
                .fetch_optional(&self.pool)
                .await
            {
                Ok(r) => r,
                Err(e) if missing_table(&e) => None,
                Err(e) => return Err(e.into()),
            };
        let name = if name.trim().is_empty() { domain } else { name };
        Ok(Some((name, live.map(|(d,)| d))))
    }

    /// The tokens on the live release's home page.
    async fn live_tokens(&self, manifest_digest: &str) -> anyhow::Result<Option<Tokens>> {
        let Some(manifest) = self.blobs.get(manifest_digest).await? else {
            return Ok(None);
        };
        let manifest: serde_json::Value = serde_json::from_slice(&manifest)?;
        let Some(home) = manifest["files"]["index.html"]["sha256"].as_str() else {
            return Ok(None);
        };
        let Some(html) = self.blobs.get(home).await? else {
            return Ok(None);
        };
        Ok(looks::tokens_from_html(&String::from_utf8_lossy(&html)))
    }

    async fn charter(&self, site_id: &str) -> anyhow::Result<Option<Charter>> {
        let row: Option<(String,)> = match sqlx::query_as(
            "SELECT body FROM charters WHERE site_id = ? ORDER BY version DESC LIMIT 1",
        )
        .bind(site_id)
        .fetch_optional(&self.pool)
        .await
        {
            Ok(r) => r,
            Err(e) if missing_table(&e) => None,
            Err(e) => return Err(e.into()),
        };
        row.map(|(body,)| serde_json::from_str(&body).context("a stored charter"))
            .transpose()
    }

    async fn saved_specs(&self, site_id: &str) -> anyhow::Result<Vec<(String, String)>> {
        match sqlx::query_as(
            "SELECT s.webagent, s.body FROM webagent_specs s \
             JOIN (SELECT webagent, MAX(version) AS v FROM webagent_specs WHERE site_id = ? GROUP BY webagent) l \
             ON s.webagent = l.webagent AND s.version = l.v WHERE s.site_id = ?",
        )
        .bind(site_id)
        .bind(site_id)
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => Ok(rows),
            Err(e) if missing_table(&e) => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }
}

fn tracing_warn(site_id: &str, e: &anyhow::Error) {
    tracing::warn!(site_id, error = %format!("{e:#}"), "couldn't read the live release's tokens");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::Level;

    async fn rosters() -> (Rosters, agentrix_host::NodeContext) {
        let ctx = agentrix_host::testing::context().await.unwrap();
        migrate(&ctx.pool).await.unwrap();
        (Rosters::new(ctx.pool.clone(), ctx.blobs.clone()), ctx)
    }

    #[tokio::test]
    async fn samples_round_trip() {
        let (r, _ctx) = rosters().await;
        assert!(r.roster("kilele-bakery-sample").await.unwrap().is_none());
        r.seed_sample("kilele-bakery-sample", "kilele")
            .await
            .unwrap();
        r.seed_sample("kilele-bakery-sample", "kilele")
            .await
            .unwrap();
        let k = r.roster("kilele-bakery-sample").await.unwrap().unwrap();
        assert_eq!(k.look.name, "Kilele Bakery");
        assert_eq!(k.desks().len(), 6);
        assert_eq!(
            r.site_ids().await.unwrap(),
            vec!["kilele-bakery-sample".to_string()]
        );
    }

    #[tokio::test]
    async fn a_moved_site_wears_its_tokens_and_its_charter() {
        let (r, ctx) = rosters().await;
        for sql in [
            "CREATE TABLE sites (site_id TEXT PRIMARY KEY, domain TEXT, name TEXT)",
            "CREATE TABLE site_pointer (site_id TEXT PRIMARY KEY, manifest_digest TEXT)",
            "CREATE TABLE charters (site_id TEXT, version INTEGER, body TEXT)",
        ] {
            sqlx::query(sql).execute(&ctx.pool).await.unwrap();
        }
        let home = ctx
            .blobs
            .put(":root { --wa-bg: #fffdf8; --wa-ink: #1f1a14; --wa-accent: #8a3b12; --wa-display: Georgia, serif; }".into())
            .await
            .unwrap();
        let manifest = ctx
            .blobs
            .put(
                serde_json::json!({"files": {"index.html": {"sha256": home}}})
                    .to_string()
                    .into(),
            )
            .await
            .unwrap();
        sqlx::query("INSERT INTO sites VALUES ('site_k1', 'mamaoliech.example', '')")
            .execute(&ctx.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO site_pointer VALUES ('site_k1', ?)")
            .bind(&manifest)
            .execute(&ctx.pool)
            .await
            .unwrap();
        let charter = serde_json::json!({"jobs": [{"webagent": "Editor", "role": "", "job": "Fix typos and out-of-date facts", "level": "do-and-tell", "ceiling": "within-budget", "ceiling_reason": ""}]});
        sqlx::query("INSERT INTO charters VALUES ('site_k1', 1, ?)")
            .bind(charter.to_string())
            .execute(&ctx.pool)
            .await
            .unwrap();

        let k = r.roster("site_k1").await.unwrap().unwrap();
        assert_eq!(k.look.bg, "#fffdf8");
        assert_eq!(k.look.monogram(), "M");
        assert_eq!(
            k.get("editor")
                .unwrap()
                .spec
                .job("freshness")
                .unwrap()
                .start,
            Level::DoAndTell
        );
        assert_eq!(k.get("guardian").unwrap().status, Status::OnDuty);
        assert_eq!(k.get("sales").unwrap().status, Status::Available);
        assert_eq!(k.get("mover").unwrap().status, Status::Done);

        let mut editor = k.get("editor").unwrap().spec.clone();
        editor.mark.signature.seed = 4242;
        assert_eq!(
            r.save_spec(
                "site_k1",
                &editor,
                "New seed from the Workshop",
                Some("d-1")
            )
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            r.save_spec("site_k1", &editor, "Again", None)
                .await
                .unwrap(),
            2
        );
        let k = r.roster("site_k1").await.unwrap().unwrap();
        assert_eq!(k.get("editor").unwrap().spec.mark.signature.seed, 4242);
        assert_eq!(r.site_ids().await.unwrap(), vec!["site_k1".to_string()]);
    }
}
