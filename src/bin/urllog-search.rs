// urllog-search.rs

use handlebars::{to_json, Handlebars};
use log::*;
use regex::Regex;
use rusqlite::Connection;
use serde_derive::Deserialize;
use std::net::SocketAddr;
use structopt::StructOpt;
use warp::Filter;

use urlharvest::*;

const INDEX_PATH: &str = "$HOME/urllog/templates2/search.html.hbs";
const INDEX_NAME: &str = "index";
const REQ_PATH_SEARCH: &str = "search";
const RE_SEARCH: &str = r#"^[-_\.:/0-9a-zA-Z\?\* ]*$"#;
const RESULT_MAXSZ: usize = 100;
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsSearch::from_args();
    opts.finish()?;
    start_pgm(&opts.c, "urllog search server");
    {
        // Just init the database if necessary,
        // and then drop the connection.
        let _db = start_db(&opts.c)?;
    }
    let sql_search = format!(
        "select min(u.id), min(seen), max(seen), count(seen), \
        group_concat(channel, ' '), group_concat(nick, ' '),
        url, {table_meta}.title from {table_url} as u \
        inner join {table_meta} on {table_meta}.url_id = u.id \
        where lower(channel) like ? \
        and lower(nick) like ? \
        and lower(url) like ? \
        and lower({table_meta}.title) like ? \
        group by url \
        order by max(seen) desc \
        limit {sz}",
        table_url = opts.c.table_url,
        table_meta = opts.c.table_meta,
        sz = RESULT_MAXSZ,
    );

    let re_srch = Regex::new(RE_SEARCH)?;
    let index_path = shellexpand::full(INDEX_PATH)?.into_owned();

    let mut hb_reg = Handlebars::new();
    hb_reg.register_template_file(INDEX_NAME, &index_path)?;
    let mut tpl_data = serde_json::value::Map::new();
    tpl_data.insert("cmd_search".into(), to_json("search"));
    let index_html = hb_reg.render(INDEX_NAME, &tpl_data)?;
    let server_addr: SocketAddr = opts.listen;

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
            match search(&opts.c.db_file, &sql_search, s) {
                Ok(result) => my_response(TEXT_HTML, result),
                Err(e) => my_response(TEXT_PLAIN, format!("Query error: {:?}", e)),
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

fn search<S1, S2>(db: S1, sql: S2, srch: SearchParam) -> anyhow::Result<String>
where
    S1: AsRef<str>,
    S2: AsRef<str>,
{
    info!("search({:?})", srch);
    let chan = srch.chan.sql_search();
    let nick = srch.nick.sql_search();
    let url = srch.url.sql_search();
    let title = srch.title.sql_search();
    info!("Search {} {} {} {}", chan, nick, url, title);

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

    let sqc = Connection::open(db.as_ref())?;
    {
        let mut st_s = sqc.prepare(sql.as_ref())?;
        let mut rows = st_s.query(&[&chan, &nick, &url, &title])?;
        while let Some(row) = rows.next()? {
            let id = row.get::<usize, i64>(0)?;
            let first_seen = ts_y_short(row.get::<usize, i64>(1)?);
            let last_seen = ts_short(row.get::<usize, i64>(2)?);
            let num_seen = row.get::<usize, u64>(3)?;
            let chans = row.get::<usize, String>(4)?.esc_ltgt().sort_dedup_br();
            let nicks = row.get::<usize, String>(5)?.esc_ltgt().sort_dedup_br();
            let url = row.get::<usize, String>(6)?.esc_quot();
            let title = row.get::<usize, String>(7)?.esc_ltgt();

            html.push_str(&format!(
                "<td>{id}</td><td>{first}</td><td>{last}</td><td>{num}</td>\n\
                <td>{chans}</td><td>{nicks}</td>\n\
                <td>{title}<br>\n<a href=\"{url}\">{url}</a></td>\n</tr>\n",
                id = id,
                first = first_seen,
                last = last_seen,
                num = num_seen,
                chans = chans,
                nicks = nicks,
                title = title,
                url = url,
            ));
        }
    }
    let _ = sqc.close();
    html.push_str("</table>\n");
    Ok(html)
}
// EOF
