// db_util.rs

use chrono::*;
use log::*;
// use rusqlite::{named_params, Connection};
use sqlx::{Connection, SqliteConnection};
use std::{thread, time};

use crate::*;

pub const TABLE_URL: &str = "url";
pub const TABLE_CHANGED: &str = "url_changed";
pub const TABLE_META: &str = "url_meta";

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

pub async fn db_last_change(db: &mut DbCtx) -> anyhow::Result<i64> {
    let sql = format!("select last from {table} limit 1", table = TABLE_CHANGED);
    let ts: (i64,) = sqlx::query_as(&sql).fetch_one(&mut db.dbc).await?;
    Ok(ts.0)
}

pub async fn db_mark_change(db: &mut DbCtx) -> anyhow::Result<()> {
    let sql = format!(
        "update {table} set last={ts};",
        table = TABLE_CHANGED,
        ts = Utc::now().timestamp()
    );
    sqlx::query(&sql).execute(&mut db.dbc).await?;
    Ok(())
}

pub async fn db_add_url(db: &mut DbCtx, ur: &UrlCtx) -> anyhow::Result<u64> {
    let sql_i = format!(
        "insert into {table} (id, seen, channel, nick, url) \
        values (null, ?, ?, ?, ?)",
        table = TABLE_URL
    );

    let mut rowcnt = 0;
    let mut retry = 0;
    while retry < RETRY_CNT {
        match sqlx::query(&sql_i)
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
        thread::sleep(time::Duration::new(RETRY_SLEEP, 0));
        retry += 1;
    }
    if db.update_change {
        db_mark_change(db).await?;
    }
    if retry > 0 {
        error!("GAVE UP after {RETRY_CNT} retries.");
    }
    Ok(rowcnt)
}

pub async fn db_add_meta(db: &mut DbCtx, m: &MetaCtx) -> anyhow::Result<u64> {
    let sql_i = format!(
        "insert into {table_meta} (id, url_id, lang, title, desc) \
        values (null, ?, ?, ?, ?)",
        table_meta = TABLE_META
    );
    let res = sqlx::query(&sql_i)
        .bind(&m.url_id)
        .bind(&m.lang)
        .bind(&m.title)
        .bind(&m.desc)
        .execute(&mut db.dbc)
        .await?;

    if db.update_change {
        db_mark_change(db).await?;
    }
    Ok(res.rows_affected())
}
// EOF
