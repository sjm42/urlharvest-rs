// urllog-search.rs

use futures::TryStreamExt;
use handlebars::{to_json, Handlebars};
use log::*;
use regex::Regex;
use serde::Deserialize;
use sqlx::{Connection, SqliteConnection};
use std::fmt::Write as _;
use std::net::SocketAddr;
use structopt::StructOpt;
use warp::Filter; // provides `try_next`

use urlharvest::*;

const INDEX_NAME: &str = "index";
const REQ_PATH_SEARCH: &str = "search";
const RE_SEARCH: &str = r#"^[-_\.:/0-9a-zA-Z\?\* ]*$"#;
const DEFAULT_CAP: usize = 65536;

const TEXT_PLAIN: &str = "text/plain; charset=utf-8";
const TEXT_HTML: &str = "text/html; charset=utf-8";

#[derive(Debug, Deserialize)]
pub struct SearchParam {
    chan: String,
    nick: String,
    url: String,
    title: String,
}

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::from_args();
    opts.finish()?;
    start_pgm(&opts, "urllog_search");
    info!("Starting up");
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{:#?}", &cfg);

    {
        // Just init the database if necessary,
        // and then drop the connection.
        let _db = start_db(&cfg).await?;
    }

    let re_srch = Regex::new(RE_SEARCH)?;
    let index_path = cfg.search_template.clone();

    let mut hb_reg = Handlebars::new();
    hb_reg.register_template_file(INDEX_NAME, &index_path)?;
    let mut tpl_data = serde_json::value::Map::new();
    tpl_data.insert("cmd_search".into(), to_json("search"));
    let index_html = hb_reg.render(INDEX_NAME, &tpl_data)?;
    let server_addr: SocketAddr = cfg.search_listen;

    // GET / -> index html
    let req_index = warp::get()
        .and(warp::path::end())
        .map(move || my_response(TEXT_HTML, index_html.clone()));

    let req_search = warp::get()
        .and(warp::path(REQ_PATH_SEARCH))
        .and(warp::path::end())
        .and(warp::query::<SearchParam>())
        .map(move |s: SearchParam| {
            if !re_srch.is_match(&s.chan)
                || !re_srch.is_match(&s.nick)
                || !re_srch.is_match(&s.url)
                || !re_srch.is_match(&s.title)
            {
                return my_response(TEXT_PLAIN, "*** Illegal characters in query ***\n");
            }
            match futures::executor::block_on(search(&cfg.db_file, s)) {
                Ok(result) => my_response(TEXT_HTML, result),
                Err(e) => my_response(TEXT_PLAIN, format!("Query error: {e:?}")),
            }
        });

    let req_routes = req_search.or(req_index);
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

async fn search<S1>(db: S1, srch: SearchParam) -> anyhow::Result<String>
where
    S1: AsRef<str>,
{
    info!("search({srch:?})");
    let chan = srch.chan.sql_search();
    let nick = srch.nick.sql_search();
    let url = srch.url.sql_search();
    let title = srch.title.sql_search();
    info!("Search {chan} {nick} {url} {title}");

    let mut html = String::with_capacity(DEFAULT_CAP);
    html.push_str(
        r#"<table>
  <tr>
    <th>ID</th>
    <th>First seen</th>
    <th>Last seen</th>
    <th>#</th>
    <th>Channel</th>
    <th>Nick</th>
    <th>Title + URL</th>
  </tr>"#,
    );

    let mut dbc = SqliteConnection::connect(&format!("sqlite:{}", db.as_ref())).await?;
    let mut st_s = sqlx::query_as::<_, DbRead>(SQL_SEARCH)
        .bind(&chan)
        .bind(&nick)
        .bind(&url)
        .bind(&title)
        .fetch(&mut dbc);

    while let Some(row) = st_s.try_next().await? {
        let id = row.id;
        let first_seen = row.seen_first.ts_y_short();
        let last_seen = row.seen_last.ts_short();
        let num_seen = row.seen_count;
        let chans = row.channels.esc_ltgt().sort_dedup_br();
        let nicks = row.nicks.esc_ltgt().sort_dedup_br();
        let url = row.url.esc_quot();
        let title = row.title.esc_ltgt();

        write!(
            html,
            "<td>{id}</td><td>{first_seen}</td><td>{last_seen}</td><td>{num_seen}</td>\n\
                <td>{chans}</td><td>{nicks}</td>\n\
                <td>{title}<br>\n<a href=\"{url}\">{url}</a></td>\n</tr>\n",
        )
        .unwrap();
    }

    html.push_str("</table>\n");
    Ok(html)
}
// EOF
