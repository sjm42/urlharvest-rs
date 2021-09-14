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

pub fn esc_ltgt<S: AsRef<str>>(input: S) -> String {
    input
        .as_ref()
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
}

pub fn esc_quot<S: AsRef<str>>(input: S) -> String {
    input.as_ref().replace("\"", "&quot;")
}

pub fn sort_dedup_br<S: AsRef<str>>(input: S) -> String {
    let mut svec = input.as_ref().split_whitespace().collect::<Vec<&str>>();
    #[allow(clippy::stable_sort_primitive)]
    svec.sort();
    svec.dedup();
    svec.join("<br>")
}

pub fn sql_srch<S: AsRef<str>>(input: S) -> String {
    format!(
        "%{}%",
        input
            .as_ref()
            .to_lowercase()
            .replace("*", "%")
            .replace("?", "_")
    )
}
// EOF
