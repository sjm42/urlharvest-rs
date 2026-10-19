//! Run with a disposable PostgreSQL database:
//! URLHARVEST_TEST_DATABASE_URL=postgres:///urlharvest_test cargo test --test database -- --ignored
//! Tests use temporary tables or an isolated fixture schema. The notification
//! test applies migrations in its own schema and drops that schema on success.

use sqlx::{
    AssertSqlSafe, Connection, PgConnection,
    postgres::{PgListener, PgPoolOptions},
};
use urlharvest::{DB_CHANGE_CHANNEL, DbCtx, MetaCtx, PAGE_CHANGE_CHANNEL, UrlCtx, db_add_meta, db_add_url};

async fn assert_notifications(writer: &mut PgConnection, listener: &mut PgListener, barrier: &str, expected: &[&str]) {
    // A notification committed after the tested write is an ordered barrier:
    // receiving it proves that all notifications from that write were drained.
    sqlx::query("select pg_notify($1, '')")
        .bind(barrier)
        .execute(writer)
        .await
        .unwrap();
    let mut received = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut channels = Vec::new();
        loop {
            let notification = listener.recv().await.unwrap();
            if notification.channel() == barrier {
                break channels;
            }
            channels.push(notification.channel().to_owned());
        }
    })
    .await
    .expect("notification barrier was not received");
    let mut expected: Vec<_> = expected.iter().map(|channel| channel.to_string()).collect();
    received.sort();
    expected.sort();
    assert_eq!(received, expected);
}

#[tokio::test]
#[ignore = "requires URLHARVEST_TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
async fn notifications_wake_the_generator_only_after_metadata_or_visible_changes() {
    let db_url = std::env::var("URLHARVEST_TEST_DATABASE_URL").expect("set URLHARVEST_TEST_DATABASE_URL");
    let mut writer = PgConnection::connect(&db_url).await.unwrap();
    let backend: i32 = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(&mut writer)
        .await
        .unwrap();
    // PostgreSQL does not resolve trigger functions through pg_temp. Use a
    // private fixture schema so the actual migrations can run unchanged. Its
    // identifier contains only a fixed prefix and a database-supplied integer.
    let schema = format!("urlharvest_notify_test_{backend}");
    sqlx::query(AssertSqlSafe(format!("create schema {schema}")))
        .execute(&mut writer)
        .await
        .unwrap();
    sqlx::query("select set_config('search_path', $1, false)")
        .bind(&schema)
        .execute(&mut writer)
        .await
        .unwrap();
    sqlx::migrate!().run(&mut writer).await.unwrap();

    let barrier = format!("urlharvest_test_barrier_{backend}");
    let mut listener = PgListener::connect(&db_url).await.unwrap();
    listener
        .listen_all([DB_CHANGE_CHANNEL, PAGE_CHANGE_CHANNEL, barrier.as_str()])
        .await
        .unwrap();
    let both = &[DB_CHANGE_CHANNEL, PAGE_CHANGE_CHANNEL];

    sqlx::query(
        "insert into url (id, seen, channel, nick, url) values (1, 1, '#test', 'test', 'https://example.com/')",
    )
    .execute(&mut writer)
    .await
    .unwrap();
    assert_notifications(&mut writer, &mut listener, &barrier, &[DB_CHANGE_CHANNEL]).await;

    for statement in [
        "insert into url_meta (url_id, title) values (1, 'Title')",
        "update url_meta set title = 'New title' where url_id = 1",
        "update url set nick = 'renamed' where id = 1",
        "delete from url_meta where url_id = 1",
        "insert into url_meta (url_id, title) values (1, '(URL fetch error)')",
        // The cascading metadata delete and parent delete must coalesce into
        // one notification per channel within the same transaction.
        "delete from url where id = 1",
    ] {
        sqlx::query(statement).execute(&mut writer).await.unwrap();
        assert_notifications(&mut writer, &mut listener, &barrier, both).await;
    }

    let mut tx = writer.begin().await.unwrap();
    sqlx::query(
        "insert into url (id, seen, channel, nick, url) values (2, 2, '#test', 'test', 'https://example.com/')",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("insert into url_meta (url_id, title) values (2, 'Title')")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_notifications(&mut writer, &mut listener, &barrier, both).await;

    let mut tx = writer.begin().await.unwrap();
    sqlx::query("update url_meta set title = 'Rolled back' where url_id = 2")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_notifications(&mut writer, &mut listener, &barrier, &[]).await;

    sqlx::query(AssertSqlSafe(format!("drop schema {schema} cascade")))
        .execute(&mut writer)
        .await
        .unwrap();
}

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
