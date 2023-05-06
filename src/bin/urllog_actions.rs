// bin/urllog_actions.rs

use anyhow::anyhow;
use futures::TryStreamExt;
use handlebars::{to_json, Handlebars};
use itertools::Itertools;
use log::*;
use regex::Regex;
use serde::Deserialize;
use sqlx::{Connection, SqliteConnection};
use std::{fmt::Display, net::SocketAddr, path::Path, sync::Arc};
use structopt::StructOpt;
use warp::Filter; // provides `try_next`

use urlharvest::*;

const TEXT_PLAIN: &str = "text/plain; charset=utf-8";
const TEXT_HTML: &str = "text/html; charset=utf-8";

const TPL_INDEX: &str = "index";
const TPL_RESULT_HEADER: &str = "result_header";
const TPL_RESULT_ROW: &str = "result_row";
const TPL_RESULT_FOOTER: &str = "result_footer";

const DEFAULT_REPLY_CAP: usize = 65536;

const REQ_PATH_SEARCH: &str = "search";
const RE_SEARCH: &str = r#"^[-_\.:;/0-9a-zA-Z\?\*\(\)\[\]\{\}\|\\ ]*$"#;

const REQ_PATH_REMOVE_URL: &str = "remove_url";
const REQ_PATH_REMOVE_META: &str = "remove_meta";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::from_args();
    opts.finish()?;
    opts.start_pgm(env!("CARGO_BIN_NAME"));
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{:#?}", &cfg);

    // Just check and init the database if necessary,
    // and then drop the connection immediately.
    let db = start_db(&cfg).await?;
    drop(db);

    // Now it's time for some iterator porn.
    let (
        tpl_path_search_index,
        tpl_path_search_result_header,
        tpl_path_search_result_row,
        tpl_path_search_result_footer,
    ) = [
        &cfg.tpl_search_index,
        &cfg.tpl_search_result_header,
        &cfg.tpl_search_result_row,
        &cfg.tpl_search_result_footer,
    ]
    .iter()
    .map(|t| {
        // template names are relative to template_dir
        // hence we construct full paths here
        Path::new(&cfg.template_dir).join(*t)
    })
    .collect_tuple()
    .ok_or_else(|| anyhow!("Template iteration failed"))?;

    // Create Handlebars registry
    let mut hb_reg = Handlebars::new();

    // We handle html escaping ourselves
    hb_reg.register_escape_fn(handlebars::no_escape);

    // We render index html statically and save it
    hb_reg.register_template_file(TPL_INDEX, &tpl_path_search_index)?;
    let mut tpl_data = serde_json::value::Map::new();
    tpl_data.insert("cmd_search".into(), to_json("search"));
    let index_html = hb_reg.render(TPL_INDEX, &tpl_data)?;

    // Register other templates
    hb_reg.register_template_file(TPL_RESULT_HEADER, &tpl_path_search_result_header)?;
    hb_reg.register_template_file(TPL_RESULT_ROW, &tpl_path_search_result_row)?;
    hb_reg.register_template_file(TPL_RESULT_FOOTER, &tpl_path_search_result_footer)?;

    // precompile this regex
    let re_srch = Regex::new(RE_SEARCH)?;

    let server_addr: SocketAddr = cfg.search_listen;

    // GET / -> index html
    let req_index = warp::get()
        .and(warp::path::end())
        .map(move || my_response(TEXT_HTML, index_html.clone()));

    let db_search = Arc::new(cfg.db_file.clone());
    let db_rm_url = db_search.clone();
    let db_rm_meta = db_search.clone();
    let a_hb = Arc::new(hb_reg);
    let a_srch = Arc::new(re_srch);

    let req_search = warp::get()
        .and(warp::path(REQ_PATH_SEARCH))
        .and(warp::path::end())
        .and(warp::query::<SearchParam>())
        .then(move |s: SearchParam| {
            let db = db_search.clone();
            let reg = a_hb.clone();
            let srch = a_srch.clone();
            async move {
                if !validate_search_param(&s, srch) {
                    return my_response(TEXT_PLAIN, "*** Illegal characters in query*** ");
                }

                match search(db, reg, s).await {
                    Ok(result) => my_response(TEXT_HTML, result),
                    Err(e) => my_response(TEXT_PLAIN, format!("Query error: {e:?}")),
                }
            }
        });

    let req_remove_url = warp::get()
        .and(warp::path(REQ_PATH_REMOVE_URL))
        .and(warp::path::end())
        .and(warp::query::<RemoveParam>())
        .then(move |s: RemoveParam| {
            let db = db_rm_url.clone();
            async move {
                match remove_url(db, s).await {
                    Ok(result) => my_response(TEXT_HTML, result),
                    Err(e) => my_response(TEXT_PLAIN, format!("Query error: {e:?}")),
                }
            }
        });

    let req_remove_meta = warp::get()
        .and(warp::path(REQ_PATH_REMOVE_META))
        .and(warp::path::end())
        .and(warp::query::<RemoveParam>())
        .then(move |s: RemoveParam| {
            let db = db_rm_meta.clone();
            async move {
                match remove_meta(db, s).await {
                    Ok(result) => my_response(TEXT_HTML, result),
                    Err(e) => my_response(TEXT_PLAIN, format!("Query error: {e:?}")),
                }
            }
        });

    let req_routes = req_search
        .or(req_remove_url)
        .or(req_remove_meta)
        .or(req_index);

    warp::serve(req_routes).run(server_addr).await;
    Ok(())
}

fn my_response<S1, S2>(
    resp_type: S1,
    resp_body: S2,
) -> Result<warp::http::Response<String>, warp::http::Error>
where
    S1: AsRef<str>,
    S2: AsRef<str>,
{
    warp::http::Response::builder()
        .header("cache-control", "no-store")
        .header("content-type", resp_type.as_ref())
        .body(resp_body.as_ref().into())
}

#[derive(Debug, Deserialize)]
pub struct SearchParam {
    chan: String,
    nick: String,
    url: String,
    title: String,
}

const SQL_SEARCH: &str = "select min(u.id) as id, min(seen) as seen_first, max(seen) as seen_last, count(seen) as seen_count, \
    group_concat(channel, ' ') as channels, group_concat(nick, ' ') as nicks, \
    url, url_meta.title from url as u \
    inner join url_meta on url_meta.url_id = u.id \
    where lower(channel) like ? \
    and lower(nick) like ? \
    and lower(url) like ? \
    and lower(url_meta.title) like ? \
    group by url \
    order by max(seen) desc \
    limit 255";

#[derive(Debug, sqlx::FromRow)]
struct DbRead {
    id: i64,
    seen_first: i64,
    seen_last: i64,
    seen_count: i64,
    channels: String,
    nicks: String,
    url: String,
    title: String,
}

async fn search<S1>(
    db: Arc<S1>,
    hb_reg: Arc<Handlebars<'_>>,
    params: SearchParam,
) -> anyhow::Result<String>
where
    S1: AsRef<str> + Display,
{
    info!("search({params:?})");
    let chan = params.chan.sql_search();
    let nick = params.nick.sql_search();
    let url = params.url.sql_search();
    let title = params.title.sql_search();
    info!("Search {chan} {nick} {url} {title}");

    let tpl_data_empty = serde_json::value::Map::new();
    let html_header = hb_reg.render(TPL_RESULT_HEADER, &tpl_data_empty)?;
    let html_footer = hb_reg.render(TPL_RESULT_FOOTER, &tpl_data_empty)?;

    let mut html = String::with_capacity(DEFAULT_REPLY_CAP);
    html.push_str(&html_header);

    let mut dbc = SqliteConnection::connect(&format!("sqlite:{}", db.as_ref())).await?;
    let mut st_s = sqlx::query_as::<_, DbRead>(SQL_SEARCH)
        .bind(&chan)
        .bind(&nick)
        .bind(&url)
        .bind(&title)
        .fetch(&mut dbc);

    while let Some(row) = st_s.try_next().await? {
        let mut tpl_data_row = serde_json::value::Map::new();
        [
            ("id", row.id.to_string()),
            ("first_seen", row.seen_first.ts_short_y()),
            ("last_seen", row.seen_last.ts_short()),
            ("num_seen", row.seen_count.to_string()),
            ("chans", row.channels.esc_et_lt_gt().sort_dedup_br()),
            ("nicks", row.nicks.esc_et_lt_gt().sort_dedup_br()),
            ("url", row.url.esc_quot()),
            ("title", row.title.esc_et_lt_gt()),
        ]
        .iter()
        .for_each(|(k, v)| {
            tpl_data_row.insert(k.to_string(), to_json(v));
        });
        // debug!("Result row:\n{tpl_data_row:#?}");
        html.push_str(&hb_reg.render(TPL_RESULT_ROW, &tpl_data_row)?);
    }

    html.push_str(&html_footer);
    Ok(html)
}

fn validate_search_param(par: &SearchParam, re: Arc<Regex>) -> bool {
    re.is_match(&par.chan)
        && re.is_match(&par.nick)
        && re.is_match(&par.url)
        && re.is_match(&par.title)
}

#[derive(Debug, Deserialize)]
pub struct RemoveParam {
    id: String,
}

const SQL_REMOVE_URL: &str = "delete from url where url in (select url from url where id = ?)";

async fn remove_url<S1>(db: Arc<S1>, params: RemoveParam) -> anyhow::Result<String>
where
    S1: AsRef<str> + Display,
{
    info!("remove_url({params:?})");
    let id = params.id.parse::<i64>().unwrap_or_default();
    info!("Remove url id {id}");

    let mut dbc = SqliteConnection::connect(&format!("sqlite:{}", db.as_ref())).await?;
    let db_res = sqlx::query(SQL_REMOVE_URL)
        .bind(id)
        .execute(&mut dbc)
        .await?;
    let n_rows = db_res.rows_affected();

    let msg = format!("Removed #{n_rows}");
    info!("{msg}");
    db_mark_change(&mut dbc).await?;
    Ok(msg)
}

const SQL_REMOVE_META: &str = "delete from url_meta where url_id = ?";

async fn remove_meta<S1>(db: Arc<S1>, params: RemoveParam) -> anyhow::Result<String>
where
    S1: AsRef<str> + Display,
{
    info!("remove_meta({params:?})");
    let id = params.id.parse::<i64>().unwrap_or_default();
    info!("Remove meta id {id}");

    let mut dbc = SqliteConnection::connect(&format!("sqlite:{}", db.as_ref())).await?;
    let _db_res = sqlx::query(SQL_REMOVE_META)
        .bind(id)
        .execute(&mut dbc)
        .await?;

    let msg = "Refreshing".into();
    info!("{msg}");
    db_mark_change(&mut dbc).await?;
    Ok(msg)
}

// EOF
