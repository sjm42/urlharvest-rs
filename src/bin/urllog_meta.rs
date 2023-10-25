// bin/urllog_meta.rs

use futures::TryStreamExt;
use log::*;
use regex::Regex;
use structopt::StructOpt;
use tokio::time::{sleep, Duration};
use url::Url;
use webpage::{Webpage, WebpageOptions}; // provides `try_next`

use urlharvest::*;

const STR_NA: &str = "(N/A)";
const STR_ERR: &str = "(Error)";
const BATCH_SIZE: usize = 10;
const SLEEP_POLL: u64 = 2;
const TITLE_MAX_LEN: usize = 400;

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
        if mode == ProcessMode::Live {
            let db_ts = db_last_change(db).await?;
            if db_ts <= latest_ts {
                trace!("Nothing new in DB.");
                sleep(Duration::new(SLEEP_POLL, 0)).await;
                continue;
            }
            latest_ts = db_ts;
        }

        info!("Starting {mode:?} processing");
        {
            let mut ids = Vec::with_capacity(BATCH_SIZE);
            let mut seen_i = 0;
            {
                let mut st_nometa = sqlx::query_as::<_, NoMeta>(&sql_nometa).fetch(&mut db.dbc);
                while let Some(row) = st_nometa.try_next().await? {
                    ids.push((row.id, row.url));
                    seen_i = row.seen;
                }
            }
            if mode == ProcessMode::Backlog && ids.is_empty() {
                // Backlog processing ends eventually, live processing does not.
                break;
            }
            if seen_i > 0 {
                info!("*** PROCESSING *** at {}", &seen_i.ts_short_y());
            }
            for id in &ids {
                if let Err(e) = update_meta(db, id.0, &id.1).await {
                    error!("URL meta update error: {e:?}");
                }
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

        let mut w_opt = WebpageOptions::default();
        w_opt.allow_insecure = true;
        w_opt.timeout = Duration::new(5, 0);
        info!("Fetching URL {url_c}");
        let (mut title, lang, desc) = match Webpage::from_url(&url_c, w_opt) {
            Ok(pageinfo) => (
                pageinfo.html.title.unwrap_or_else(|| STR_NA.to_owned()),
                pageinfo.html.language.unwrap_or_else(|| STR_NA.to_owned()),
                pageinfo
                    .html
                    .description
                    .unwrap_or_else(|| STR_NA.to_owned()),
            ),
            Err(e) => (format!("(Error: {e:?})"), STR_ERR.into(), STR_ERR.into()),
        };

        // Cleanup the title
        title = title.ws_collapse();
        let len = title.len();
        if len > TITLE_MAX_LEN {
            let mut i = TITLE_MAX_LEN - 8;
            loop {
                // find a UTF-8 code point boundary to safely split at
                if title.is_char_boundary(i) || i >= len {
                    break;
                }
                i += 1;
            }
            if i < len {
                let (s1, _) = title.split_at(i);
                title = format!("{}...", s1);
            } else {
                error!("Did not find char boundary, should never happen.");
            }
        }

        info!(
            "URL metadata:\nid: {url_id}\nurl: {url_c}\nlang: {lang}\ntitle: {title}\ndesc: {desc}",
        );
        info!(
            "Inserted {} row(s)",
            db_add_meta(
                db,
                &MetaCtx {
                    url_id,
                    lang,
                    title,
                    desc,
                },
            )
            .await?
        );
    }
    Ok(())
}
// EOF
