// bin/urllog_generator.rs

use anyhow::{anyhow, bail};
use chrono::*;
use enum_iterator::{all, Sequence};
use futures::TryStreamExt; // provides `try_next`
use log::*;
use sqlx::FromRow;
use std::{collections::HashMap, fmt, fs};
use structopt::StructOpt;
use tera::Tera;
use tokio::time::{sleep, Duration};
use urlharvest::*;

const URL_EXPIRE: i64 = 7 * 24 * 3600; // A week in seconds
const VEC_SZ: usize = 4096;
const TPL_SUFFIX: &str = ".tera";
const SLEEP_IDLE: u64 = 10;
const SLEEP_BUSY: u64 = 2;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::from_args();
    opts.finish()?;
    opts.start_pgm(env!("CARGO_BIN_NAME"));
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{cfg:#?}");

    let mut db = start_db(&cfg).await?;

    let tera_dir = &cfg.template_dir;
    info!("Template directory: {tera_dir}");
    let tera = match Tera::new(&format!("{tera_dir}/*.tera")) {
        Ok(t) => t,
        Err(e) => {
            return Err(anyhow!("Tera template parsing error: {e:?}"));
        }
    };
    if tera.get_template_names().count() < 1 {
        error!("No templates found. Exit.");
        bail!("Templates not found");
    }
    info!(
        "Found templates: [{}]",
        tera.get_template_names().collect::<Vec<_>>().join(", ")
    );

    let mut latest_db: i64 = 0;
    loop {
        let db_ts = db_last_change(&mut db).await?;
        if db_ts <= latest_db {
            trace!("Nothing new in DB.");
            sleep(Duration::new(SLEEP_IDLE, 0)).await;
            continue;
        }
        latest_db = db_ts;

        let mut now = Utc::now();
        let ts_limit = now.timestamp() - URL_EXPIRE;
        info!("Generating URL logs starting from {}", ts_limit.ts_long());
        let ctx = generate_ctx(&mut db, ts_limit).await?;
        info!(
            "Database read took {} ms.",
            Utc::now().signed_duration_since(now).num_milliseconds()
        );

        now = Utc::now();
        for template in tera.get_template_names() {
            let basename = template.strip_suffix(TPL_SUFFIX).unwrap_or(template);
            let filename_out = format!("{}/{basename}", &cfg.html_dir);
            let filename_tmp = format!(
                "{filename_out}.{}.{}.tmp",
                std::process::id(),
                Utc::now().timestamp_nanos()
            );
            info!("Generating {filename_out} from {template}");
            let template_output = tera.render(template, &ctx)?;
            fs::write(&filename_tmp, template_output)?;
            fs::rename(&filename_tmp, &filename_out)?;
        }
        info!(
            "Template rendering took {} ms.",
            Utc::now().signed_duration_since(now).num_milliseconds()
        );
        sleep(Duration::new(SLEEP_BUSY, 0)).await;
    }
}

#[derive(Debug, FromRow)]
struct DbRead {
    id: i64,
    seen_first: i64,
    seen_last: i64,
    seen_cnt: i64,
    channel: String,
    nick: String,
    url: String,
    title: String,
}

const CTX_NUM: usize = 32;

#[allow(non_camel_case_types)]
#[derive(Debug, Eq, Hash, Sequence, PartialEq)]
enum CtxData {
    id,
    seen_first,
    seen_last,
    seen_cnt,
    channel,
    nick,
    url,
    title,
    uniq_id,
    uniq_seen_first,
    uniq_seen_last,
    uniq_seen_cnt,
    uniq_channel,
    uniq_nick,
    uniq_url,
    uniq_title,
}
// with this we get to_string() for free
impl fmt::Display for CtxData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format!("{self:?}"))
    }
}

const SQL_URL: &str = "select min(url.id) as id, min(seen) as seen_first, max(seen) as seen_last, count(seen) as seen_cnt, \
    channel, nick, url, url_meta.title from url \
    inner join url_meta on url_meta.url_id = url.id \
    group by channel, url \
    having max(seen) > ? \
    order by max(seen) desc";

const SQL_UNIQ: &str = "select min(url.id) as id, min(seen) as seen_first, max(seen) as seen_last, count(seen) as seen_cnt, \
    group_concat(channel, ' ') as channel, group_concat(nick, ' ') as nick, \
    url, url_meta.title from url \
    inner join url_meta on url_meta.url_id = url.id \
    group by url \
    having max(seen) > ? \
    order by max(seen) desc";

async fn generate_ctx(db: &mut DbCtx, ts_limit: i64) -> anyhow::Result<tera::Context> {
    let mut data: HashMap<CtxData, Vec<String>> = HashMap::with_capacity(CTX_NUM);
    for k in all::<CtxData>() {
        let v: Vec<String> = Vec::with_capacity(VEC_SZ);
        data.insert(k, v);
    }

    let mut ctx = tera::Context::new();
    ctx.insert("last_change", &Utc::now().timestamp().ts_long());

    let mut n_rows: usize = 0;

    let mut st_url = sqlx::query_as::<_, DbRead>(SQL_URL)
        .bind(ts_limit)
        .fetch(&mut db.dbc);

    while let Some(row) = st_url.try_next().await? {
        data.get_mut(&CtxData::id)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.id.to_string());
        data.get_mut(&CtxData::seen_first)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.seen_first.ts_short_y());
        data.get_mut(&CtxData::seen_last)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.seen_last.ts_short());
        data.get_mut(&CtxData::seen_cnt)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.seen_cnt.to_string());
        data.get_mut(&CtxData::channel)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.channel.esc_et_lt_gt());
        data.get_mut(&CtxData::nick)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.nick.esc_et_lt_gt());
        data.get_mut(&CtxData::url)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.url.esc_quot());
        data.get_mut(&CtxData::title)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.title.esc_et_lt_gt());
        n_rows += 1;
    }
    drop(st_url);

    info!("Got {n_rows} rows.");
    ctx.insert("n_rows", &n_rows);

    n_rows = 0;

    let mut st_uniq = sqlx::query_as::<_, DbRead>(SQL_UNIQ)
        .bind(ts_limit)
        .fetch(&mut db.dbc);

    while let Some(row) = st_uniq.try_next().await? {
        data.get_mut(&CtxData::uniq_id)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.id.to_string());
        data.get_mut(&CtxData::uniq_seen_first)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.seen_first.ts_short_y());
        data.get_mut(&CtxData::uniq_seen_last)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.seen_last.ts_short());
        data.get_mut(&CtxData::uniq_seen_cnt)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.seen_cnt.to_string());
        data.get_mut(&CtxData::uniq_channel)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.channel.esc_et_lt_gt().sort_dedup_br());
        data.get_mut(&CtxData::uniq_nick)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.nick.esc_et_lt_gt().sort_dedup_br());
        data.get_mut(&CtxData::uniq_url)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.url.esc_quot());
        data.get_mut(&CtxData::uniq_title)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.title.esc_et_lt_gt());
        n_rows += 1;
    }
    drop(st_uniq);

    info!("Got {n_rows} uniq rows.");
    ctx.insert("uniq_n_rows", &n_rows);

    for k in all::<CtxData>() {
        ctx.insert(
            k.to_string(),
            data.get(&k).ok_or_else(|| anyhow!("no data"))?,
        );
    }

    Ok(ctx)
}
// EOF
