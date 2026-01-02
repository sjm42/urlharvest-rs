// bin/urllog_actions.rs

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::*,
};
// use axum_macros::debug_handler;
// provides `try_next`
use futures::TryStreamExt;
use handlebars::{to_json, Handlebars};
use itertools::Itertools;

use urlharvest::*;

const TPL_INDEX: &str = "index";
const TPL_RESULT_HEADER: &str = "result_header";
const TPL_RESULT_ROW: &str = "result_row";
const TPL_RESULT_FOOTER: &str = "result_footer";

const DEFAULT_REPLY_CAP: usize = 65536;
const RE_SEARCH: &str = r"^[-_\.:;/0-9a-zA-Z\?\*\(\)\[\]\{\}\|\\ ]*$";

struct MyState<'a> {
    index_html: String,
    re_search: Regex,
    hb_reg: Handlebars<'a>,
    db_url: String,
}

enum AppError {
    Params(String),
    Render(handlebars::RenderError),
    Sqlx(sqlx::Error),
    Stream(std::io::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response<Body> {
        let (status, message) = match self {
            AppError::Params(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Render(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Template render error: {e}")),
            AppError::Sqlx(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("SQLx error: {e}")),
            AppError::Stream(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Iterator error: {e}")),
        };
        (status, [(header::CACHE_CONTROL, "no-store")], message).into_response()
    }
}
impl From<handlebars::RenderError> for AppError {
    fn from(e: handlebars::RenderError) -> Self {
        Self::Render(e)
    }
}
impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        Self::Sqlx(e)
    }
}
impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Stream(e)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::parse();
    opts.finalize()?;
    opts.start_pgm(env!("CARGO_BIN_NAME"));
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{:#?}", &cfg);

    // Just check and init the database if necessary,
    // and then drop the connection immediately.
    {
        let _db = start_db(&cfg).await?;
    }

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
        path::Path::new(&cfg.template_dir).join(*t)
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
    let re_search = Regex::new(RE_SEARCH)?;

    let server_addr: net::SocketAddr = cfg.search_listen;

    let my_state = Arc::new(MyState {
        index_html: index_html.clone(),
        re_search,
        hb_reg,
        db_url: cfg.db_url.clone(),
    });

    let app = Router::new()
        .route("/", get(get_index).options(options))
        .route("/search", get(search))
        .route("/remove_url", get(remove_url))
        .route("/remove_meta", get(remove_meta))
        .with_state(my_state);

    let listener = tokio::net::TcpListener::bind(&server_addr).await?;
    info!("API server listening to {server_addr}");
    Ok(axum::serve(listener, app.into_make_service()).await?)
}

async fn options<'a>(State(_state): State<Arc<MyState<'a>>>) -> Response<Body> {
    (
        StatusCode::OK,
        [
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            (header::ACCESS_CONTROL_ALLOW_METHODS, "get,post"),
            (header::ACCESS_CONTROL_ALLOW_HEADERS, "content-type"),
        ],
    )
        .into_response()
}

async fn get_index<'a>(State(state): State<Arc<MyState<'a>>>) -> Response<Body> {
    (StatusCode::OK, Html(state.index_html.clone())).into_response()
}

#[derive(Debug, Deserialize)]
pub struct SearchParam {
    chan: String,
    nick: String,
    url: String,
    title: String,
}

const SQL_SEARCH: &str = "select min(u.id) as id, min(seen) as seen_first, max(seen) as seen_last, count(seen) as seen_count, \
    string_agg(channel, ' ') as channels, string_agg(nick, ' ') as nicks, \
    url, any_value(url_meta.title) as title from url as u \
    inner join url_meta on url_meta.url_id = u.id \
    where lower(channel) like $1 \
    and lower(nick) like $2 \
    and lower(url) like $3 \
    and lower(url_meta.title) like $4 \
    group by url \
    order by max(seen) desc \
    limit 255";
#[derive(Debug, sqlx::FromRow)]
struct DbRead {
    id: i32,
    seen_first: i64,
    seen_last: i64,
    seen_count: i64,
    channels: String,
    nicks: String,
    url: String,
    title: String,
}
async fn search<'a>(
    State(state): State<Arc<MyState<'a>>>,
    Query(params): Query<SearchParam>,
) -> Result<Response, AppError> {
    info!("search({params:?})");

    let re = &state.re_search;
    if !(re.is_match(&params.chan)
        && re.is_match(&params.nick)
        && re.is_match(&params.url)
        && re.is_match(&params.title))
    {
        return Err(AppError::Params("Invalid search parameters".to_string()));
    }

    let chan = params.chan.sql_search();
    let nick = params.nick.sql_search();
    let url = params.url.sql_search();
    let title = params.title.sql_search();
    info!("Search {chan} {nick} {url} {title}");

    let tpl_data_empty = serde_json::value::Map::new();
    let html_header = state.hb_reg.render(TPL_RESULT_HEADER, &tpl_data_empty)?;
    let html_footer = state.hb_reg.render(TPL_RESULT_FOOTER, &tpl_data_empty)?;

    let mut html = String::with_capacity(DEFAULT_REPLY_CAP);
    html.push_str(&html_header);

    let dbc = sqlx::PgPool::connect(&state.db_url).await?;
    let mut st_s = sqlx::query_as::<_, DbRead>(SQL_SEARCH)
        .bind(&chan)
        .bind(&nick)
        .bind(&url)
        .bind(&title)
        .fetch(&dbc);

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
        html.push_str(&state.hb_reg.render(TPL_RESULT_ROW, &tpl_data_row)?);
    }

    html.push_str(&html_footer);
    Ok(([(header::CACHE_CONTROL, "no-store")], Html(html)).into_response())
}

#[derive(Debug, Deserialize)]
pub struct RemoveParam {
    id: String,
}
const SQL_REMOVE_URL: &str = "delete from url where url in (select url from url where id = $1)";
async fn remove_url<'a>(
    State(state): State<Arc<MyState<'a>>>,
    Query(params): Query<RemoveParam>,
) -> Result<Response<Body>, AppError> {
    info!("remove_url({params:?})");
    let id = params.id.parse::<i32>().unwrap_or_default();
    info!("Remove url id {id}");

    let dbc = sqlx::PgPool::connect(&state.db_url).await?;
    let db_res = sqlx::query(SQL_REMOVE_URL).bind(id).execute(&dbc).await?;
    let n_rows = db_res.rows_affected();

    let msg = format!("Removed #{n_rows}");
    info!("{msg}");
    db_mark_change(&dbc).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], msg).into_response())
}

const SQL_REMOVE_META: &str = "delete from url_meta where url_id = $1";
async fn remove_meta<'a>(
    State(state): State<Arc<MyState<'a>>>,
    Query(params): Query<RemoveParam>,
) -> Result<Response<Body>, AppError> {
    info!("remove_meta({params:?})");
    let id = params.id.parse::<i32>().unwrap_or_default();
    info!("Remove meta id {id}");

    let dbc = sqlx::PgPool::connect(&state.db_url).await?;
    let db_res = sqlx::query(SQL_REMOVE_META).bind(id).execute(&dbc).await?;
    let n_rows = db_res.rows_affected();

    let msg = format!("Refreshing (#{n_rows})");
    info!("{msg}");
    db_mark_change(&dbc).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], msg).into_response())
}
// EOF
