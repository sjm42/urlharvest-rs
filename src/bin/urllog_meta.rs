// bin/urllog_meta.rs

use clap::Parser;
use futures::TryStreamExt;
use log::*;
use tokio::time::{sleep, Duration};

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
    id: i32,
    url: String,
    seen: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::parse();
    opts.finish()?;
    opts.start_pgm(env!("CARGO_BIN_NAME"));
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{:#?}", &cfg);

    let mut dbc = start_db(&cfg).await?;
    dbc.update_change = true;

    if opts.meta_backlog {
        process_meta(&dbc, ProcessMode::Backlog).await
    } else {
        process_meta(&dbc, ProcessMode::Live).await
    }
}

async fn process_meta(dbc: &DbCtx, mode: ProcessMode) -> anyhow::Result<()> {
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
            let db_ts = db_last_change(dbc).await?;
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
                let mut st_nometa = sqlx::query_as::<_, NoMeta>(&sql_nometa).fetch(&dbc.dbc);
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
                if let Err(e) = update_meta(dbc, id.0, &id.1).await {
                    error!("URL meta update error: {e:?}");
                }
            }
        }
        info!("Polling updates");
    }
    Ok(())
}

pub async fn update_meta(dbc: &DbCtx, url_id: i32, url_s: &str) -> anyhow::Result<()> {
    let (mut title, lang, descr) = match get_text_body(url_s).await {
        Err(e) => (
            format!("(URL fetch error: {e:?})"),
            STR_ERR.into(),
            STR_ERR.into(),
        ),
        Ok(None) => (STR_NA.into(), STR_NA.into(), STR_NA.into()),
        Ok(Some((body, _ct))) => match webpage::HTML::from_string(body, None) {
            Err(e) => (
                format!("(Webpage HTML error: {e:?})"),
                STR_ERR.into(),
                STR_ERR.into(),
            ),
            Ok(html) => (
                html.title.unwrap_or_else(|| STR_NA.to_owned()),
                html.language.unwrap_or_else(|| STR_NA.to_owned()),
                html.description.unwrap_or_else(|| STR_NA.to_owned()),
            ),
        },
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
        "URL metadata:\nid: {url_id}\nurl: {url_s}\nlang: {lang}\ntitle: {title}\ndescr: {descr}",
    );
    info!(
        "Inserted {} row(s)",
        db_add_meta(
            dbc,
            &MetaCtx {
                url_id,
                lang,
                title,
                descr,
            },
        )
        .await?
    );

    Ok(())
}
// EOF
