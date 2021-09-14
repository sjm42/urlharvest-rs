// urllog-generator.rs

use chrono::*;
use log::*;
use std::{error::Error, fs, thread, time};
use structopt::StructOpt;
use tera::Tera;

use urlharvest::*;

// A week in seconds
const URL_EXPIRE: i64 = 7 * 24 * 3600;
const VEC_SZ: usize = 1024;
const TPL_SUFFIX: &str = ".tera";
const TS_FMT: &str = "%Y-%m-%d %H:%M:%S";
const SHORT_TS_FMT: &str = "%b %d %H:%M";
const SHORT_TS_YEAR_FMT: &str = "%Y %b %d %H:%M";

/*
Creating global Tera template state could be done like this:

static TERA_DIR: SyncLazy<RwLock<String>> = SyncLazy::new(|| RwLock::new(String::new()));
static TERA: SyncLazy<RwLock<Tera>> = SyncLazy::new(|| {
    RwLock::new(match Tera::new(&format!("{}/*.tera", TERA_DIR.read())) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Tera template parsing error: {}", e);
            ::std::process::exit(1);
        }
    })
});
*/
*/

fn main() -> Result<(), Box<dyn Error>> {
    let mut opts = OptsGenerator::from_args();
    opts.finish()?;
    start_pgm(&opts.c, "urllog generator");
    let db = start_db(&opts.c)?;

    let tera_dir = &opts.template_dir;
    info!("Template directory: {}", tera_dir);
    let tera = match Tera::new(&format!("{}/*.tera", tera_dir)) {
        Ok(t) => t,
        Err(e) => {
            return Err(format!("Tera template parsing error: {}", e).into());
        }
    };
    if tera.get_template_names().count() < 1 {
        error!("No templates found. Exit.");
        return Err("Templates not found".into());
    }
    info!(
        "Found templates: [{}]",
        tera.get_template_names().collect::<Vec<_>>().join(", ")
    );

    let mut latest_ts: i64 = 0;
    loop {
        thread::sleep(time::Duration::new(10, 0));
        let db_ts = db_last_change(&db)?;
        if db_ts <= latest_ts {
            trace!("Nothing new in DB.");
            continue;
        }
        latest_ts = db_ts;

        let ts_limit = db_ts - URL_EXPIRE;
        let ts_limit_str = Local
            .from_utc_datetime(&NaiveDateTime::from_timestamp(ts_limit, 0))
            .format(TS_FMT);

        info!("Generating URL logs starting from {}", &ts_limit_str);
        let ctx = populate_ctx(&db, ts_limit)?;
        for template in tera.get_template_names() {
            let cut_idx = template.rfind(TPL_SUFFIX).unwrap_or(template.len());
            let filename_out = format!("{}/{}", &opts.html_dir, &template[0..cut_idx]);
            let filename_tmp = format!("{}.{}.tmp", filename_out, std::process::id());
            info!("Generating {} from {}", filename_out, template);
            let template_output = tera.render(template, &ctx)?;
            fs::write(&filename_tmp, &template_output)?;
            fs::rename(&filename_tmp, &filename_out)?;
        }
    }
}

fn populate_ctx(db: &DbCtx, ts_limit: i64) -> Result<tera::Context, Box<dyn Error>> {
    let sql_url = format!(
        "select min(u.id), min(seen), max(seen), count(seen), \
          channel, url, {table_meta}.title \
        from {table_url} as u \
        inner join {table_meta} on {table_meta}.url_id = u.id \
        group by channel, url \
        having max(seen) > ? \
        order by max(seen) desc",
        table_url = db.table_url,
        table_meta = db.table_meta
    );
    let sql_uniq = format!(
        "select min(u.id), min(seen), max(seen), count(seen), \
        group_concat(channel, ' '), group_concat(nick, ' '), \
        url, {table_meta}.title \
        from {table_url} as u \
        inner join {table_meta} on {table_meta}.url_id = u.id \
        group by url \
        having max(seen) > ? \
        order by max(seen) desc",
        table_url = db.table_url,
        table_meta = db.table_meta
    );

    let mut ctx = tera::Context::new();
    ctx.insert("last_change", &Utc::now().format(TS_FMT).to_string());
    {
        let mut arr_id = Vec::with_capacity(VEC_SZ);
        let mut arr_first_seen = Vec::with_capacity(VEC_SZ);
        let mut arr_last_seen = Vec::with_capacity(VEC_SZ);
        let mut arr_num_seen = Vec::with_capacity(VEC_SZ);
        let mut arr_channel = Vec::with_capacity(VEC_SZ);
        let mut arr_url = Vec::with_capacity(VEC_SZ);
        let mut arr_title = Vec::with_capacity(VEC_SZ);

        let mut i_row: usize = 0;
        {
            let mut st_url = db.dbc.prepare(&sql_url)?;
            let mut rows = st_url.query([ts_limit])?;
            while let Some(row) = rows.next()? {
                arr_id.push(row.get::<usize, i64>(0)?);

                let first_seen_i: i64 = row.get(1)?;
                let first_seen_str = Local
                    .from_utc_datetime(&NaiveDateTime::from_timestamp(first_seen_i, 0))
                    .format(SHORT_TS_YEAR_FMT)
                    .to_string();
                arr_first_seen.push(first_seen_str);

                let last_seen_i: i64 = row.get(2)?;
                let last_seen_str = Local
                    .from_utc_datetime(&NaiveDateTime::from_timestamp(last_seen_i, 0))
                    .format(SHORT_TS_FMT)
                    .to_string();
                arr_last_seen.push(last_seen_str);

                arr_num_seen.push(row.get::<usize, i64>(3)?);

                let ch = row
                    .get::<usize, String>(4)?
                    .replace("<", "&lt;")
                    .replace(">", "&gt;");
                arr_channel.push(ch);

                let url = row.get::<usize, String>(5)?.replace("\"", "&quot;");
                arr_url.push(url);

                let title = row
                    .get::<usize, String>(6)?
                    .replace("<", "&lt;")
                    .replace(">", "&gt;");
                arr_title.push(title);

                i_row += 1;
            }
        }
        debug!("Read {} rows.", i_row);
        ctx.insert("n_rows", &i_row);
        ctx.insert("id", &arr_id);
        ctx.insert("first_seen", &arr_first_seen);
        ctx.insert("last_seen", &arr_last_seen);
        ctx.insert("num_seen", &arr_num_seen);
        ctx.insert("channel", &arr_channel);
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
            let mut st_uniq = db.dbc.prepare(&sql_uniq)?;
            let mut uniq_rows = st_uniq.query([ts_limit])?;
            while let Some(row) = uniq_rows.next()? {
                uniq_id.push(row.get::<usize, i64>(0)?);

                let first_seen_i: i64 = row.get(1)?;
                let first_seen_str = Local
                    .from_utc_datetime(&NaiveDateTime::from_timestamp(first_seen_i, 0))
                    .format(SHORT_TS_YEAR_FMT)
                    .to_string();
                uniq_first_seen.push(first_seen_str);

                let last_seen_i: i64 = row.get(2)?;
                let last_seen_str = Local
                    .from_utc_datetime(&NaiveDateTime::from_timestamp(last_seen_i, 0))
                    .format(SHORT_TS_FMT)
                    .to_string();
                uniq_last_seen.push(last_seen_str);

                uniq_num_seen.push(row.get::<usize, u64>(3)?);

                // channels and nicks returned from db in arbitrary order separated by whitespace so we sort them
                let db_ch = row
                    .get::<usize, String>(4)?
                    .replace("<", "&lt;")
                    .replace(">", "&gt;");
                let mut ch = db_ch.split_whitespace().collect::<Vec<&str>>();
                #[allow(clippy::stable_sort_primitive)]
                ch.sort();
                ch.dedup();
                uniq_channel.push(ch.join("<br>"));

                let db_ni = row
                    .get::<usize, String>(5)?
                    .replace("<", "&lt;")
                    .replace(">", "&gt;");
                let mut ni = db_ni.split_whitespace().collect::<Vec<&str>>();
                #[allow(clippy::stable_sort_primitive)]
                ni.sort();
                ni.dedup();
                uniq_nick.push(ni.join("<br>"));

                let url = row.get::<usize, String>(6)?.replace("\"", "&quot;");
                uniq_url.push(url);

                let title = row
                    .get::<usize, String>(7)?
                    .replace("<", "&lt;")
                    .replace(">", "&gt;");
                uniq_title.push(title);
                i_uniq_row += 1;
            }
        }
        debug!("Read {} uniq rows.", i_uniq_row);
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
