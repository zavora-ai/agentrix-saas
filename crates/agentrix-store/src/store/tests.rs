//! The single-node store layer on a temporary directory.

use std::sync::Arc;

use agentrix_types::{Digest, KeySet, ReceiptBody, ReceiptKind, Signer, SiteId, Subject};
use async_trait::async_trait;
use chrono::Utc;

use super::*;

fn stores(dir: &tempfile::TempDir) -> (SqliteSiteStores, Arc<Signer>) {
    let signer = Arc::new(Signer::generate());
    (
        SqliteSiteStores::new(dir.path().join("sites"), signer.clone()),
        signer,
    )
}

fn body(site: &SiteId, title: &str) -> ReceiptBody {
    let now = Utc::now();
    ReceiptBody::new(
        ReceiptKind::RoutingChanged,
        site.clone(),
        Subject::Site(site.clone()),
        now,
        now,
    )
    .with_title(title)
}

fn page() -> BlobMeta {
    BlobMeta {
        kind: "page".into(),
        content_type: "text/html".into(),
        class: DataClass::C0,
    }
}

#[tokio::test]
async fn each_site_gets_its_own_directory_and_migrated_records() {
    let dir = tempfile::tempdir().unwrap();
    let (stores, _) = stores(&dir);
    let kilele = SiteId::new("site_kilele");
    let store = stores.open(&kilele).await.unwrap();
    assert!(dir.path().join("sites/site_kilele/records.db").exists());
    let (tables,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' \
         AND name IN ('blobs', 'site_log', 'outbox', 'relay_cursors', '_sqlx_migrations_store')",
    )
    .fetch_one(&store.records)
    .await
    .unwrap();
    assert_eq!(tables, 5);
    // Opening again hands back the same pool.
    let again = stores.open(&kilele).await.unwrap();
    assert!(again.records.size() >= 1);
    stores.open(&SiteId::new("site_tana")).await.unwrap();
    let listed: Vec<String> = stores
        .sites()
        .await
        .unwrap()
        .iter()
        .map(|s| s.as_str().to_owned())
        .collect();
    assert_eq!(listed, vec!["site_kilele", "site_tana"]);
}

#[tokio::test]
async fn site_ids_that_could_escape_the_folder_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (stores, _) = stores(&dir);
    for bad in ["../control", "a/b", "", "-x", "site.db"] {
        assert!(
            matches!(
                stores.open(&SiteId::new(bad)).await,
                Err(StoreError::Refused(_))
            ),
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn blobs_stay_with_their_site() {
    let dir = tempfile::tempdir().unwrap();
    let (stores, _) = stores(&dir);
    let a = stores.open(&SiteId::new("site_a")).await.unwrap();
    let b = stores.open(&SiteId::new("site_b")).await.unwrap();
    let digest = a.blobs.put(b"<h1>Kilele</h1>", &page()).await.unwrap();
    assert_eq!(digest, Digest::of(b"<h1>Kilele</h1>"));
    assert_eq!(
        a.blobs.get(&digest).await.unwrap().as_deref(),
        Some(&b"<h1>Kilele</h1>"[..])
    );
    assert_eq!(
        b.blobs.get(&digest).await.unwrap(),
        None,
        "site b can't read a's blob"
    );
    // The same bytes again are a no-op.
    assert_eq!(
        a.blobs.put(b"<h1>Kilele</h1>", &page()).await.unwrap(),
        digest
    );
    assert!(a.blobs.remove(&digest).await.unwrap());
    assert_eq!(a.blobs.get(&digest).await.unwrap(), None);
    assert!(!a.blobs.remove(&digest).await.unwrap());
}

struct XorSealer;

#[async_trait]
impl Sealer for XorSealer {
    async fn seal(&self, _site: &SiteId, plain: &[u8]) -> Result<Vec<u8>> {
        Ok(plain.iter().map(|b| b ^ 0x5a).collect())
    }
    async fn open(&self, _site: &SiteId, sealed: &[u8]) -> Result<Vec<u8>> {
        Ok(sealed.iter().map(|b| b ^ 0x5a).collect())
    }
}

#[tokio::test]
async fn personal_blobs_need_a_sealer_and_secrets_never_go_in() {
    let dir = tempfile::tempdir().unwrap();
    let (plain_stores, _) = stores(&dir);
    let site = SiteId::new("site_a");
    let store = plain_stores.open(&site).await.unwrap();
    let personal = BlobMeta {
        class: DataClass::C3,
        ..page()
    };
    assert!(matches!(
        store.blobs.put(b"Njeri, +44 7700 900418", &personal).await,
        Err(StoreError::Refused(_))
    ));
    let secret = BlobMeta {
        class: DataClass::C4,
        ..page()
    };
    assert!(matches!(
        store.blobs.put(b"key", &secret).await,
        Err(StoreError::Refused(_))
    ));

    let dir = tempfile::tempdir().unwrap();
    let signer = Arc::new(Signer::generate());
    let sealed_stores =
        SqliteSiteStores::new(dir.path().join("sites"), signer).with_sealer(Arc::new(XorSealer));
    let store = sealed_stores.open(&site).await.unwrap();
    let digest = store
        .blobs
        .put(b"Njeri, +44 7700 900418", &personal)
        .await
        .unwrap();
    let hex = digest.hex();
    let on_disk = std::fs::read(
        dir.path()
            .join("sites/site_a/cas/sha256")
            .join(&hex[..2])
            .join(&hex[2..]),
    )
    .unwrap();
    assert_ne!(on_disk, b"Njeri, +44 7700 900418", "sealed on disk");
    assert_eq!(
        store.blobs.get(&digest).await.unwrap().as_deref(),
        Some(&b"Njeri, +44 7700 900418"[..])
    );
}

#[tokio::test]
async fn the_site_log_chains_verifies_and_survives_redaction() {
    let dir = tempfile::tempdir().unwrap();
    let (stores, signer) = stores(&dir);
    let site = SiteId::new("site_kilele");
    let store = stores.open(&site).await.unwrap();
    let keys = KeySet {
        keys: vec![signer.published_key()],
    };
    let first = store.log.issue(body(&site, "Hours changed")).await.unwrap();
    let second = store
        .log
        .issue(body(&site, "Sunday hours live"))
        .await
        .unwrap();
    let third = store.log.issue(body(&site, "A refusal")).await.unwrap();
    assert_eq!((first.seq, second.seq, third.seq), (1, 2, 3));
    assert_eq!(first.prev_chain, "");
    assert_eq!(second.prev_chain, first.chain, "the chain predecessor");
    assert_eq!(third.chain, chain_of(&second.chain, &third.receipt_digest));
    assert_eq!(store.log.key_id(), signer.key_id());

    let ok = store.log.verify(&keys).await.unwrap();
    assert_eq!((ok.entries, ok.problem.clone()), (3, None));
    assert_eq!(ok.head.as_deref(), Some(third.chain.as_str()));
    assert_eq!(store.log.count_since("").await.unwrap(), 3);

    // Erasure redacts the words and keeps the chain whole.
    assert!(store.log.redact(&second.receipt_digest).await.unwrap());
    let after = store.log.verify(&keys).await.unwrap();
    assert_eq!((after.redacted, after.problem.clone()), (1, None));
    let listed = store.log.entries(0, 10).await.unwrap();
    assert!(listed[1].receipt.is_none());
    assert_eq!(
        listed[2].receipt.as_ref().unwrap().body.title(),
        Some("A refusal")
    );

    // A changed row breaks the chain at that row.
    sqlx::query("UPDATE site_log SET receipt_digest = ? WHERE seq = 3")
        .bind(Digest::of(b"forged").to_string())
        .execute(&store.records)
        .await
        .unwrap();
    let broken = store.log.verify(&keys).await.unwrap();
    assert_eq!(broken.problem.map(|(seq, _)| seq), Some(3));

    // A receipt for another site never goes in this log.
    assert!(matches!(
        store.log.issue(body(&SiteId::new("site_other"), "x")).await,
        Err(StoreError::Refused(_))
    ));
    // Another key's signature doesn't verify.
    let stranger = KeySet {
        keys: vec![Signer::generate().published_key()],
    };
    let fresh = stores.open(&SiteId::new("site_b")).await.unwrap();
    fresh
        .log
        .issue(body(&SiteId::new("site_b"), "x"))
        .await
        .unwrap();
    assert!(fresh.log.verify(&stranger).await.unwrap().problem.is_some());
}

#[tokio::test]
async fn the_outbox_keeps_order_refuses_personal_events_and_rolls_back_with_its_write() {
    let dir = tempfile::tempdir().unwrap();
    let (stores, _) = stores(&dir);
    let store = stores.open(&SiteId::new("site_a")).await.unwrap();
    let event = |n: i64, class| Event {
        topic: "agx.usage.v1".into(),
        key: format!("call_{n}"),
        class,
        payload: serde_json::json!({ "n": n }),
    };
    assert_eq!(
        store.outbox.emit(&event(1, DataClass::C1)).await.unwrap(),
        1
    );
    assert_eq!(
        store.outbox.emit(&event(2, DataClass::C0)).await.unwrap(),
        2
    );
    assert!(matches!(
        store.outbox.emit(&event(3, DataClass::C3)).await,
        Err(StoreError::Refused(_))
    ));
    // An event written in a transaction that rolls back never appears.
    let mut tx = store.records.begin().await.unwrap();
    store
        .outbox
        .emit_in(&mut tx, &event(4, DataClass::C1))
        .await
        .unwrap();
    tx.rollback().await.unwrap();

    let pending = store.outbox.pending("lake", 10).await.unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|(seq, e)| (*seq, e.key.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "call_1"), (2, "call_2")]
    );
    store.outbox.ack("lake", 1).await.unwrap();
    store.outbox.ack("lake", 0).await.unwrap();
    let left = store.outbox.pending("lake", 10).await.unwrap();
    assert_eq!(left.len(), 1, "a cursor never moves back");
    assert_eq!(store.outbox.pending("other", 10).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_crates_site_migrations_run_in_its_own_table_and_closing_removes_the_site() {
    static RECORDS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/site");
    let dir = tempfile::tempdir().unwrap();
    let signer = Arc::new(Signer::generate());
    // Any crate's migrator works; this reuses W8's own files under another owner's table.
    let stores =
        SqliteSiteStores::new(dir.path().join("sites"), signer).with_migrations(SiteMigrations {
            owner: "records_test",
            migrator: &RECORDS,
        });
    let site = SiteId::new("site_a");
    let store = stores.open(&site).await;
    // The same CREATE TABLE twice fails: proof the second migrator really ran.
    assert!(store.is_err());

    let dir = tempfile::tempdir().unwrap();
    let (stores, _) = self::stores(&dir);
    let store = stores.open(&site).await.unwrap();
    store.blobs.put(b"page", &page()).await.unwrap();
    let closed = stores.close(&site).await.unwrap();
    assert!(closed.files >= 2, "{closed:?}");
    assert!(!dir.path().join("sites/site_a").exists());
    assert!(stores.sites().await.unwrap().is_empty());
}
