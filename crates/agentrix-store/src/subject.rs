//! `control.db`'s personal data about a customer, for W6's "Download my data" and "Delete my
//! account" (ADR-028, 03-data.md §2.4 and §6): one more `SubjectDataStore` beside W6's own and
//! W2's records, so one request reaches every table that holds a customer's words or contact
//! details.
//!
//! W6 scopes a request to sites (`app_names`) and the customer's visitor account (`vac_…`).
//! These the first release tables don't know that id; they hold what the customer or their AI agent typed.
//! So this store reads the account's email and phone from W6's `visitor_accounts` (W6 erases
//! that table last, after every registered store) and matches on them:
//!
//! - `passed_questions` (questions passed on to the owner from Ask, with a way to reply):
//!   matched on `contact`; **deleted** (H).
//! - `decisions` of kind `order-held` (orders an AI agent placed through the agent door): matched
//!   on the order's email; the customer's name, email and phone in the card are **redacted**
//!   and the decision, its fingerprint and receipt stay (R). While such an order still waits
//!   for the owner, erasure is refused with W6's open-orders code, as W2's records do.
//!
//! Receipts are not matched: none carries a customer's contact details. A Concierge refusal
//! keeps 200 characters of the visitor's words with no link to any account (03 §2.4); that
//! stays on the fix list for W6.

use adk_core::subject::{StoreErasure, SubjectDataStore, SubjectScope};
use adk_core::{AdkError, ErrorComponent};
use async_trait::async_trait;
use serde_json::{Value, json};
use sqlx::SqlitePool;

/// W6's code for "this customer has an open order" (`agentrix_identity::visitor::rights::
/// OPEN_ORDERS_CODE`); nothing is erased when a store returns it.
pub const OPEN_ORDERS_CODE: &str = "subject.open_orders";

/// What replaces a redacted field.
const ERASED: &str = "[erased]";

/// `control.db`'s questions passed on and agent-held orders, as one subject store.
#[derive(Debug, Clone)]
pub struct ControlSubjectData {
    pool: SqlitePool,
}

impl ControlSubjectData {
    /// A store over `control.db`.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The account's email and phone on `site`, if W6 knows the account.
    async fn contacts(&self, site: &str, account: &str) -> adk_core::Result<Option<Contacts>> {
        let row: Result<Option<(String, Option<String>)>, sqlx::Error> = sqlx::query_as(
            "SELECT email, phone FROM visitor_accounts WHERE id = ? AND site_id = ?",
        )
        .bind(account)
        .bind(site)
        .fetch_optional(&self.pool)
        .await;
        match row {
            Ok(found) => Ok(found.map(|(email, phone)| Contacts {
                email,
                phone: phone.filter(|p| !p.trim().is_empty()),
            })),
            // Before W6's tables exist there is no account to match.
            Err(sqlx::Error::Database(e)) if e.message().contains("no such table") => Ok(None),
            Err(e) => Err(failed(e)),
        }
    }
}

struct Contacts {
    email: String,
    phone: Option<String>,
}

fn failed(e: impl std::fmt::Display) -> AdkError {
    AdkError::internal(ErrorComponent::Tool, "store.subject", e.to_string())
}

/// Questions passed on with this email or phone as the way to reply.
const QUESTIONS: &str = "FROM passed_questions WHERE site_id = ?1 \
     AND (contact = ?2 COLLATE NOCASE OR (?3 IS NOT NULL AND contact = ?3))";

/// Orders an agent placed for this email.
const HELD: &str = "FROM decisions WHERE site_id = ?1 AND kind = 'order-held' \
     AND json_extract(card, '$.effect.args.input.email') = ?2 COLLATE NOCASE";

#[async_trait]
impl SubjectDataStore for ControlSubjectData {
    fn name(&self) -> &str {
        "Questions passed on and orders placed by agents"
    }

    async fn export(&self, scope: &SubjectScope) -> adk_core::Result<Value> {
        let account = scope.subject.id.as_str();
        let mut out = Vec::new();
        for site in &scope.app_names {
            let Some(c) = self.contacts(site, account).await? else {
                continue;
            };
            let questions: Vec<(String, String, String)> = sqlx::query_as(sqlx::AssertSqlSafe(
                format!("SELECT question_id, question, passed_at {QUESTIONS} ORDER BY passed_at"),
            ))
            .bind(site)
            .bind(&c.email)
            .bind(c.phone.as_deref())
            .fetch_all(&self.pool)
            .await
            .map_err(failed)?;
            let orders: Vec<(String, String, String, Option<String>, Option<String>)> =
                sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT decision_id, state, created_at, json_extract(card, '$.title'), \
                     json_extract(card, '$.effect.args.input') {HELD} ORDER BY created_at"
                )))
                .bind(site)
                .bind(&c.email)
                .fetch_all(&self.pool)
                .await
                .map_err(failed)?;
            if questions.is_empty() && orders.is_empty() {
                continue;
            }
            out.push(json!({
                "site": site,
                "questions_passed_on": questions.into_iter().map(|(id, question, at)| json!({
                    "id": id, "question": question, "passed_at": at,
                })).collect::<Vec<_>>(),
                "orders_placed_by_agents": orders.into_iter().map(|(id, state, at, title, input)| json!({
                    "decision": id,
                    "state": state,
                    "raised_at": at,
                    "title": title,
                    "order": input.and_then(|i| serde_json::from_str::<Value>(&i).ok()),
                })).collect::<Vec<_>>(),
            }));
        }
        Ok(Value::Array(out))
    }

    async fn erase(&self, scope: &SubjectScope) -> adk_core::Result<StoreErasure> {
        let account = scope.subject.id.as_str();
        let mut found = Vec::new();
        for site in &scope.app_names {
            if let Some(c) = self.contacts(site, account).await? {
                let (waiting,): (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT COUNT(*) {HELD} AND state = 'waiting'"
                )))
                .bind(site)
                .bind(&c.email)
                .fetch_one(&self.pool)
                .await
                .map_err(failed)?;
                if waiting > 0 {
                    return Err(AdkError::internal(
                        ErrorComponent::Tool,
                        OPEN_ORDERS_CODE,
                        format!("{waiting} order placed by an agent still waits for the owner"),
                    ));
                }
                found.push((site.clone(), c));
            }
        }
        let mut tx = self.pool.begin().await.map_err(failed)?;
        let mut deleted = 0u64;
        for (site, c) in &found {
            deleted += sqlx::query(sqlx::AssertSqlSafe(format!("DELETE {QUESTIONS}")))
                .bind(site)
                .bind(&c.email)
                .bind(c.phone.as_deref())
                .execute(&mut *tx)
                .await
                .map_err(failed)?
                .rows_affected();
            deleted += sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE decisions SET card = json_replace(card, \
                    '$.effect.args.input.name', ?3, '$.effect.args.input.email', ?3, \
                    '$.effect.args.input.phone', ?3, \
                    '$.body', 'The customer''s details were erased at their request.') \
                 WHERE decision_id IN (SELECT decision_id {HELD})"
            )))
            .bind(site)
            .bind(&c.email)
            .bind(ERASED)
            .execute(&mut *tx)
            .await
            .map_err(failed)?
            .rows_affected();
        }
        tx.commit().await.map_err(failed)?;
        Ok(StoreErasure::deleted(deleted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::principal::SubjectRef;

    async fn db() -> SqlitePool {
        let pool = crate::testing::control_db().await;
        // W6's table, as 0600_w6_visitor_accounts.sql creates it (the columns this store reads).
        sqlx::raw_sql(
            "CREATE TABLE visitor_accounts (id TEXT PRIMARY KEY, site_id TEXT NOT NULL, \
             email TEXT NOT NULL COLLATE NOCASE, name TEXT NOT NULL, phone TEXT);
             INSERT INTO visitor_accounts VALUES ('vac_1', 'site_1', 'njeri@example.com', 'Njeri', '+44 7700 900418');
             INSERT INTO visitor_accounts VALUES ('vac_2', 'site_1', 'amani@example.com', 'Amani', NULL);
             INSERT INTO sites (site_id, domain, old_origin, created_at) VALUES \
               ('site_1', 'kilele.example', 'https://kilele.example', '2026-11-08T00:00:00.000000Z');
             INSERT INTO passed_questions VALUES ('q1', 'site_1', 1, 'Karen delivery?', 'NJERI@example.com', '2026-11-08T01:00:00.000000Z');
             INSERT INTO passed_questions VALUES ('q2', 'site_1', 1, 'Gluten-free?', '+44 7700 900418', '2026-11-08T02:00:00.000000Z');
             INSERT INTO passed_questions VALUES ('q3', 'site_1', 1, 'Open Sunday?', 'someone@example.com', '2026-11-08T03:00:00.000000Z');",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn card(pool: &SqlitePool, id: &str, email: &str, state: &str) {
        let card = json!({
            "title": "Hold a cake order",
            "body": format!("For Sat 14 Nov, from Njeri ({email})."),
            "effect": {"args": {"input": {"name": "Njeri", "email": email, "description": "2-tier cake"}}},
        });
        let (method, signer, assertion, decided) = if state == "waiting" {
            (None, None, None, None)
        } else {
            (
                Some("passkey"),
                None::<&str>,
                Some("a"),
                Some("2026-11-08T05:00:00.000000Z"),
            )
        };
        // An approved card needs a signer account; use a declined one instead.
        let state = if state == "approved" {
            "declined"
        } else {
            state
        };
        sqlx::query(
            "INSERT INTO decisions (decision_id, site_id, kind, card, fingerprint, state, method, \
             signer, assertion, created_at, decided_at) VALUES (?, 'site_1', 'order-held', ?, \
             'sha256:f', ?, ?, ?, ?, '2026-11-08T04:00:00.000000Z', ?)",
        )
        .bind(id)
        .bind(card.to_string())
        .bind(state)
        .bind(method)
        .bind(signer)
        .bind(assertion)
        .bind(decided)
        .execute(pool)
        .await
        .unwrap();
    }

    fn scope(account: &str) -> SubjectScope {
        SubjectScope::new(["site_1"], SubjectRef::human(account))
    }

    #[tokio::test]
    async fn exports_and_erases_the_customers_questions_and_agent_orders() {
        let pool = db().await;
        card(&pool, "dec_1", "njeri@example.com", "approved").await;
        let store = ControlSubjectData::new(pool.clone());

        let out = store.export(&scope("vac_1")).await.unwrap();
        assert_eq!(out[0]["questions_passed_on"].as_array().unwrap().len(), 2);
        assert_eq!(
            out[0]["orders_placed_by_agents"][0]["order"]["name"],
            "Njeri"
        );
        assert_eq!(store.export(&scope("vac_2")).await.unwrap(), json!([]));
        assert_eq!(store.export(&scope("vac_none")).await.unwrap(), json!([]));

        let erased = store.erase(&scope("vac_1")).await.unwrap();
        assert_eq!(
            erased.deleted, 3,
            "two questions deleted, one card redacted"
        );
        let left: Vec<(String,)> = sqlx::query_as("SELECT question_id FROM passed_questions")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(
            left,
            vec![("q3".to_owned(),)],
            "someone else's question stays"
        );
        let (card, fingerprint): (String, String) =
            sqlx::query_as("SELECT card, fingerprint FROM decisions WHERE decision_id = 'dec_1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            !card.contains("Njeri") && !card.contains("njeri@"),
            "{card}"
        );
        assert!(card.contains("2-tier cake"), "the order itself stays");
        assert_eq!(fingerprint, "sha256:f");
        assert_eq!(store.export(&scope("vac_1")).await.unwrap(), json!([]));
    }

    #[tokio::test]
    async fn a_waiting_agent_order_refuses_erasure_and_changes_nothing() {
        let pool = db().await;
        card(&pool, "dec_w", "njeri@example.com", "waiting").await;
        let store = ControlSubjectData::new(pool.clone());
        let refused = store.erase(&scope("vac_1")).await.unwrap_err();
        assert_eq!(refused.code, OPEN_ORDERS_CODE, "W6 matches on the code");
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM passed_questions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 3);
    }

    #[tokio::test]
    async fn without_w6s_tables_there_is_nothing_to_match() {
        let pool = crate::testing::control_db().await;
        let store = ControlSubjectData::new(pool);
        assert_eq!(store.export(&scope("vac_1")).await.unwrap(), json!([]));
        assert_eq!(store.erase(&scope("vac_1")).await.unwrap().deleted, 0);
    }
}
