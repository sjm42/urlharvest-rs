// db-migrate.rs

use futures::TryStreamExt; // provides `try_next`
use log::*;
use sqlx::{Connection, Executor, SqliteConnection};
use structopt::StructOpt;
use urlharvest::*;

const TX_SZ: usize = 1024;
const OLD_DB: &str = "sqlite:$HOME/urllog/data/urllog2.db";
const OLD_TABLE_URL: &str = "urllog2";
const OLD_TABLE_META: &str = "urlmeta";

#[derive(Debug, sqlx::FromRow)]
struct DbRead {
    pub id: i64,
    pub seen: i64,
    pub channel: String,
    pub nick: String,
    pub url: String,
    pub lang: String,
    pub title: String,
    pub desc: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::from_args();
    opts.finish()?;
    start_pgm(&opts, "db_migrate");
    info!("Starting up");
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{:#?}", &cfg);

    let mut old_db =
        SqliteConnection::connect(&format!("sqlite:{}", shellexpand::full(OLD_DB)?)).await?;
    let mut new_db = start_db(&cfg).await?;

    let mut i = 0;
    let sql_url_read = format!(
        "select u.id, u.seen, u.channel, u.nick, u.url, m.lang, m.title, m.desc \
        from {OLD_TABLE_URL} u, {OLD_TABLE_META} m \
        where m.url_id = u.id \
        order by u.id"
    );
    let mut sql_read = sqlx::query_as::<_, DbRead>(&sql_url_read).fetch(&mut old_db);

    let sql_write_url =
        format!("insert into {TABLE_URL} (id,seen,channel,nick,url) values (?,?,?,?,?)");
    let sql_write_meta =
        format!("insert into {TABLE_META} (id,url_id,lang,title,desc) values (null,?,?,?,?)");

    new_db.dbc.execute("BEGIN").await?;
    while let Some(row) = sql_read.try_next().await? {
        i += 1;
        if i % TX_SZ == 0 {
            error!("Processing {i}...");
            info!("Data:\n{:#?}", row);
            new_db.dbc.execute("COMMIT").await?;
            new_db.dbc.execute("BEGIN").await?;
        }
        sqlx::query(&sql_write_url)
            .bind(row.id)
            .bind(row.seen)
            .bind(&row.channel)
            .bind(&row.nick)
            .bind(&row.url)
            .execute(&mut new_db.dbc)
            .await?;
        sqlx::query(&sql_write_meta)
            .bind(row.id)
            .bind(&row.lang)
            .bind(&row.title)
            .bind(&row.desc)
            .execute(&mut new_db.dbc)
            .await?;
    }
    new_db.dbc.execute("COMMIT").await?;

    Ok(())
}
// EOF
