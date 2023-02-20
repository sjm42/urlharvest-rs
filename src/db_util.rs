// db_util.rs

use chrono::*;
use log::*;
use sqlx::{Connection, SqliteConnection};
use tokio::time::{sleep, Duration};

use crate::*;

const RETRY_CNT: usize = 5;
const RETRY_SLEEP: u64 = 1;

#[derive(Debug, sqlx::FromRow)]
pub struct DbUrl {
    pub id: i64,
    pub seen: i64,
    pub channel: String,
    pub nick: String,
    pub url: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct DbMeta {
    pub id: i64,
    pub url_id: i64,
    pub lang: String,
    pub title: String,
    pub desc: String,
}

#[derive(Debug)]
pub struct DbCtx {
    pub dbc: SqliteConnection,
    pub update_change: bool,
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
    pub url_id: i64,
    pub lang: String,
    pub title: String,
    pub desc: String,
}

pub async fn start_db(c: &ConfigCommon) -> anyhow::Result<DbCtx> {
    let mut dbc = SqliteConnection::connect(&format!("sqlite:{}", &c.db_file)).await?;
    sqlx::migrate!().run(&mut dbc).await?; // will create tables if necessary
    let db = DbCtx {
        dbc,
        update_change: false,
    };
    Ok(db)
}

const SQL_LAST_CHANGE: &str = "select last from url_changed limit 1";
pub async fn db_last_change(db: &mut DbCtx) -> anyhow::Result<i64> {
    let ts: (i64,) = sqlx::query_as(SQL_LAST_CHANGE)
        .fetch_one(&mut db.dbc)
        .await?;
    Ok(ts.0)
}

const SQL_UPDATE_CHANGE: &str = "update url_changed set last=?";
pub async fn db_mark_change(dbc: &mut SqliteConnection) -> anyhow::Result<()> {
    sqlx::query(SQL_UPDATE_CHANGE)
        .bind(Utc::now().timestamp())
        .execute(dbc)
        .await?;
    Ok(())
}

const SQL_INSERT_URL: &str = "insert into url (id, seen, channel, nick, url) \
    values (null, ?, ?, ?, ?)";
pub async fn db_add_url(db: &mut DbCtx, ur: &UrlCtx) -> anyhow::Result<u64> {
    let mut rowcnt = 0;
    let mut retry = 0;
    while retry < RETRY_CNT {
        match sqlx::query(SQL_INSERT_URL)
            .bind(ur.ts)
            .bind(&ur.chan)
            .bind(&ur.nick)
            .bind(&ur.url)
            .execute(&mut db.dbc)
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
    if db.update_change {
        db_mark_change(&mut db.dbc).await?;
    }
    if retry > 0 {
        error!("GAVE UP after {RETRY_CNT} retries.");
    }
    Ok(rowcnt)
}

const SQL_INSERT_META: &str = "insert into url_meta (id, url_id, lang, title, desc) \
        values (null, ?, ?, ?, ?)";
pub async fn db_add_meta(db: &mut DbCtx, m: &MetaCtx) -> anyhow::Result<u64> {
    let res = sqlx::query(SQL_INSERT_META)
        .bind(m.url_id)
        .bind(&m.lang)
        .bind(&m.title)
        .bind(&m.desc)
        .execute(&mut db.dbc)
        .await?;

    if db.update_change {
        db_mark_change(&mut db.dbc).await?;
    }
    Ok(res.rows_affected())
}
// EOF
