// str_util.rs

use chrono::*;

const TS_FMT_LONG: &str = "%Y-%m-%d %H:%M:%S";
const TS_FMT_SHORT: &str = "%b %d %H:%M";
const TS_FMT_YEAR_SHORT: &str = "%Y %b %d %H:%M";

pub fn ts_fmt<S: AsRef<str>>(fmt: S, ts: i64) -> String {
    Local
        .from_utc_datetime(&NaiveDateTime::from_timestamp(ts, 0))
        .format(fmt.as_ref())
        .to_string()
}

pub fn ts_long(ts: i64) -> String {
    ts_fmt(TS_FMT_LONG, ts)
}

pub fn ts_short(ts: i64) -> String {
    ts_fmt(TS_FMT_SHORT, ts)
}

pub fn ts_y_short(ts: i64) -> String {
    ts_fmt(TS_FMT_YEAR_SHORT, ts)
}

pub trait EscLtGt {
    fn esc_ltgt(self) -> Self;
}
impl EscLtGt for String {
    fn esc_ltgt(self) -> Self {
        self.replace("&", "&amp;")
            .replace("<", "&lt;")
            .replace(">", "&gt;")
    }
}

pub trait EscQuot {
    fn esc_quot(self) -> Self;
}
impl EscQuot for String {
    fn esc_quot(self) -> Self {
        self.replace("\"", "&quot;")
    }
}

pub trait SortDedupBr {
    fn sort_dedup_br(self) -> Self;
}
impl SortDedupBr for String {
    fn sort_dedup_br(self) -> Self {
        let mut svec = self.split_whitespace().collect::<Vec<&str>>();
        svec.sort_unstable();
        svec.dedup();
        svec.join("<br>")
    }
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
