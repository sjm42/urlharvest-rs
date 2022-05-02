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

pub trait TimeStampFormats {
    fn ts_long(self) -> String;
    fn ts_short(self) -> String;
    fn ts_y_short(self) -> String;
}
impl TimeStampFormats for i64 {
    fn ts_long(self) -> String {
        ts_fmt(TS_FMT_LONG, self)
    }

    fn ts_short(self) -> String {
        ts_fmt(TS_FMT_SHORT, self)
    }

    fn ts_y_short(self) -> String {
        ts_fmt(TS_FMT_YEAR_SHORT, self)
    }
}

pub trait EscLtGt {
    fn esc_ltgt(self) -> String;
}
impl<S> EscLtGt for S
where
    S: AsRef<str>,
{
    fn esc_ltgt(self) -> String {
        self.as_ref()
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
}

pub trait EscQuot {
    fn esc_quot(self) -> String;
}
impl<S> EscQuot for S
where
    S: AsRef<str>,
{
    fn esc_quot(self) -> String {
        self.as_ref().replace('\"', "&quot;")
    }
}

pub trait SortDedupBr {
    fn sort_dedup_br(self) -> String;
}
impl<S> SortDedupBr for S
where
    S: AsRef<str>,
{
    fn sort_dedup_br(self) -> String {
        let mut svec = self.as_ref().split_whitespace().collect::<Vec<&str>>();
        svec.sort_unstable();
        svec.dedup();
        svec.join("<br>")
    }
}

pub trait StringSqlSearch {
    fn sql_search(self) -> String;
}
impl<S> StringSqlSearch for S
where
    S: AsRef<str>,
{
    fn sql_search(self) -> String {
        format!(
            "%{}%",
            self.as_ref()
                .to_lowercase()
                .replace('*', "%")
                .replace('?', "_")
        )
    }
}
// EOF
