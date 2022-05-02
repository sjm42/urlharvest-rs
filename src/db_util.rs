// db_util.rs

use chrono::*;
use log::*;
use rusqlite::{named_params, Connection};
use std::{thread, time};

use crate::*;

const RETRY_CNT: usize = 5;
const RETRY_SLEEP: u64 = 1;

pub struct DbCtx {
    pub dbc: Connection,
    pub update_change: bool,
}

pub struct UrlCtx<'a> {
    pub ts: i64,
    pub chan: &'a str,
    pub nick: &'a str,
    pub url: &'a str,
}

pub struct MetaCtx<'a> {
    pub url_id: i64,
    pub lang: &'a str,
    pub title: &'a str,
    pub desc: &'a str,
}

fn table_exist(dbc: &Connection, table: &str) -> anyhow::Result<bool> {
    let mut st = dbc.prepare(
        "select count(name) from sqlite_master \
        where type='table' and name=?",
    )?;
    let n: usize = st.query([table])?.next()?.unwrap().get(0)?;
    Ok(n == 1)
}

pub fn db_init(db: &DbCtx) -> anyhow::Result<()> {
    if !table_exist(&db.dbc, TABLE_URL)? {
        info!("Creating table {TABLE_URL}");
        let sql = format!(
            "begin; \
            create table {table_url} ( \
            id integer primary key autoincrement, \
            seen integer, \
            channel text, \
            nick text, \
            url text); \
            create index {table_url}_seen on {table_url}(seen); \
            create index {table_url}_channel on {table_url}(channel); \
            create index {table_url}_nick on {table_url}(nick); \
            create index {table_url}_url on {table_url}(url); \
            create table {table_url}_changed (last integer); \
            insert into {table_url}_changed values (0); \
            commit;",
            table_url = TABLE_URL
        );
        debug!("SQL:\n{sql}");
        db.dbc.execute_batch(&sql)?;
    }
    if !table_exist(&db.dbc, TABLE_META)? {
        info!("Creating table {TABLE_META}");
        let sql = format!(
            "begin; \
            create table {table_meta} ( \
            id integer primary key autoincrement, \
            url_id integer unique not null, \
            lang text, \
            title text, \
            desc text, \
            foreign key(url_id) references {table_url}(id) \
            on update cascade \
            on delete cascade \
            ); \
            create index {table_meta}_urlid on {table_meta}(url_id); \
            commit;",
            table_url = TABLE_URL,
            table_meta = TABLE_META
        );
        info!("Creating new DB table+indexes.");
        debug!("SQL:\n{sql}");
        db.dbc.execute_batch(&sql)?;
    }
    Ok(())
}

pub fn db_last_change(db: &DbCtx) -> anyhow::Result<i64> {
    let sql_ts = format!("select last from {table} limit 1", table = TABLE_CHANGED);
    let mut st_ts = db.dbc.prepare(&sql_ts)?;
    Ok(st_ts.query_row([], |r| r.get::<usize, i64>(0))?)
}

pub fn db_mark_change(db: &DbCtx) -> anyhow::Result<()> {
    let sql = format!(
        "update {table} set last={ts};",
        table = TABLE_CHANGED,
        ts = Utc::now().timestamp()
    );
    Ok(db.dbc.execute_batch(&sql)?)
}

pub fn db_add_url(db: &DbCtx, ur: UrlCtx) -> anyhow::Result<()> {
    let sql_i = format!(
        "insert into {table} (id, seen, channel, nick, url) \
        values (null, :ts, :ch, :ni, :ur)",
        table = TABLE_URL
    );
    let mut st_i = db.dbc.prepare(&sql_i)?;
    let mut retry = 0;
    while retry < RETRY_CNT {
        match st_i
            .execute(named_params! {":ts": ur.ts, ":ch": ur.chan, ":ni": ur.nick, ":ur": ur.url})
        {
            Ok(n) => {
                info!("Inserted {n} row");
                retry = 0;
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
        db_mark_change(db)?;
    }
    if retry > 0 {
        error!("GAVE UP after {RETRY_CNT} retries.");
    }
    Ok(())
}

pub fn db_add_meta(db: &DbCtx, m: MetaCtx) -> anyhow::Result<()> {
    let sql_i = format!(
        "insert into {table_meta} (id, url_id, lang, title, desc) \
        values (null, :ur, :la, :ti, :de)",
        table_meta = TABLE_META
    );
    let mut st_i = db.dbc.prepare(&sql_i)?;
    st_i.execute(named_params! {
    ":ur": m.url_id,
    ":la": m.lang,
    ":ti": m.title,
    ":de": m.desc,
    })?;
    if db.update_change {
        db_mark_change(db)?;
    }
    Ok(())
}
// EOF
