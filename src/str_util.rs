// str_util.rs

use chrono::*;

const SHORT_TS_FMT: &str = "%b %d %H:%M";
const SHORT_TS_YEAR_FMT: &str = "%Y %b %d %H:%M";

pub fn ts_fmt(ts: i64) -> String {
    Local
        .from_utc_datetime(&NaiveDateTime::from_timestamp(ts, 0))
        .format(SHORT_TS_FMT)
        .to_string()
}

pub fn ts_y_fmt(ts: i64) -> String {
    Local
        .from_utc_datetime(&NaiveDateTime::from_timestamp(ts, 0))
        .format(SHORT_TS_YEAR_FMT)
        .to_string()
}

pub fn esc_ltgt(input: String) -> String {
    input
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
}

pub fn esc_quot(input: String) -> String {
    input.replace("\"", "&quot;")
}

pub fn sort_dedup_br(input: String) -> String {
    let mut svec = input.split_whitespace().collect::<Vec<&str>>();
    #[allow(clippy::stable_sort_primitive)]
    svec.sort();
    svec.dedup();
    svec.join("<br>")
}

pub fn sql_srch(input: &str) -> String {
    format!(
        "%{}%",
        input.to_lowercase().replace("*", "%").replace("?", "_")
    )
}
// EOF
