// urllog-meta.rs

use log::*;
use std::{thread, time};
use structopt::StructOpt;
use webpage::{Webpage, WebpageOptions};

use urlharvest::*;

const STR_NA: &str = "(N/A)";
const STR_ERR: &str = "(Error)";
const BATCH_SIZE: usize = 10;
const SLEEP_POLL: u64 = 2;

#[allow(dead_code)]
#[derive(Debug, PartialEq)]
enum ProcessMode {
    Backlog,
    Live,
}

fn main() -> anyhow::Result<()> {
    let mut opts = OptsMeta::from_args();
    opts.finish()?;
    start_pgm(&opts.c, "URL metadata updater");
    let mut db = start_db(&opts.c)?;
    db.update_change = true;

    if opts.backlog {
        process_meta(&db, ProcessMode::Backlog)
    } else {
        process_meta(&db, ProcessMode::Live)
    }
}

fn process_meta(db: &DbCtx, mode: ProcessMode) -> anyhow::Result<()> {
    let order = match mode {
        ProcessMode::Backlog => "asc",
        ProcessMode::Live => "desc",
    };
    // find the lines in {table_url} where corresponding line does not exist
    // in table {url_meta}
    let sql_nometa = format!(
        "select url.id, url.url, url.seen \
        from {table_url} url \
        where not exists ( \
            select null \
            from {table_meta} meta \
            where url.id = meta.url_id \
        ) \
        order by seen {order} \
        limit {sz}",
        table_url = db.table_url,
        table_meta = db.table_meta,
        order = order,
        sz = BATCH_SIZE,
    );

    let mut latest_ts: i64 = 0;
    loop {
        let db_ts = db_last_change(db)?;
        if mode == ProcessMode::Live && db_ts <= latest_ts {
            trace!("Nothing new in DB.");
            thread::sleep(time::Duration::new(SLEEP_POLL, 0));
            continue;
        }
        latest_ts = db_ts;

        info!("Starting {:?} processing", mode);
        {
            let mut ids = Vec::with_capacity(BATCH_SIZE);
            let mut urls = Vec::with_capacity(BATCH_SIZE);
            let mut seen_i = 0;
            {
                let mut st_nometa = db.dbc.prepare(&sql_nometa)?;
                let mut rows = st_nometa.query([])?;
                while let Some(row) = rows.next()? {
                    ids.push(row.get::<usize, i64>(0)?);
                    urls.push(row.get::<usize, String>(1)?);
                    seen_i = row.get::<usize, i64>(2)?;
                }
            }
            if seen_i > 0 {
                info!("*** PROCESSING *** at {}", &ts_y_short(seen_i));
            }
            for i in 0..ids.len() {
                if let Err(e) = update_meta(db, ids[i], &urls[i]) {
                    error!("URL meta update error: {}", e);
                }
            }
            if mode == ProcessMode::Backlog && ids.len() < BATCH_SIZE {
                // Backlog processing ends eventually, live processing does not.
                break;
            }
        }
        info!("Polling updates");
    }
    Ok(())
}

pub fn update_meta(db: &DbCtx, url_id: i64, url: &str) -> anyhow::Result<()> {
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
// EOF
