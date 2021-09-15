// urllog-search.rs

use handlebars::{to_json, Handlebars};
use log::*;
use regex::Regex;
use rusqlite::Connection;
use serde_derive::Deserialize;
use serde_json::value::Map;
use std::{error::Error, net::SocketAddr};
use structopt::StructOpt;
use warp::Filter;

use urlharvest::*;

const INDEX_PATH: &str = "$HOME/urllog/templates2/search.html.hbs";
const INDEX_NAME: &str = "index";
const REQ_PATH_SEARCH: &str = "search";
const RE_SEARCH: &str = r#"^[-_\.:/0-9a-zA-Z\?\* ]*$"#;
const RESULT_MAXSZ: usize = 100;

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
async fn main() -> Result<(), Box<dyn Error>> {
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
    let mut index_path = INDEX_PATH.to_string();
    expand_home(&mut index_path)?;

    let mut hb_reg = Handlebars::new();
    hb_reg.register_template_file(INDEX_NAME, &index_path)?;
    let mut tpl_data = Map::new();
    tpl_data.insert("cmd_search".into(), to_json("search"));
    let index_html = hb_reg.render(INDEX_NAME, &tpl_data)?;

    // GET / -> index html
    let req_index = warp::get()
        .and(warp::path::end())
        .map(move || my_response(TEXT_HTML, index_html.clone()));

    let server_addr: SocketAddr = opts.listen.parse()?;
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
                return my_response(TEXT_PLAIN, "*** Illegal characters in query ***\n".into());
            }
            match search(&opts.c.db_file, &sql_search, &s) {
                Ok(result) => my_response(TEXT_HTML, result),
                Err(e) => my_response(TEXT_PLAIN, format!("Query error: {:?}", e)),
            }
        });

    let req_routes = req_search.or(req_index);
    warp::serve(req_routes).run(server_addr).await;
    Ok(())
}

fn my_response(
    resp_type: &str,
    resp_body: String,
) -> Result<warp::http::Response<String>, warp::http::Error> {
    warp::http::Response::builder()
        .header("cache-control", "no-store")
        .header("content-type", resp_type)
        .body(resp_body)
}

fn search(db: &str, sql: &str, srch: &SearchParam) -> Result<String, Box<dyn Error>> {
    info!("search({:?})", srch);
    let chan = sql_srch(&srch.chan);
    let nick = sql_srch(&srch.nick);
    let url = sql_srch(&srch.url);
    let title = sql_srch(&srch.title);
    info!("Search {} {} {} {}", chan, nick, url, title);

    let mut html = r#"<table>
    <tr>
    <th>ID</th>
    <th>First seen</th>
    <th>Last seen</th>
    <th>#</th>
    <th>Channel</th>
    <th>Nick</th>
    <th>Title + URL</th>
    </tr>"#
        .to_string();
    let sqc = Connection::open(db)?;
    {
        let mut st_s = sqc.prepare(sql)?;
        let mut rows = st_s.query(&[&chan, &nick, &url, &title])?;
        while let Some(row) = rows.next()? {
            let id = row.get::<usize, i64>(0)?;
            let first_seen = ts_y_fmt(row.get::<usize, i64>(1)?);
            let last_seen = ts_fmt(row.get::<usize, i64>(2)?);
            let num_seen = row.get::<usize, u64>(3)?;
            let chans = sort_dedup_br(esc_ltgt(row.get::<usize, String>(4)?));
            let nicks = sort_dedup_br(esc_ltgt(row.get::<usize, String>(5)?));
            let url = esc_quot(row.get::<usize, String>(6)?);
            let title = esc_ltgt(row.get::<usize, String>(7)?);

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
