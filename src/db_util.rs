// db_util.rs

use chrono::*;
use log::*;
use rusqlite::{named_params, Connection};
use std::error::Error;
use std::{thread, time};

const RETRY_CNT: usize = 5;
const RETRY_SLEEP: u64 = 1;

pub struct DbCtx<'c, 's> {
    pub dbc: &'c Connection,
    pub table_url: &'s str,
    pub table_meta: &'s str,
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

fn table_exist(dbc: &Connection, table: &str) -> Result<bool, Box<dyn Error>> {
    let mut st = dbc.prepare(
        "select count(name) from sqlite_master \
        where type='table' and name=?",
    )?;
    let n: usize = st.query([table])?.next()?.unwrap().get(0)?;
    Ok(n == 1)
}

pub fn db_init(db: &DbCtx) -> Result<(), Box<dyn Error>> {
    if !table_exist(db.dbc, db.table_url)? {
        info!("Creating table {}", db.table_url);
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
            table_url = db.table_url,
        );
        debug!("SQL:\n{}", &sql);
        db.dbc.execute_batch(&sql)?;
    }
    if !table_exist(db.dbc, db.table_meta)? {
        info!("Creating table {}", db.table_meta);
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
            table_url = db.table_url,
            table_meta = db.table_meta,
        );
        info!("Creating new DB table+indexes.");
        debug!("SQL:\n{}", &sql);
        db.dbc.execute_batch(&sql)?;
    }
    Ok(())
}

pub fn db_last_change(db: &DbCtx) -> Result<i64, Box<dyn Error>> {
    let sql_ts = format!(
        "select last from {table}_changed limit 1",
        table = db.table_url
    );
    let mut st_ts = db.dbc.prepare(&sql_ts)?;
    Ok(st_ts.query_row([], |r| r.get::<usize, i64>(0))?)
}

pub fn db_mark_change(db: &DbCtx) -> Result<(), Box<dyn Error>> {
    let sql = format!(
        "update {table}_changed set last={ts};",
        table = db.table_url,
        ts = Utc::now().timestamp()
    );
    db.dbc.execute_batch(&sql)?;
    Ok(())
}

pub fn db_add_url(db: &DbCtx, ur: &UrlCtx) -> Result<(), Box<dyn Error>> {
    let sql_i = format!(
        "insert into {table} (id, seen, channel, nick, url) \
        values (null, :ts, :ch, :ni, :ur)",
        table = db.table_url
    );
    let mut st_i = db.dbc.prepare(&sql_i)?;
    let mut retry = 0;
    while retry < RETRY_CNT {
        match st_i
            .execute(named_params! {":ts": ur.ts, ":ch": ur.chan, ":ni": ur.nick, ":ur": ur.url})
        {
            Ok(n) => {
                info!("Inserted {} row", n);
                retry = 0;
                break;
            }
            Err(e) => {
                error!("Insert failed: {}", e);
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
        error!("GAVE UP after {} retries.", RETRY_CNT);
    }
    Ok(())
}

pub fn db_add_meta(db: &DbCtx, m: &MetaCtx) -> Result<(), Box<dyn Error>> {
    let sql_i = format!(
        "insert into {} (id, url_id, lang, title, desc) \
        values (null, :ur, :la, :ti, :de)",
        db.table_meta
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
