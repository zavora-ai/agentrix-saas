//! The receipts store (design §4.4): the one writer of the `receipts` table.
//!
//! A lane surfaces something to the owner by issuing a receipt here, never by writing to
//! console tables. The Trust tab lists receipts by kind, and the Chief brief summarises them.

use std::sync::Arc;

use agentrix_types::{
    Digest, KeySet, PublishedKey, Receipt, ReceiptBody, ReceiptKind, Signer, SiteId, Subject,
};
use anyhow::Context as _;
use sqlx::SqlitePool;

use super::{now, time};

/// A stored receipt and its content address.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    /// The receipt's digest, as served at `GET /receipts/{digest}`.
    pub digest: Digest,
    /// The signed receipt.
    pub receipt: Receipt,
}

/// Signs receipts with control's key and stores them.
#[derive(Debug, Clone)]
pub struct ReceiptStore {
    pool: SqlitePool,
    signer: Arc<Signer>,
    keys: KeySet,
}

impl ReceiptStore {
    /// A store that signs with `signer` and publishes its key.
    pub fn new(pool: SqlitePool, signer: Arc<Signer>) -> Self {
        let keys = KeySet {
            keys: vec![signer.published_key()],
        };
        Self { pool, signer, keys }
    }

    /// Also publishes retired keys, so receipts they signed still verify.
    pub fn with_retired_keys(mut self, retired: impl IntoIterator<Item = PublishedKey>) -> Self {
        self.keys.keys.extend(retired);
        self
    }

    /// The document for `GET /.well-known/agentrix/keys`.
    pub fn keys(&self) -> &KeySet {
        &self.keys
    }

    /// Signs `body` with control's key (setting its issuer), stores it and returns it with its
    /// digest.
    pub async fn issue(&self, body: ReceiptBody) -> anyhow::Result<Stored> {
        let receipt = Receipt::sign(body, &self.signer).context("signing the receipt")?;
        let json = receipt
            .canonical_json()
            .context("canonicalising the receipt")?;
        let digest = Digest::of(&json);
        let json = String::from_utf8(json).context("canonical JSON is UTF-8")?;
        let (subject_type, subject_id) = subject_parts(&receipt.body.subject);
        sqlx::query(
            "INSERT INTO receipts (digest, receipt_id, site_id, kind, subject_type, subject_id, \
             issuer, started_at, ended_at, issued_at, body) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(digest.to_string())
        .bind(receipt.body.receipt_id.as_str())
        .bind(receipt.body.site.as_str())
        .bind(receipt.body.kind.as_str())
        .bind(subject_type)
        .bind(subject_id)
        .bind(receipt.body.issuer.as_str())
        .bind(time(receipt.body.started_at))
        .bind(time(receipt.body.ended_at))
        .bind(now())
        .bind(json)
        .execute(&self.pool)
        .await
        .context("storing the receipt")?;
        Ok(Stored { digest, receipt })
    }

    /// Writes the `claim.verified` receipt for a proved claim (claim/'s `ClaimEvents`). The
    /// receipt binds the digest of what the sandbox saw; the claim row keeps the evidence.
    pub async fn claim_verified(
        &self,
        site: &SiteId,
        claim_id: &str,
        domain: &str,
        method: &str,
        evidence: &serde_json::Value,
        at: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Stored> {
        let mut body = ReceiptBody::new(
            ReceiptKind::ClaimVerified,
            site.clone(),
            Subject::Site(site.clone()),
            at,
            at,
        );
        body.inputs = vec![Digest::of(&agentrix_types::canonical_json(evidence)?)];
        body.detail = Some(serde_json::json!({
            "claim": claim_id,
            "domain": domain,
            "method": method,
        }));
        let how = if method == "dns" {
            "a DNS record"
        } else {
            "a file on the site"
        };
        self.issue(body.with_title(format!("Proved you own {domain} with {how}")))
            .await
    }

    /// The canonical JSON stored under `digest`, byte for byte: what
    /// `GET /receipts/{digest}` serves.
    pub async fn get_json(&self, digest: &Digest) -> anyhow::Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT body FROM receipts WHERE digest = ?")
            .bind(digest.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(body,)| body))
    }

    /// The receipt stored under `digest`.
    pub async fn get(&self, digest: &Digest) -> anyhow::Result<Option<Receipt>> {
        self.get_json(digest)
            .await?
            .map(|body| serde_json::from_str(&body).context("a stored receipt does not parse"))
            .transpose()
    }

    /// A site's receipts, newest first, optionally of one kind.
    pub async fn list(
        &self,
        site: &SiteId,
        kind: Option<ReceiptKind>,
        limit: u32,
    ) -> anyhow::Result<Vec<Stored>> {
        let kind = kind.map(ReceiptKind::as_str);
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT digest, body FROM receipts WHERE site_id = ? AND (? IS NULL OR kind = ?) \
             ORDER BY issued_at DESC, rowid DESC LIMIT ?",
        )
        .bind(site.as_str())
        .bind(kind)
        .bind(kind)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(parse_row).collect()
    }

    /// A site's receipts issued since `since`, oldest first, for the Chief brief.
    pub async fn list_since(
        &self,
        site: &SiteId,
        since: chrono::DateTime<chrono::Utc>,
        limit: u32,
    ) -> anyhow::Result<Vec<Stored>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT digest, body FROM receipts WHERE site_id = ? AND issued_at >= ? \
             ORDER BY issued_at, rowid LIMIT ?",
        )
        .bind(site.as_str())
        .bind(time(since))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(parse_row).collect()
    }

    /// Every receipt about one subject, oldest first.
    pub async fn for_subject(&self, subject: &Subject) -> anyhow::Result<Vec<Stored>> {
        let (subject_type, subject_id) = subject_parts(subject);
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT digest, body FROM receipts WHERE subject_type = ? AND subject_id = ? \
             ORDER BY issued_at, rowid",
        )
        .bind(subject_type)
        .bind(subject_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(parse_row).collect()
    }
}

fn parse_row((digest, body): (String, String)) -> anyhow::Result<Stored> {
    Ok(Stored {
        digest: digest.parse().context("a stored digest does not parse")?,
        receipt: serde_json::from_str(&body).context("a stored receipt does not parse")?,
    })
}

fn subject_parts(subject: &Subject) -> (&'static str, &str) {
    match subject {
        Subject::Assessment(id) => ("assessment", id.as_str()),
        Subject::Move(id) => ("move", id.as_str()),
        Subject::Job(id) => ("job", id.as_str()),
        Subject::Decision(id) => ("decision", id.as_str()),
        Subject::Account(id) => ("account", id.as_str()),
        Subject::Site(id) => ("site", id.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentrix_types::{JobId, ReceiptKind};
    use chrono::Utc;

    async fn store() -> ReceiptStore {
        let pool = super::super::open_in_memory().await.unwrap();
        sqlx::query("INSERT INTO sites (site_id, domain, old_origin, created_at) VALUES ('site_1', 'bakery.example', 'https://bakery.example', ?)")
            .bind(now())
            .execute(&pool)
            .await
            .unwrap();
        ReceiptStore::new(pool, Arc::new(Signer::generate()))
    }

    fn body(kind: ReceiptKind, job: &str) -> ReceiptBody {
        let at = Utc::now();
        ReceiptBody::new(
            kind,
            SiteId::new("site_1"),
            Subject::Job(JobId::new(job)),
            at,
            at,
        )
    }

    #[tokio::test]
    async fn issued_receipts_are_served_verifiable_and_listed() {
        let store = store().await;
        let fetch = store
            .issue(body(ReceiptKind::JobFetch, "job_1"))
            .await
            .unwrap();
        let build = store
            .issue(body(ReceiptKind::JobBuild, "job_2"))
            .await
            .unwrap();

        let served = store.get_json(&fetch.digest).await.unwrap().unwrap();
        assert_eq!(Digest::of(served.as_bytes()), fetch.digest);
        let got = store.get(&fetch.digest).await.unwrap().unwrap();
        got.verify_with(store.keys()).unwrap();

        let site = SiteId::new("site_1");
        assert_eq!(store.list(&site, None, 10).await.unwrap().len(), 2);
        let builds = store
            .list(&site, Some(ReceiptKind::JobBuild), 10)
            .await
            .unwrap();
        assert_eq!(builds, vec![build]);
        let about_job = store
            .for_subject(&Subject::Job(JobId::new("job_1")))
            .await
            .unwrap();
        assert_eq!(about_job, vec![fetch]);
        assert!(store.get(&Digest::of(b"absent")).await.unwrap().is_none());
        let recent = store
            .list_since(&site, Utc::now() - chrono::Duration::minutes(5), 10)
            .await
            .unwrap();
        assert_eq!(recent.len(), 2);
        assert!(
            store
                .list_since(&site, Utc::now() + chrono::Duration::minutes(5), 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_verified_claim_gets_a_titled_receipt() {
        let store = store().await;
        let evidence = serde_json::json!({ "txt": ["agentrix-verify=abcd-efgh-ijkl-mnop"] });
        let stored = store
            .claim_verified(
                &SiteId::new("site_1"),
                "claim_1",
                "bakery.example",
                "dns",
                &evidence,
                Utc::now(),
            )
            .await
            .unwrap();
        let body = stored.receipt.verify_with(store.keys()).unwrap();
        assert_eq!(body.kind, ReceiptKind::ClaimVerified);
        assert_eq!(
            body.title(),
            Some("Proved you own bakery.example with a DNS record")
        );
        assert_eq!(
            body.inputs,
            vec![Digest::of(
                &agentrix_types::canonical_json(&evidence).unwrap()
            )]
        );
    }
}
