//! Temporary fixtures for the binaries' opt-in database query tests.

use sqlx::{Connection, PgConnection};

pub async fn metadata_fixture() -> PgConnection {
    let db_url = std::env::var("URLHARVEST_TEST_DATABASE_URL").expect("set URLHARVEST_TEST_DATABASE_URL");
    let mut conn = PgConnection::connect(&db_url).await.unwrap();
    for statement in [
        "create temporary table url (id serial primary key, seen bigint not null, channel text not null, nick text not null, url text not null)",
        "create temporary table url_meta (id serial primary key, url_id integer unique references url(id), title text not null)",
        "insert into url (seen, channel, nick, url) values \
         (30, '#test', 'alice', 'https://example.com/'), \
         (20, '#test', 'bob', 'https://example.com/'), \
         (10, '#other', 'carol', 'https://example.com/'), \
         (40, '#test', 'alice', 'https://other.example/'), \
         (50, '#test', 'pending', 'https://example.com/')",
        // Metadata order deliberately differs from URL IDs and sighting times.
        "insert into url_meta (url_id, title) values \
         (3, 'Other channel title'), (1, 'Old title'), (2, 'Current title'), (4, 'Unrelated title')",
    ] {
        sqlx::query(statement).execute(&mut conn).await.unwrap();
    }
    conn
}
