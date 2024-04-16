// bin/migrate_db.rs

use std::collections::HashSet;

use clap::Parser;
// provides `try_next`
use futures::TryStreamExt;
use sqlx::{Connection, Executor, FromRow, Row, SqliteConnection};

use urlharvest::*;

const TX_SZ: usize = 1024;

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
     values ($1, $2, $3, $4, $5) returning id";

const SQL_INSERT_META: &str = "insert into url_meta (url_id, lang, title, descr) \
     values ($1, $2, $3, $4) returning id";

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

    let mut tx_i = 0;
    let mut st_read = sqlx::query_as::<_, DbReadUrl>(SQL_READ_URL).fetch(&mut sqlite);
    dbc.dbc.execute("BEGIN").await?;
    while let Some(row) = st_read.try_next().await? {
        tx_i += 1;

        // debug!("row:\n{row:#?}");
        let res = sqlx::query(SQL_INSERT_URL)
            .bind(row.id)
            .bind(row.seen)
            .bind(&row.channel)
            .bind(&row.nick)
            .bind(&row.url)
            .fetch_one(&dbc.dbc)
            .await?;
        let url_id: Option<i32> = res.get(0);
        if url_id.is_none() {
            error!("url_id is none");
            continue;
        }
        let url_id = url_id.unwrap();
        url_ids.insert(url_id);

        if tx_i >= TX_SZ {
            debug!("Inserted url_id #{url_id}");
            dbc.dbc.execute("COMMIT").await?;
            dbc.dbc.execute("BEGIN").await?;
            tx_i = 0;
        }
    }
    drop(st_read);
    dbc.dbc.execute("COMMIT").await?;

    tx_i = 0;
    let mut orphans = 0;
    let mut st_read = sqlx::query_as::<_, DbReadMeta>(SQL_READ_META).fetch(&mut sqlite);
    dbc.dbc.execute("BEGIN").await?;
    while let Some(row) = st_read.try_next().await? {
        if !url_ids.contains(&row.url_id) {
            orphans += 1;
            continue;
        }

        tx_i += 1;
        let res = match sqlx::query(SQL_INSERT_META)
            .bind(row.url_id)
            .bind(&row.lang)
            .bind(&row.title)
            .bind(&row.desc)
            .fetch_one(&dbc.dbc)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                error!("Insert error: {e}");
                continue;
            }
        };
        let meta_id: Option<i32> = res.get(0);
        if meta_id.is_none() {
            error!("meta_id is none");
            continue;
        }
        let meta_id = meta_id.unwrap();

        if tx_i >= TX_SZ {
            debug!("Inserted url_meta #{meta_id}");
            dbc.dbc.execute("COMMIT").await?;
            dbc.dbc.execute("BEGIN").await?;
            tx_i = 0;
        }
    }
    drop(st_read);
    dbc.dbc.execute("COMMIT").await?;
    info!("Detected {orphans} orphan url_meta lines.");

    // update the sequence to actually give unique values
    // since url(id) were just copied from previous db
    let seq_val: i32 = sqlx::query("select max(id) from url")
        .fetch_one(&dbc.dbc)
        .await?
        .get(0);
    info!("url id seq: {seq_val}");
    sqlx::query("select setval('url_id_seq', $1)")
        .bind(seq_val)
        .execute(&dbc.dbc)
        .await?;

    Ok(())
}

// EOF
