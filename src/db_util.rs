// db_util.rs

use sqlx::{Pool, Postgres};

use crate::*;

const RETRY_CNT: usize = 5;
const RETRY_SLEEP: u64 = 1;
pub const DB_CHANGE_CHANNEL: &str = "url_db_changed";

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
    let dbc = sqlx::PgPool::connect(&c.db_url).await?;
    sqlx::migrate!().run(&dbc).await?; // will create tables if necessary
    let db = DbCtx { dbc };
    Ok(db)
}

const SQL_INSERT_URL: &str = "insert into url (seen, channel, nick, url) \
    values ($1, $2, $3, $4)";
pub async fn db_add_url(db: &mut DbCtx, ur: &UrlCtx) -> Result<u64, sqlx::Error> {
    let mut rowcnt = 0;
    let mut retry = 0;
    while retry < RETRY_CNT {
        match sqlx::query(SQL_INSERT_URL)
            .bind(ur.ts)
            .bind(&ur.chan)
            .bind(&ur.nick)
            .bind(&ur.url)
            .execute(&db.dbc)
            .await
        {
            Ok(res) => {
                info!("Insert result: {res:#?}");
                retry = 0;
                rowcnt = res.rows_affected();
                break;
            }
            Err(e) => {
                error!("Insert failed: {e:?}");
            }
        }
        error!("Retrying in {}s...", RETRY_SLEEP);
        sleep(Duration::new(RETRY_SLEEP, 0)).await;
        retry += 1;
    }
    if retry > 0 {
        error!("GAVE UP after {RETRY_CNT} retries.");
    }
    Ok(rowcnt)
}

const SQL_INSERT_META: &str = "insert into url_meta (url_id, lang, title, descr) \
        values ($1, $2, $3, $4)";
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
