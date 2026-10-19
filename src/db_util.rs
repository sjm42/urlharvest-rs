// db_util.rs

use sqlx::{
    Executor, Pool, Postgres,
    postgres::{PgConnectOptions, PgPoolOptions},
};

use crate::*;

const DB_JOB_TIMEOUT: Duration = Duration::from_secs(8);
pub const DB_CHANGE_CHANNEL: &str = "url_db_changed";
pub const PAGE_CHANGE_CHANNEL: &str = "url_pages_changed";

#[derive(Debug, sqlx::FromRow)]
pub struct DbUrl {
    pub id: i32,
    pub seen: i64,
    pub channel: String,
    pub nick: String,
    pub url: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct DbMeta {
    pub id: i32,
    pub url_id: i64,
    pub lang: String,
    pub title: String,
    pub descr: String,
}

#[derive(Debug)]
pub struct DbCtx {
    pub dbc: Pool<Postgres>,
}

#[derive(Debug)]
pub struct UrlCtx {
    pub ts: i64,
    pub chan: String,
    pub nick: String,
    pub url: String,
}

#[derive(Debug)]
pub struct MetaCtx {
    pub url_id: i32,
    pub lang: String,
    pub title: String,
    pub descr: String,
}

pub async fn start_db(c: &ConfigCommon) -> Result<DbCtx, sqlx::Error> {
    let options = c
        .db_url
        .parse::<PgConnectOptions>()?
        .options([("statement_timeout", "5000"), ("lock_timeout", "1000")]);
    let dbc = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(3))
        .connect_with(options)
        .await?;
    sqlx::migrate!().run(&dbc).await?; // will create tables if necessary
    let db = DbCtx { dbc };
    Ok(db)
}

const SQL_INSERT_URL: &str = "insert into url (seen, channel, nick, url) \
    values ($1, $2, $3, $4)";
/// Execute once: retrying a write after an ambiguous connection failure can duplicate it.
/// Accepting an executor lets history imports use their existing transaction.
pub async fn db_add_url<'e, E>(executor: E, ur: &UrlCtx) -> anyhow::Result<u64>
where
    E: Executor<'e, Database = Postgres>,
{
    let res = timeout(
        DB_JOB_TIMEOUT,
        sqlx::query(SQL_INSERT_URL)
            .bind(ur.ts)
            .bind(&ur.chan)
            .bind(&ur.nick)
            .bind(&ur.url)
            .execute(executor),
    )
    .await
    .context("URL insert timed out")?
    .context("URL insert failed")?;
    Ok(res.rows_affected())
}

const SQL_INSERT_META: &str = "insert into url_meta (url_id, lang, title, descr) \
    select id, $2, $3, $4 from url where id = $1 for key share \
    on conflict (url_id) do nothing";
pub async fn db_add_meta(db: &DbCtx, m: &MetaCtx) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(SQL_INSERT_META)
        .bind(m.url_id)
        .bind(&m.lang)
        .bind(&m.title)
        .bind(&m.descr)
        .execute(&db.dbc)
        .await?;
    Ok(res.rows_affected())
}
// EOF
