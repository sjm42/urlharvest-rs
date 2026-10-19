// bin/urllog_actions.rs

use axum::{
    Router,
    body::Body,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use handlebars::{Handlebars, to_json};

use urlharvest::*;

#[cfg(test)]
#[path = "../../tests/support/mod.rs"]
mod db_test_support;

const TPL_INDEX: &str = "index";
const TPL_RESULT_HEADER: &str = "result_header";
const TPL_RESULT_ROW: &str = "result_row";
const TPL_RESULT_FOOTER: &str = "result_footer";

const DEFAULT_REPLY_CAP: usize = 65536;
const MAX_SEARCH_BYTES: usize = 4096;

struct MyState<'a> {
    index_html: String,
    hb_reg: Handlebars<'a>,
    dbc: sqlx::PgPool,
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

    let db = start_db(&cfg).await?;

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

    let server_addr: net::SocketAddr = cfg.search_listen;

    let my_state = Arc::new(MyState {
        index_html: index_html.clone(),
        hb_reg,
        dbc: db.dbc,
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

fn validate_search(params: &SearchParam) -> Result<(), AppError> {
    for value in [&params.chan, &params.nick, &params.url, &params.title] {
        if value.len() > MAX_SEARCH_BYTES || value.chars().any(char::is_control) {
            return Err(AppError::Params(
                "Search fields must be at most 4096 bytes and contain no control characters".into(),
            ));
        }
    }
    Ok(())
}

const SQL_SEARCH: &str = "with grouped as ( \
    select min(u.id) as id, min(seen) as seen_first, max(seen) as seen_last, count(seen) as seen_count, \
    string_agg(channel, ' ') as channels, string_agg(nick, ' ') as nicks, \
    url, (array_agg(url_meta.title order by url_meta.id desc))[1] as title from url as u \
    inner join url_meta on url_meta.url_id = u.id \
    where lower(channel) like $1 \
    and lower(nick) like $2 \
    and lower(url) like $3 \
    group by url) \
    select * from grouped where lower(title) like $4 \
    order by seen_last desc \
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

    validate_search(&params)?;

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

    let mut st_s = sqlx::query_as::<_, DbRead>(SQL_SEARCH)
        .bind(&chan)
        .bind(&nick)
        .bind(&url)
        .bind(&title)
        .fetch(&state.dbc);

    while let Some(row) = st_s.try_next().await? {
        html.push_str(&render_result_row(&state.hb_reg, &row)?);
    }

    html.push_str(&html_footer);
    Ok(([(header::CACHE_CONTROL, "no-store")], Html(html)).into_response())
}

fn render_result_row(registry: &Handlebars<'_>, row: &DbRead) -> Result<String, handlebars::RenderError> {
    let mut data = serde_json::value::Map::new();
    for (key, value) in [
        ("id", row.id.to_string()),
        ("first_seen", row.seen_first.ts_short_y()),
        ("last_seen", row.seen_last.ts_short()),
        ("num_seen", row.seen_count.to_string()),
        ("chans", row.channels.esc_et_lt_gt().sort_dedup_br()),
        ("nicks", row.nicks.esc_et_lt_gt().sort_dedup_br()),
        ("url", row.url.esc_et_lt_gt().esc_quot()),
        ("title", row.title.esc_et_lt_gt()),
    ] {
        data.insert(key.to_owned(), to_json(value));
    }
    registry.render(TPL_RESULT_ROW, &data)
}

#[derive(Debug, Deserialize)]
pub struct RemoveParam {
    id: String,
}

fn parse_action_id(value: &str) -> Result<i32, AppError> {
    value
        .parse::<i32>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| AppError::Params("ID must be a positive 32-bit integer".into()))
}

const SQL_REMOVE_URL: &str = "delete from url where url in (select url from url where id = $1)";
async fn remove_url<'a>(
    State(state): State<Arc<MyState<'a>>>,
    Query(params): Query<RemoveParam>,
) -> Result<Response<Body>, AppError> {
    info!("remove_url({params:?})");
    let id = parse_action_id(&params.id)?;
    info!("Remove url id {id}");

    let db_res = sqlx::query(SQL_REMOVE_URL).bind(id).execute(&state.dbc).await?;
    let n_rows = db_res.rows_affected();

    let msg = format!("Removed #{n_rows}");
    info!("{msg}");
    Ok(([(header::CACHE_CONTROL, "no-store")], msg).into_response())
}

const SQL_REMOVE_META: &str = "delete from url_meta where url_id = $1";
async fn remove_meta<'a>(
    State(state): State<Arc<MyState<'a>>>,
    Query(params): Query<RemoveParam>,
) -> Result<Response<Body>, AppError> {
    info!("remove_meta({params:?})");
    let id = parse_action_id(&params.id)?;
    info!("Remove meta id {id}");

    let db_res = sqlx::query(SQL_REMOVE_META).bind(id).execute(&state.dbc).await?;
    let n_rows = db_res.rows_affected();

    let msg = format!("Refreshing (#{n_rows})");
    info!("{msg}");
    Ok(([(header::CACHE_CONTROL, "no-store")], msg).into_response())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn actions_reject_invalid_ids_before_accessing_the_database() {
        let state = Arc::new(MyState {
            index_html: String::new(),
            hb_reg: Handlebars::new(),
            dbc: sqlx::PgPool::connect_lazy("postgres://localhost:1/urlharvest_test").unwrap(),
        });
        assert_eq!(parse_action_id("42").ok(), Some(42));
        assert_eq!(parse_action_id("2147483647").ok(), Some(i32::MAX));
        for id in ["", "abc", "42x", "0", "-1", "2147483648", " 42"] {
            let params = || Query(RemoveParam { id: id.to_owned() });
            for result in [
                remove_url(State(state.clone()), params()).await,
                remove_meta(State(state.clone()), params()).await,
            ] {
                let response = match result {
                    Ok(_) => panic!("invalid ID {id:?} was accepted"),
                    Err(error) => error.into_response(),
                };
                assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{id:?}");
                assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            }
        }
        assert_eq!(state.dbc.size(), 0);
    }

    #[test]
    fn renders_urls_without_interpreting_entities_or_markup() {
        let mut registry = Handlebars::new();
        registry.register_escape_fn(handlebars::no_escape);
        registry
            .register_template_string(
                TPL_RESULT_ROW,
                include_str!("../../templates/search_result_row.html.hbs"),
            )
            .unwrap();
        let url = r#"https://example.com/?a=&copy;&b="><img/src=x>"#;
        let escaped = "https://example.com/?a=&amp;copy;&amp;b=&quot;&gt;&lt;img/src=x&gt;";
        let row = DbRead {
            id: 1,
            seen_first: 1,
            seen_last: 2,
            seen_count: 2,
            channels: "#test".into(),
            nicks: "test".into(),
            url: url.into(),
            title: "Example".into(),
        };
        let html = render_result_row(&registry, &row).unwrap();
        assert!(html.contains(&format!("<a href=\"{escaped}\">{escaped}</a>")));
        assert!(!html.contains("<img"));
        assert_eq!(row.url, url);
    }

    #[tokio::test]
    #[ignore = "requires URLHARVEST_TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
    async fn search_matches_the_latest_title_and_keeps_all_matching_sightings() {
        let mut conn = db_test_support::metadata_fixture().await;
        for (refreshed, expected_title) in [(false, "Current title"), (true, "Refreshed title")] {
            if refreshed {
                sqlx::query(SQL_REMOVE_META)
                    .bind(1_i32)
                    .execute(&mut conn)
                    .await
                    .unwrap();
                sqlx::query("insert into url_meta (url_id, title) values (1, 'Refreshed title')")
                    .execute(&mut conn)
                    .await
                    .unwrap();
            }
            for title in [
                "%".to_owned(),
                expected_title.to_lowercase().sql_search(),
                "%old title%".into(),
            ] {
                let rows = sqlx::query_as::<_, DbRead>(SQL_SEARCH)
                    .bind("%#test%")
                    .bind("%")
                    .bind("https://example.com/")
                    .bind(&title)
                    .fetch_all(&mut conn)
                    .await
                    .unwrap();
                if title == "%old title%" {
                    assert!(rows.is_empty());
                } else {
                    assert_eq!(rows.len(), 1);
                    assert_eq!(rows[0].id, 1);
                    assert_eq!(rows[0].title, expected_title);
                    assert_eq!(rows[0].seen_count, 2);
                    assert_eq!(rows[0].seen_first, 20);
                    assert_eq!(rows[0].seen_last, 30);
                    assert!(rows[0].nicks.contains("alice"));
                    assert!(rows[0].nicks.contains("bob"));
                }
            }
        }
        let rows = sqlx::query_as::<_, DbRead>(SQL_SEARCH)
            .bind("%#test%")
            .bind("%")
            .bind("https://example.com/")
            .bind("%current title%")
            .fetch_all(&mut conn)
            .await
            .unwrap();
        assert!(rows.is_empty(), "a superseded title must not match after refresh");
    }

    #[test]
    fn search_accepts_channels_unicode_urls_and_sql_punctuation() {
        assert!(
            validate_search(&SearchParam {
                chan: "#42".into(),
                nick: "Mäkinen".into(),
                url: "https://example.com/?a=1&b=2".into(),
                title: "It's 100% ' OR 1=1 --".into(),
            })
            .is_ok()
        );
    }

    #[test]
    fn search_rejects_controls_and_oversized_fields() {
        for value in ["a\0b".to_owned(), "a\nb".to_owned(), "é".repeat(2049)] {
            assert!(
                validate_search(&SearchParam {
                    chan: value,
                    nick: String::new(),
                    url: String::new(),
                    title: String::new(),
                })
                .is_err()
            );
        }
    }
}

// EOF
