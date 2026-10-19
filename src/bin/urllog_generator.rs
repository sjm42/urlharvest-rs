// bin/urllog_generator.rs

use enum_iterator::Sequence;
use sqlx::{FromRow, postgres::PgListener};
use tera::Tera;

use urlharvest::*;

#[cfg(test)]
#[path = "../../tests/support/mod.rs"]
mod db_test_support;

const URL_EXPIRE: i64 = 7 * 24 * 3600;
// A week in seconds
const VEC_SZ: usize = 4096;
const TPL_SUFFIX: &str = ".tera";
const SLEEP_BUSY: u64 = 2;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::parse();
    opts.finalize()?;
    opts.start_pgm(env!("CARGO_BIN_NAME"));
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{cfg:#?}");

    let mut dbc = start_db(&cfg).await?;

    let tera_dir = &cfg.template_dir;
    info!("Template directory: {tera_dir}");
    let mut tera = Tera::new();
    if let Err(e) = tera.load_from_glob(&format!("{tera_dir}/*.tera")) {
        return Err(anyhow!("Tera template parsing error: {e:?}"));
    }
    if tera.get_template_names().count() < 1 {
        error!("No templates found. Exit.");
        bail!("Templates not found");
    }
    info!(
        "Found templates: [{}]",
        tera.get_template_names().collect::<Vec<_>>().join(", ")
    );

    let mut listener = PgListener::connect_with(&dbc.dbc).await?;
    listener.listen(PAGE_CHANGE_CHANNEL).await?;

    loop {
        if let Err(e) = generate_pages(&mut dbc, &tera, &cfg).await {
            error!("Page generate error: {e}");
            sleep(Duration::new(SLEEP_BUSY, 0)).await;
            continue;
        }

        info!("Waiting for page data updates");
        match listener.try_recv().await? {
            Some(notification) => trace!(
                "Page data update notification from backend {}",
                notification.process_id()
            ),
            None => warn!("Database listener reconnected; reconciling current state"),
        }

        // Read notifications throughout a fixed batching window. next_buffered()
        // alone does not consume notifications still waiting on the socket.
        if let Ok(result) = timeout(Duration::from_secs(SLEEP_BUSY), async {
            while listener.try_recv().await?.is_some() {}
            warn!("Database listener reconnected; reconciling current state");
            Ok::<(), sqlx::Error>(())
        })
        .await
        {
            result?;
        }
    }
}

async fn generate_pages(dbc: &mut DbCtx, tera: &Tera, cfg: &ConfigCommon) -> anyhow::Result<()> {
    let mut now = Utc::now();
    let ts_limit = now.timestamp() - URL_EXPIRE;
    info!("Generating URL logs starting from {}", ts_limit.ts_long());
    let (db_data, db_data_uniq) = read_db(dbc, ts_limit).await?;
    info!(
        "Database read took {} ms.",
        Utc::now().signed_duration_since(now).num_milliseconds()
    );

    now = Utc::now();
    let html_dir = &cfg.html_dir;
    for template in tera.get_template_names() {
        let basename = template.strip_suffix(TPL_SUFFIX).unwrap_or(template);
        let filename_out = format!("{html_dir}/{basename}");
        let filename_tmp = format!(
            "{filename_out}.{}.{}.tmp",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let tz = match cfg.template_tz.as_ref() {
            Some(map) => get_wild(map, basename).unwrap_or(&Tz::UTC),
            None => &Tz::UTC,
        };

        info!("Generating {filename_out} from {template}");
        let ctx = generate_ctx(&db_data, &db_data_uniq, tz).await?;
        let template_output = tera.render(template, &ctx)?;
        fs::write(&filename_tmp, template_output)?;
        fs::rename(&filename_tmp, &filename_out)?;
    }
    info!(
        "Template rendering took {} ms.",
        Utc::now().signed_duration_since(now).num_milliseconds()
    );
    Ok(())
}

#[derive(Debug, FromRow)]
struct DbRead {
    id: i32,
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

const SQL_URL: &str = "select min(url.id) as id, \
    min(seen) as seen_first, max(seen) as seen_last, count(seen) as seen_cnt, \
    channel, string_agg(nick, ' ') as nick, \
    url, (array_agg(url_meta.title order by url_meta.id desc))[1] as title from url \
    inner join url_meta on url_meta.url_id = url.id \
    group by channel, url \
    having max(seen) > $1 \
    order by max(seen) desc";

const SQL_UNIQ: &str = "select min(url.id) as id, \
    min(seen) as seen_first, max(seen) as seen_last, count(seen) as seen_cnt, \
    string_agg(channel, ' ') as channel, string_agg(nick, ' ') as nick, \
    url, (array_agg(url_meta.title order by url_meta.id desc))[1] as title from url \
    inner join url_meta on url_meta.url_id = url.id \
    group by url \
    having max(seen) > $1 \
    order by max(seen) desc";

async fn read_db(dbc: &mut DbCtx, ts_limit: i64) -> anyhow::Result<(Vec<DbRead>, Vec<DbRead>)> {
    let mut db_data = Vec::with_capacity(VEC_SZ);
    let mut db_data_uniq = Vec::with_capacity(VEC_SZ);

    let mut n_rows: usize = 0;
    let mut st_url = sqlx::query_as::<_, DbRead>(SQL_URL).bind(ts_limit).fetch(&dbc.dbc);

    while let Some(row) = st_url.try_next().await? {
        n_rows += 1;
        db_data.push(row);
    }
    drop(st_url);
    info!("Got {n_rows} rows.");

    n_rows = 0;

    let mut st_uniq = sqlx::query_as::<_, DbRead>(SQL_UNIQ).bind(ts_limit).fetch(&dbc.dbc);

    while let Some(row) = st_uniq.try_next().await? {
        n_rows += 1;
        db_data_uniq.push(row);
    }
    drop(st_uniq);

    info!("Got {n_rows} uniq rows.");
    Ok((db_data, db_data_uniq))
}

async fn generate_ctx(db_data: &Vec<DbRead>, db_data_uniq: &Vec<DbRead>, tz: &Tz) -> anyhow::Result<tera::Context> {
    let mut data: HashMap<CtxData, Vec<String>> = HashMap::with_capacity(CTX_NUM);
    // Magic to iterate through all enum variants
    for k in enum_iterator::all::<CtxData>() {
        let v: Vec<String> = Vec::with_capacity(VEC_SZ);
        data.insert(k, v);
    }

    let mut ctx = tera::Context::new();
    ctx.insert("last_change", &Utc::now().timestamp().ts_long_tz(tz));

    let mut n_rows: usize = 0;
    for row in db_data {
        data.get_mut(&CtxData::id)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.id.to_string());
        data.get_mut(&CtxData::seen_first)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.seen_first.ts_short_y_tz(tz));
        data.get_mut(&CtxData::seen_last)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.seen_last.ts_short_tz(tz));
        data.get_mut(&CtxData::seen_cnt)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.seen_cnt.to_string());
        data.get_mut(&CtxData::channel)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.channel.clone().esc_et_lt_gt());
        data.get_mut(&CtxData::nick)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.nick.clone().esc_et_lt_gt());
        data.get_mut(&CtxData::url)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.url.esc_et_lt_gt().esc_quot());
        data.get_mut(&CtxData::title)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.title.clone().esc_et_lt_gt());
        n_rows += 1;
    }
    info!("Got {n_rows} rows.");
    ctx.insert("n_rows", &n_rows);

    n_rows = 0;
    for row in db_data_uniq {
        data.get_mut(&CtxData::uniq_id)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.id.to_string());
        data.get_mut(&CtxData::uniq_seen_first)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.seen_first.ts_short_y_tz(tz));
        data.get_mut(&CtxData::uniq_seen_last)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.seen_last.ts_short_tz(tz));
        data.get_mut(&CtxData::uniq_seen_cnt)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.seen_cnt.to_string());
        data.get_mut(&CtxData::uniq_channel)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.channel.clone().esc_et_lt_gt().sort_dedup_br());
        data.get_mut(&CtxData::uniq_nick)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.nick.clone().esc_et_lt_gt().sort_dedup_br());
        data.get_mut(&CtxData::uniq_url)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.url.esc_et_lt_gt().esc_quot());
        data.get_mut(&CtxData::uniq_title)
            .ok_or_else(|| anyhow!("no data"))?
            .push(row.title.clone().esc_et_lt_gt());
        n_rows += 1;
    }
    info!("Got {n_rows} uniq rows.");
    ctx.insert("uniq_n_rows", &n_rows);

    for k in enum_iterator::all::<CtxData>() {
        let k_name = k.to_string();
        ctx.insert(
            k_name.clone(),
            data.get(&k).ok_or_else(|| anyhow!("No data for {k_name}"))?,
        );
    }

    Ok(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn renders_urls_without_interpreting_entities_or_markup() {
        let url = r#"https://example.com/?a=&copy;&b="><img/src=x>"#;
        let escaped = "https://example.com/?a=&amp;copy;&amp;b=&quot;&gt;&lt;img/src=x&gt;";
        let rows = vec![DbRead {
            id: 1,
            seen_first: 1,
            seen_last: 2,
            seen_cnt: 2,
            channel: "#42".into(),
            nick: "test".into(),
            url: url.into(),
            title: "Example".into(),
        }];
        let ctx = generate_ctx(&rows, &rows, &Tz::UTC).await.unwrap();
        let mut tera = Tera::new();
        tera.load_from_glob(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/*.tera"))
            .unwrap();
        for template in ["42.html.tera", "url.html.tera", "url2.html.tera"] {
            let html = tera.render(template, &ctx).unwrap();
            assert!(
                html.contains(&format!("<a href=\"{escaped}\">{escaped}</a>")),
                "{template}"
            );
            assert!(!html.contains("<img"), "{template}");
        }
        assert_eq!(rows[0].url, url);
    }

    #[tokio::test]
    #[ignore = "requires URLHARVEST_TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
    async fn generator_queries_choose_the_latest_metadata_after_refresh() {
        let mut conn = db_test_support::metadata_fixture().await;
        for (refreshed, expected_title) in [(false, "Current title"), (true, "Refreshed title")] {
            if refreshed {
                sqlx::query("delete from url_meta where url_id = 1")
                    .execute(&mut conn)
                    .await
                    .unwrap();
                sqlx::query("insert into url_meta (url_id, title) values (1, 'Refreshed title')")
                    .execute(&mut conn)
                    .await
                    .unwrap();
            }
            for (sql, channel, expected_count) in [(SQL_URL, Some("#test"), 2), (SQL_UNIQ, None, 3)] {
                let rows = sqlx::query_as::<_, DbRead>(sql)
                    .bind(0_i64)
                    .fetch_all(&mut conn)
                    .await
                    .unwrap();
                let row = rows
                    .iter()
                    .find(|row| row.url == "https://example.com/" && channel.is_none_or(|c| row.channel == c))
                    .unwrap();
                assert_eq!(row.id, 1);
                assert_eq!(row.title, expected_title);
                assert_eq!(row.seen_cnt, expected_count);
                assert_eq!(row.seen_last, 30);
                if channel.is_some() {
                    let other = rows.iter().find(|row| row.channel == "#other").unwrap();
                    assert_eq!(other.title, "Other channel title");
                }
            }
        }
    }

    #[tokio::test]
    async fn renders_checked_in_templates() {
        let template_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/templates");
        let mut tera = Tera::new();
        tera.load_from_glob(&format!("{template_dir}/*.tera"))
            .expect("checked-in Tera templates should parse");
        let rows = vec![DbRead {
            id: 1,
            seen_first: 1,
            seen_last: 2,
            seen_cnt: 2,
            channel: "#42".to_owned(),
            nick: "test".to_owned(),
            url: "https://example.com".to_owned(),
            title: "Example".to_owned(),
        }];
        let unique_rows = vec![DbRead {
            id: 1,
            seen_first: 1,
            seen_last: 2,
            seen_cnt: 2,
            channel: "#42".to_owned(),
            nick: "test".to_owned(),
            url: "https://example.com".to_owned(),
            title: "Example".to_owned(),
        }];
        let context = generate_ctx(&rows, &unique_rows, &Tz::UTC)
            .await
            .expect("representative template context should build");

        for template in tera.get_template_names() {
            tera.render(template, &context)
                .unwrap_or_else(|e| panic!("checked-in Tera template {template} should render: {e}"));
        }
    }
}
// EOF
