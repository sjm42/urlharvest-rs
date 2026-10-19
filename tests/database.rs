//! Run with a disposable PostgreSQL database:
//! URLHARVEST_TEST_DATABASE_URL=postgres:///urlharvest_test cargo test --test database -- --ignored
//! Each test shadows the real URL table with a connection-local temporary table.

use sqlx::{Connection, postgres::PgPoolOptions};
use urlharvest::{DbCtx, MetaCtx, UrlCtx, db_add_meta, db_add_url};

#[tokio::test]
#[ignore = "requires URLHARVEST_TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
async fn url_insert_honors_transaction_commit_and_rollback_and_reports_errors() {
    let db_url = std::env::var("URLHARVEST_TEST_DATABASE_URL").expect("set URLHARVEST_TEST_DATABASE_URL");
    let pool = PgPoolOptions::new().max_connections(1).connect(&db_url).await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("create temporary table url (id serial primary key, seen bigint, channel text, nick text, url text check(url <> 'invalid'))")
        .execute(&mut *conn)
        .await
        .unwrap();
    let mut row = UrlCtx {
        ts: 1234,
        chan: "#test".into(),
        nick: "test".into(),
        url: "https://example.com/".into(),
    };
    let mut tx = conn.begin().await.unwrap();
    assert_eq!(db_add_url(&mut *tx, &row).await.unwrap(), 1);
    tx.rollback().await.unwrap();
    let count: i64 = sqlx::query_scalar("select count(*) from url")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(count, 0);

    let mut tx = conn.begin().await.unwrap();
    db_add_url(&mut *tx, &row).await.unwrap();
    tx.commit().await.unwrap();
    let count: i64 = sqlx::query_scalar("select count(*) from url")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(count, 1);

    row.url = "invalid".into();
    let error = db_add_url(&mut *conn, &row).await.unwrap_err();
    assert!(
        error
            .chain()
            .any(|cause| cause.to_string().contains("check constraint"))
    );
    let count: i64 = sqlx::query_scalar("select count(*) from url")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(count, 1);
    sqlx::query(
        "create temporary table url_meta (url_id integer unique references url(id), lang text, title text, descr text)",
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    drop(conn);
    let db = DbCtx { dbc: pool };
    let mut meta = MetaCtx {
        url_id: 2,
        lang: "en".into(),
        title: "Original".into(),
        descr: String::new(),
    };
    assert_eq!(db_add_meta(&db, &meta).await.unwrap(), 1);
    meta.title = "Duplicate".into();
    assert_eq!(db_add_meta(&db, &meta).await.unwrap(), 0);
    let title: String = sqlx::query_scalar("select title from url_meta")
        .fetch_one(&db.dbc)
        .await
        .unwrap();
    assert_eq!(title, "Original");
    meta.url_id = 999;
    assert_eq!(db_add_meta(&db, &meta).await.unwrap(), 0);
}
