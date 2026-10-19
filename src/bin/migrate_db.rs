// bin/migrate_db.rs

use sqlx::{Connection, FromRow, SqliteConnection};

use urlharvest::*;

const SQL_READ_URL: &str = "select id, seen, channel, nick, url from url";
const SQL_READ_META: &str = "select url_id, lang, title, desc from url_meta";

#[derive(Debug, FromRow)]
struct DbReadUrl {
    id: i32,
    seen: i64,
    channel: String,
    nick: String,
    url: String,
}

#[derive(Debug, FromRow)]
struct DbReadMeta {
    url_id: i32,
    lang: String,
    title: String,
    desc: String,
}

const SQL_INSERT_URL: &str = "insert into url (id, seen, channel, nick, url) \
     values ($1, $2, $3, $4, $5)";

const SQL_INSERT_META: &str = "insert into url_meta (url_id, lang, title, descr) \
     values ($1, $2, $3, $4)";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::parse();
    opts.finalize()?;
    opts.start_pgm(env!("CARGO_BIN_NAME"));
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{cfg:#?}");

    let dbc = start_db(&cfg).await?;

    let mut sqlite = SqliteConnection::connect("sqlite:./url.db").await?;
    let mut url_ids: HashSet<i32> = HashSet::with_capacity(1_000_000);

    // Keep every write on one connection. An error rolls back the import instead
    // of committing an incomplete copy; this offline tool must run without writers.
    let mut tx = dbc.dbc.begin().await?;
    let mut st_read = sqlx::query_as::<_, DbReadUrl>(SQL_READ_URL).fetch(&mut sqlite);
    while let Some(row) = st_read.try_next().await? {
        sqlx::query(SQL_INSERT_URL)
            .bind(row.id)
            .bind(row.seen)
            .bind(&row.channel)
            .bind(&row.nick)
            .bind(&row.url)
            .execute(&mut *tx)
            .await?;
        url_ids.insert(row.id);
    }
    drop(st_read);

    let mut orphans = 0;
    let mut st_read = sqlx::query_as::<_, DbReadMeta>(SQL_READ_META).fetch(&mut sqlite);
    while let Some(row) = st_read.try_next().await? {
        if !url_ids.contains(&row.url_id) {
            orphans += 1;
            continue;
        }
        sqlx::query(SQL_INSERT_META)
            .bind(row.url_id)
            .bind(&row.lang)
            .bind(&row.title)
            .bind(&row.desc)
            .execute(&mut *tx)
            .await?;
    }
    drop(st_read);
    info!("Detected {orphans} orphan url_meta lines.");

    // Explicit IDs do not advance the sequence. Handle an empty source too:
    // setval(..., 1, false) makes the next generated ID equal to one.
    sqlx::query("select setval('url_id_seq', coalesce(max(id), 1), max(id) is not null) from url")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(())
}

// EOF
