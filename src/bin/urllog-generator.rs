// urllog-generator.rs

use anyhow::{anyhow, bail};
use chrono::*;
use futures::TryStreamExt; // provides `try_next`
use log::*;
use std::{fs, thread, time};
use structopt::StructOpt;
use tera::Tera;

use urlharvest::*;

// A week in seconds
const URL_EXPIRE: i64 = 7 * 24 * 3600;
const VEC_SZ: usize = 1024;
const TPL_SUFFIX: &str = ".tera";
const SLEEP_IDLE: u64 = 10;
const SLEEP_BUSY: u64 = 2;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::from_args();
    opts.finish()?;
    start_pgm(&opts, "urllog_generator");
    info!("Starting up");
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{:#?}", &cfg);

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
            thread::sleep(time::Duration::new(SLEEP_IDLE, 0));
            continue;
        }
        latest_db = db_ts;

        let ts_limit = Utc::now().timestamp() - URL_EXPIRE;
        info!("Generating URL logs starting from {}", ts_limit.ts_long());
        let ctx = generate_ctx(&mut db, ts_limit).await?;
        for template in tera.get_template_names() {
            let cut_idx = template.rfind(TPL_SUFFIX).unwrap_or(template.len());
            let filename_out = format!("{}/{}", &cfg.html_dir, &template[0..cut_idx]);
            let filename_tmp = format!(
                "{filename_out}.{}.{}.tmp",
                std::process::id(),
                Utc::now().timestamp_nanos()
            );
            info!("Generating {filename_out} from {template}");
            let template_output = tera.render(template, &ctx)?;
            fs::write(&filename_tmp, &template_output)?;
            fs::rename(&filename_tmp, &filename_out)?;
        }
        thread::sleep(time::Duration::new(SLEEP_BUSY, 0));
    }
}

#[derive(Debug, sqlx::FromRow)]
struct DbRead {
    id: i64,
    seen_first: i64,
    seen_last: i64,
    seen_count: i64,
    channel: String,
    nick: String,
    url: String,
    title: String,
}

async fn generate_ctx(db: &mut DbCtx, ts_limit: i64) -> anyhow::Result<tera::Context> {
    let sql_url = format!(
        "select min(u.id) as id, min(seen) as seen_first, max(seen) as seen_last, count(seen) as seen_count, \
          channel, nick, url, {table_meta}.title \
        from {table_url} as u \
        inner join {table_meta} on {table_meta}.url_id = u.id \
        group by channel, url \
        having max(seen) > ? \
        order by max(seen) desc",
        table_url = TABLE_URL,
        table_meta = TABLE_META
    );
    let sql_uniq = format!(
        "select min(u.id) as id, min(seen) as seen_first, max(seen) as seen_last, count(seen) as seen_count, \
        group_concat(channel, ' ') as channel, group_concat(nick, ' ') as nick, \
        url, {table_meta}.title \
        from {table_url} as u \
        inner join {table_meta} on {table_meta}.url_id = u.id \
        group by url \
        having max(seen) > ? \
        order by max(seen) desc",
        table_url = TABLE_URL,
        table_meta = TABLE_META
    );

    let mut ctx = tera::Context::new();
    ctx.insert("last_change", &Utc::now().timestamp().ts_long());
    {
        let mut arr_id = Vec::with_capacity(VEC_SZ);
        let mut arr_first_seen = Vec::with_capacity(VEC_SZ);
        let mut arr_last_seen = Vec::with_capacity(VEC_SZ);
        let mut arr_num_seen = Vec::with_capacity(VEC_SZ);
        let mut arr_channel = Vec::with_capacity(VEC_SZ);
        let mut arr_nick = Vec::with_capacity(VEC_SZ);
        let mut arr_url = Vec::with_capacity(VEC_SZ);
        let mut arr_title = Vec::with_capacity(VEC_SZ);

        let mut i_row: usize = 0;
        {
            let mut st_url = sqlx::query_as::<_, DbRead>(&sql_url)
                .bind(ts_limit)
                .fetch(&mut db.dbc);

            while let Some(row) = st_url.try_next().await? {
                arr_id.push(row.id);
                arr_first_seen.push(row.seen_first.ts_y_short());
                arr_last_seen.push(row.seen_last.ts_short());
                arr_num_seen.push(row.seen_count);
                arr_channel.push(row.channel.esc_ltgt());
                arr_nick.push(row.nick.esc_ltgt());
                arr_url.push(row.url.esc_quot());
                arr_title.push(row.title.esc_ltgt());
                i_row += 1;
            }
        }
        info!("Got {i_row} rows.");
        ctx.insert("n_rows", &i_row);
        ctx.insert("id", &arr_id);
        ctx.insert("first_seen", &arr_first_seen);
        ctx.insert("last_seen", &arr_last_seen);
        ctx.insert("num_seen", &arr_num_seen);
        ctx.insert("channel", &arr_channel);
        ctx.insert("nick", &arr_nick);
        ctx.insert("url", &arr_url);
        ctx.insert("title", &arr_title);
    }
    {
        let mut uniq_id = Vec::with_capacity(VEC_SZ);
        let mut uniq_first_seen = Vec::with_capacity(VEC_SZ);
        let mut uniq_last_seen = Vec::with_capacity(VEC_SZ);
        let mut uniq_num_seen = Vec::with_capacity(VEC_SZ);
        let mut uniq_channel = Vec::with_capacity(VEC_SZ);
        let mut uniq_nick = Vec::with_capacity(VEC_SZ);
        let mut uniq_url = Vec::with_capacity(VEC_SZ);
        let mut uniq_title = Vec::with_capacity(VEC_SZ);

        let mut i_uniq_row: usize = 0;
        {
            let mut st_uniq = sqlx::query_as::<_, DbRead>(&sql_uniq)
                .bind(ts_limit)
                .fetch(&mut db.dbc);

            while let Some(row) = st_uniq.try_next().await? {
                uniq_id.push(row.id);
                uniq_first_seen.push(row.seen_first.ts_y_short());
                uniq_last_seen.push(row.seen_last.ts_short());
                uniq_num_seen.push(row.seen_count);
                uniq_channel.push(row.channel.esc_ltgt().sort_dedup_br());
                uniq_nick.push(row.nick.esc_ltgt().sort_dedup_br());
                uniq_url.push(row.url.esc_quot());
                uniq_title.push(row.title.esc_ltgt());
                i_uniq_row += 1;
            }
        }
        info!("Got {i_uniq_row} uniq rows.");
        ctx.insert("n_uniq_rows", &i_uniq_row);
        ctx.insert("uniq_id", &uniq_id);
        ctx.insert("uniq_first_seen", &uniq_first_seen);
        ctx.insert("uniq_last_seen", &uniq_last_seen);
        ctx.insert("uniq_num_seen", &uniq_num_seen);
        ctx.insert("uniq_channel", &uniq_channel);
        ctx.insert("uniq_nick", &uniq_nick);
        ctx.insert("uniq_url", &uniq_url);
        ctx.insert("uniq_title", &uniq_title);
    }
    Ok(ctx)
}
// EOF
