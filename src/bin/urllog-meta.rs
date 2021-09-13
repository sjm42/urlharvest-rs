// main.rs

use log::*;
use rusqlite::Connection;
use std::error::Error;
use std::{env, thread, time};
use structopt::StructOpt;
use webpage::{Webpage, WebpageOptions};

use urlharvest::*;

const STR_NA: &str = "(N/A)";
const STR_ERR: &str = "(Error)";

#[derive(Debug, Clone, StructOpt)]
pub struct GlobalOptions {
    #[structopt(short, long)]
    pub debug: bool,
    #[structopt(short, long)]
    pub trace: bool,
    #[structopt(long, default_value = "$HOME/urllog/data/urllog2.db")]
    pub db_file: String,
    #[structopt(long, default_value = "urllog2")]
    pub table_url: String,
    #[structopt(long, default_value = "urlmeta")]
    pub table_meta: String,
}

fn update_meta(dbc: &DbCtx, url_id: i64, url: &str) -> Result<(), Box<dyn Error>> {
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
    db_add_meta(dbc, &m)?;
    info!("Inserted row.");
    Ok(())
}

#[allow(unreachable_code)]
fn main() -> Result<(), Box<dyn Error>> {
    let home = env::var("HOME")?;
    let mut opt = GlobalOptions::from_args();
    opt.db_file = opt.db_file.replace("$HOME", &home);
    let loglevel = if opt.trace {
        LevelFilter::Trace
    } else if opt.debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };

    env_logger::Builder::new()
        .filter_level(loglevel)
        .format_timestamp_secs()
        .init();
    info!("Starting up URL metadata updater");
    debug!("Git branch: {}", env!("GIT_BRANCH"));
    debug!("Git commit: {}", env!("GIT_COMMIT"));
    debug!("Source timestamp: {}", env!("SOURCE_TIMESTAMP"));
    debug!("Compiler version: {}", env!("RUSTC_VERSION"));
    debug!("Global config: {:?}", opt);

    let dbc = &Connection::open(&opt.db_file)?;
    let table_url = &opt.table_url;
    let table_meta = &opt.table_meta;
    let db = DbCtx {
        dbc,
        table_url,
        table_meta,
        update_change: true,
    };
    db_init(&db)?;

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
        table_url = table_url,
        table_meta = table_meta,
    );

    let mut latest_ts: i64 = 0;
    loop {
        thread::sleep(time::Duration::new(2, 0));
        let db_ts = db_last_change(&db)?;
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
                let mut st_nometa = dbc.prepare(&sql_nometa)?;
                let mut rows = st_nometa.query([])?;
                while let Some(row) = rows.next()? {
                    ids.push(row.get::<usize, i64>(0)?);
                    urls.push(row.get::<usize, String>(1)?);
                }
            }
            for i in 0..ids.len() {
                if let Err(e) = update_meta(&db, ids[i], &urls[i]) {
                    error!("URL meta update error: {}", e);
                }
            }
        }
        info!("Some metadata updated, waiting for new stuff...");
    }
    Ok(())
}
// EOF
