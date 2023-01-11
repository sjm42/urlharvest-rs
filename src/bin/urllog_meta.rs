// bin/urllog_meta.rs

use futures::TryStreamExt;
use log::*;
use regex::Regex;
use std::{thread, time};
use structopt::StructOpt;
use url::Url;
use webpage::{Webpage, WebpageOptions}; // provides `try_next`

use urlharvest::*;

const STR_NA: &str = "(N/A)";
const STR_ERR: &str = "(Error)";
const BATCH_SIZE: usize = 10;
const SLEEP_POLL: u64 = 2;

#[derive(Debug, PartialEq, Eq)]
enum ProcessMode {
    Backlog,
    Live,
}

#[derive(Debug, sqlx::FromRow)]
struct NoMeta {
    id: i64,
    url: String,
    seen: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::from_args();
    opts.finish()?;
    opts.start_pgm(env!("CARGO_BIN_NAME"));
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{:#?}", &cfg);

    let mut db = start_db(&cfg).await?;
    db.update_change = true;

    if opts.meta_backlog {
        process_meta(&mut db, ProcessMode::Backlog).await
    } else {
        process_meta(&mut db, ProcessMode::Live).await
    }
}

async fn process_meta(db: &mut DbCtx, mode: ProcessMode) -> anyhow::Result<()> {
    let order = match mode {
        ProcessMode::Backlog => "asc",
        ProcessMode::Live => "desc",
    };
    // find the lines in {table_url} where corresponding line does not exist
    // in table {url_meta}
    let sql_nometa = format!(
        "select url.id, url.url, url.seen \
        from url \
        where not exists ( \
            select null \
            from url_meta \
            where url.id = url_meta.url_id \
        ) \
        order by seen {order} \
        limit {sz}",
        sz = BATCH_SIZE,
    );

    let mut latest_ts: i64 = 0;
    loop {
        let db_ts = db_last_change(db).await?;
        if mode == ProcessMode::Live && db_ts <= latest_ts {
            trace!("Nothing new in DB.");
            thread::sleep(time::Duration::new(SLEEP_POLL, 0));
            continue;
        }
        latest_ts = db_ts;

        info!("Starting {mode:?} processing");
        {
            let mut ids = Vec::with_capacity(BATCH_SIZE);
            let mut urls = Vec::with_capacity(BATCH_SIZE);
            let mut seen_i = 0;
            {
                let mut st_nometa = sqlx::query_as::<_, NoMeta>(&sql_nometa).fetch(&mut db.dbc);
                while let Some(row) = st_nometa.try_next().await? {
                    ids.push(row.id);
                    urls.push(row.url);
                    seen_i = row.seen;
                }
            }
            if seen_i > 0 {
                info!("*** PROCESSING *** at {}", &seen_i.ts_short_y());
            }
            for i in 0..ids.len() {
                if let Err(e) = update_meta(db, ids[i], &urls[i]).await {
                    error!("URL meta update error: {e:?}");
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

pub async fn update_meta(db: &mut DbCtx, url_id: i64, url_s: &str) -> anyhow::Result<()> {
    static mut WS_RE: Option<Regex> = None;

    // We are called sequentially and thus no race condition here.
    unsafe {
        if WS_RE.is_none() {
            // pre-compile whitespace regex once
            WS_RE = Some(Regex::new(r"\s+")?);
        }
    }

    if let Ok(url) = Url::parse(url_s) {
        // Now we should have a canonical url, IDN handled etc.
        let url_c = String::from(url);

        let w_opt = WebpageOptions {
            allow_insecure: true,
            timeout: time::Duration::new(5, 0),
            ..Default::default()
        };
        info!("Fetching URL {url_c}");
        let lang: String;
        let title: String;
        let desc: String;
        match Webpage::from_url(&url_c, w_opt) {
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
                title = format!("(Error: {e:?})");
                desc = STR_ERR.into();
            }
        }

        // Cleanup the title
        let mut title_c = unsafe {
            WS_RE
                .as_ref()
                .unwrap()
                .replace_all(&title, " ")
                .trim()
                .to_string()
        };
        if title_c.len() > 42 {
            let mut i = 38;
            loop {
                // find a UTF-8 code point boundary to safely split at
                if title_c.is_char_boundary(i) {
                    break;
                }
                i += 1;
            }
            let (s1, _) = title_c.split_at(i);
            title_c = format!("{}...", s1);
        }

        info!(
            "URL metadata:\nid: {url_id}\nurl: {url_c}\nlang: {lang}\ntitle: {title_c}\ndesc: {desc}",
        );
        info!(
            "Inserted {} row(s)",
            db_add_meta(
                db,
                &MetaCtx {
                    url_id,
                    lang,
                    title: title_c,
                    desc,
                },
            )
            .await?
        );
    }
    Ok(())
}
// EOF
