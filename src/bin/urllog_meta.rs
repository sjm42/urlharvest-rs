// bin/urllog_meta.rs

use clap::Parser;
use futures::TryStreamExt;
use sqlx::postgres::PgListener;

use urlharvest::*;

const STR_NA: &str = "(N/A)";
const STR_ERR: &str = "(Error)";
const BATCH_SIZE: usize = 10;
const TITLE_MAX_LEN: usize = 400;

macro_rules! sql_nometa {
    ($order:literal) => {
        concat!(
            "select url.id, url.url, url.seen ",
            "from url ",
            "where not exists (",
            "select null ",
            "from url_meta ",
            "where url.id = url_meta.url_id ",
            ") ",
            "order by seen ",
            $order,
            " limit $1"
        )
    };
}

const SQL_NOMETA_ASC: &str = sql_nometa!("asc");
const SQL_NOMETA_DESC: &str = sql_nometa!("desc");

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
    opts.finalize()?;
    opts.start_pgm(env!("CARGO_BIN_NAME"));
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{:#?}", &cfg);

    let dbc = start_db(&cfg).await?;

    if opts.meta_backlog {
        process_meta(&dbc, ProcessMode::Backlog).await
    } else {
        process_meta(&dbc, ProcessMode::Live).await
    }
}

async fn process_meta(dbc: &DbCtx, mode: ProcessMode) -> anyhow::Result<()> {
    let sql_nometa = match mode {
        ProcessMode::Backlog => SQL_NOMETA_ASC,
        ProcessMode::Live => SQL_NOMETA_DESC,
    };

    let mut listener = match mode {
        ProcessMode::Backlog => None,
        ProcessMode::Live => {
            let mut listener = PgListener::connect_with(&dbc.dbc).await?;
            listener.listen(DB_CHANGE_CHANNEL).await?;
            Some(listener)
        }
    };

    loop {
        info!("Starting {mode:?} processing");
        loop {
            let mut ids = Vec::with_capacity(BATCH_SIZE);
            let mut seen_i = 0;
            {
                let mut st_nometa = sqlx::query_as::<_, NoMeta>(sql_nometa)
                    .bind(BATCH_SIZE as i64)
                    .fetch(&dbc.dbc);
                while let Some(row) = st_nometa.try_next().await? {
                    ids.push((row.id, row.url));
                    seen_i = row.seen;
                }
            }
            if ids.is_empty() {
                break;
            }
            if seen_i > 0 {
                info!("*** PROCESSING *** at {}", &seen_i.ts_short_y());
            }
            for id in &ids {
                update_meta(dbc, id.0, &id.1).await?;
            }
        }

        if mode == ProcessMode::Backlog {
            // Backlog processing ends once all currently missing metadata is handled.
            break;
        }

        info!("Waiting for database updates");
        let listener = listener.as_mut().expect("live mode has a listener");
        match listener.try_recv().await? {
            Some(notification) => trace!(
                "Database update notification from backend {}",
                notification.process_id()
            ),
            None => warn!("Database listener reconnected; reconciling current state"),
        }
        while listener.next_buffered().is_some() {}
    }
    Ok(())
}

pub async fn update_meta(dbc: &DbCtx, url_id: i32, url_s: &str) -> anyhow::Result<()> {
    let (mut title, lang, descr) = match get_text_body(url_s).await {
        Err(e) => (format!("(URL fetch error: {e:?})"), STR_ERR.into(), STR_ERR.into()),
        Ok(None) => (STR_NA.into(), STR_NA.into(), STR_NA.into()),
        Ok(Some((body, _ct))) => match webpage::HTML::from_string(body, None) {
            Err(e) => (format!("(Webpage HTML error: {e:?})"), STR_ERR.into(), STR_ERR.into()),
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

    info!("URL metadata:\nid: {url_id}\nurl: {url_s}\nlang: {lang}\ntitle: {title}\ndescr: {descr}",);
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
