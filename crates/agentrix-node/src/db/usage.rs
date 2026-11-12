//! Reading model spend (R11.3, S2-11). The Model Gateway (agentrix-models) is the one writer of
//! `usage`; this module only reads it, for the console's meter, cost per Move and the brief.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use agentrix_types::{MoveId, SiteId};

use super::time;

/// Spend over some window, in micro-USD (estimates from the catalogue prices).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Spend {
    /// Total estimated cost.
    pub micro_usd: i64,
    /// Calls that reached a provider or a recording.
    pub calls: i64,
    /// Calls answered from recordings (D10a), which cost nothing.
    pub replayed: i64,
    /// Calls the budget refused.
    pub refused: i64,
    /// Cost by purpose, most expensive first.
    pub by_purpose: Vec<(String, i64)>,
    /// Cost by provider, most expensive first.
    pub by_provider: Vec<(String, i64)>,
}

impl Spend {
    /// Dollars, rounded to the cent.
    pub fn usd(&self) -> f64 {
        (self.micro_usd as f64 / 10_000.0).round() / 100.0
    }

    /// Plain words for the console and the brief, for example
    /// "Models cost $0.42 across 31 calls".
    pub fn words(&self, window: &str) -> String {
        if self.calls == 0 && self.refused == 0 {
            return format!("No model calls {window}.");
        }
        let mut line = format!(
            "Models cost ${:.2} {window} across {} call{}",
            self.usd(),
            self.calls,
            if self.calls == 1 { "" } else { "s" }
        );
        if self.replayed > 0 {
            line.push_str(&format!(", {} of them replayed at no cost", self.replayed));
        }
        if self.refused > 0 {
            line.push_str(&format!("; {} stopped by the budget", self.refused));
        }
        line.push('.');
        line
    }
}

/// A site's spend since `since`.
pub async fn site_since(
    pool: &SqlitePool,
    site: &SiteId,
    since: DateTime<Utc>,
) -> Result<Spend, sqlx::Error> {
    spend(
        pool,
        "site_id = ? AND at >= ?",
        site.as_str(),
        Some(time(since)),
    )
    .await
}

/// What one Move has cost so far (S2-11).
pub async fn for_move(pool: &SqlitePool, move_id: &MoveId) -> Result<Spend, sqlx::Error> {
    spend(pool, "move_id = ?", move_id.as_str(), None).await
}

async fn spend(
    pool: &SqlitePool,
    filter: &'static str,
    key: &str,
    since: Option<String>,
) -> Result<Spend, sqlx::Error> {
    // Two fixed query shapes, chosen by `filter`; nothing from callers reaches the SQL text.
    let (totals, purposes, providers) = match filter {
        "site_id = ? AND at >= ?" => (
            "SELECT COALESCE(SUM(cost_micro_usd), 0), \
             COALESCE(SUM(outcome IN ('ok', 'error', 'replayed')), 0), \
             COALESCE(SUM(outcome = 'replayed'), 0), COALESCE(SUM(outcome = 'over-budget'), 0) \
             FROM usage WHERE site_id = ? AND at >= ?",
            "SELECT purpose, SUM(cost_micro_usd) AS c FROM usage WHERE site_id = ? AND at >= ? \
             GROUP BY purpose ORDER BY c DESC, purpose",
            "SELECT provider, SUM(cost_micro_usd) AS c FROM usage WHERE site_id = ? AND at >= ? \
             GROUP BY provider ORDER BY c DESC, provider",
        ),
        _ => (
            "SELECT COALESCE(SUM(cost_micro_usd), 0), \
             COALESCE(SUM(outcome IN ('ok', 'error', 'replayed')), 0), \
             COALESCE(SUM(outcome = 'replayed'), 0), COALESCE(SUM(outcome = 'over-budget'), 0) \
             FROM usage WHERE move_id = ?",
            "SELECT purpose, SUM(cost_micro_usd) AS c FROM usage WHERE move_id = ? \
             GROUP BY purpose ORDER BY c DESC, purpose",
            "SELECT provider, SUM(cost_micro_usd) AS c FROM usage WHERE move_id = ? \
             GROUP BY provider ORDER BY c DESC, provider",
        ),
    };
    let bind = |q: &'static str| {
        let query = sqlx::query_as::<_, (String, i64)>(q).bind(key.to_owned());
        match &since {
            Some(at) => query.bind(at.clone()),
            None => query,
        }
    };
    let mut totals_query = sqlx::query_as::<_, (i64, i64, i64, i64)>(totals).bind(key.to_owned());
    if let Some(at) = &since {
        totals_query = totals_query.bind(at.clone());
    }
    let (micro_usd, calls, replayed, refused) = totals_query.fetch_one(pool).await?;
    Ok(Spend {
        micro_usd,
        calls,
        replayed,
        refused,
        by_purpose: bind(purposes).fetch_all(pool).await?,
        by_provider: bind(providers).fetch_all(pool).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spend_is_summed_per_site_and_per_move() {
        let pool = super::super::open_in_memory().await.unwrap();
        let now = super::super::now();
        sqlx::query(
            "INSERT INTO sites (site_id, domain, old_origin, created_at) \
             VALUES ('site_1', 'bakery.example', 'https://bakery.example', ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO moves (move_id, site_id, state, created_at, updated_at) \
             VALUES ('move_1', 'site_1', 'claimed', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        for (purpose, provider, cost, outcome, move_id) in [
            ("build", "anthropic", 300_000, "ok", Some("move_1")),
            ("build", "anthropic", 0, "replayed", Some("move_1")),
            ("concierge", "anthropic", 120_000, "ok", None),
            ("embed", "vultr", 1, "ok", None),
            ("build", "anthropic", 0, "over-budget", Some("move_1")),
        ] {
            sqlx::query(
                "INSERT INTO usage (site_id, purpose, provider, model, move_id, tokens_in, \
                 tokens_out, cost_micro_usd, outcome, at) VALUES ('site_1', ?, ?, 'm', ?, 0, 0, ?, ?, ?)",
            )
            .bind(purpose)
            .bind(provider)
            .bind(move_id)
            .bind(cost)
            .bind(outcome)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();
        }
        let site = site_since(
            &pool,
            &SiteId::new("site_1"),
            Utc::now() - chrono::Duration::hours(1),
        )
        .await
        .unwrap();
        assert_eq!(
            (site.micro_usd, site.calls, site.replayed, site.refused),
            (420_001, 4, 1, 1)
        );
        assert_eq!(site.by_purpose[0], ("build".to_owned(), 300_000));
        assert_eq!(site.by_provider[0].0, "anthropic");
        assert_eq!(
            site.words("today"),
            "Models cost $0.42 today across 4 calls, 1 of them replayed at no cost; 1 stopped by the budget."
        );
        let mv = for_move(&pool, &MoveId::new("move_1")).await.unwrap();
        assert_eq!((mv.micro_usd, mv.calls), (300_000, 2));
        let none = for_move(&pool, &MoveId::new("move_9")).await.unwrap();
        assert_eq!(none.words("on this Move"), "No model calls on this Move.");
    }
}
