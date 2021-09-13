// meta_util.rs

use chrono::*;
use log::*;
use std::error::Error;
use std::{thread, time};
use webpage::{Webpage, WebpageOptions};

use super::*;

const STR_NA: &str = "(N/A)";
const STR_ERR: &str = "(Error)";

// const TS_FMT: &str = "%Y-%m-%d %H:%M:%S";
const SHORT_TS_YEAR_FMT: &str = "%Y %b %d %H:%M";

pub fn update_meta(db: &DbCtx, url_id: i64, url: &str) -> Result<(), Box<dyn Error>> {
    let w_opt = WebpageOptions {
        allow_insecure: true,
        timeout: time::Duration::new(5, 0),
        ..Default::default()
    };
    info!("Fetching URL {}", url);
    let lang: String;
    let title: String;
    let desc: String;
    match Webpage::from_url(url, w_opt) {
        Ok(pageinfo) => {
            lang = pageinfo.html.language.unwrap_or_else(|| STR_NA.to_owned());
            title = pageinfo.html.title.unwrap_or_else(|| STR_NA.to_owned());
            desc = pageinfo
                .html
                .description
                .unwrap_or_else(|| STR_NA.to_owned());
        }
        Err(e) => {
            lang = STR_ERR.into();
            title = format!("(Error: {})", e);
            desc = STR_ERR.into();
        }
    }
    info!(
        "URL metadata:\nid: {}\nurl: {}\nlang: {}\ntitle: {}\ndesc: {}",
        url_id, url, &lang, &title, &desc
    );
    let m = MetaCtx {
        url_id,
        lang: &lang,
        title: &title,
        desc: &desc,
    };
    db_add_meta(db, &m)?;
    info!("Inserted row.");
    Ok(())
}

pub fn process_backlog(db: &DbCtx) -> Result<(), Box<dyn Error>> {
    let sql_hist = format!(
        "select url.id, url.url, url.seen \
        from {table_url} url \
        where not exists ( \
            select null \
            from {table_meta} meta \
            where url.id = meta.url_id \
        ) \
        order by seen asc \
        limit 10",
        table_url = db.table_url,
        table_meta = db.table_meta,
    );

    loop {
        info!("Reading history...");
        let mut urls = Vec::with_capacity(12);
        let mut ids = Vec::with_capacity(12);
        let mut seen_i = 0;
        {
            let mut st_h = db.dbc.prepare(&sql_hist)?;
            let mut rows = st_h.query([])?;
            while let Some(row) = rows.next()? {
                ids.push(row.get::<usize, i64>(0)?);
                urls.push(row.get::<usize, String>(1)?);
                seen_i = row.get::<usize, i64>(2)?;
            }
        }
        if urls.len() < 10 {
            break;
        }
        let first_seen_str = Local
            .from_utc_datetime(&NaiveDateTime::from_timestamp(seen_i, 0))
            .format(SHORT_TS_YEAR_FMT)
            .to_string();
        info!("*** PROCESSING *** at {}", &first_seen_str);
        for i in 0..ids.len() {
            if let Err(e) = update_meta(db, ids[i], &urls[i]) {
                error!("URL error: {}", e);
            }
            thread::sleep(time::Duration::new(0, 100_000_000));
        }
    }
    Ok(())
}

pub fn process_live(db: &DbCtx) -> Result<(), Box<dyn Error>> {
    info!("Starting live processing...");
    let sql_nometa = format!(
        "select url.id, url.url \
        from {table_url} url \
        where not exists ( \
            select null \
            from {table_meta} meta \
            where url.id = meta.url_id \
        ) \
        order by seen desc \
        limit 42",
        table_url = db.table_url,
        table_meta = db.table_meta,
    );

    let mut latest_ts: i64 = 0;
    loop {
        thread::sleep(time::Duration::new(2, 0));
        let db_ts = db_last_change(db)?;
        if db_ts <= latest_ts {
            trace!("Nothing new in DB.");
            continue;
        }
        latest_ts = db_ts;

        // Ha! There IS something new in db.
        info!("New stuff, waking up!");
        {
            let mut ids = Vec::with_capacity(50);
            let mut urls = Vec::with_capacity(50);
            {
                let mut st_nometa = db.dbc.prepare(&sql_nometa)?;
                let mut rows = st_nometa.query([])?;
                while let Some(row) = rows.next()? {
                    ids.push(row.get::<usize, i64>(0)?);
                    urls.push(row.get::<usize, String>(1)?);
                }
            }
            for i in 0..ids.len() {
                if let Err(e) = update_meta(db, ids[i], &urls[i]) {
                    error!("URL meta update error: {}", e);
                }
            }
        }
        info!("Some metadata updated, waiting for new stuff...");
    }
}
// EOF
