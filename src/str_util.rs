// str_util.rs

use chrono::*;
use chrono_tz::Tz;

const TS_FMT_LONG: &str = "%Y-%m-%d %H:%M:%S";
const TS_FMT_SHORT: &str = "%b %d %H:%M";
const TS_FMT_SHORT_YEAR: &str = "%Y %b %d %H:%M";
const TS_NONE: &str = "(none)";

pub fn ts_fmt<S: AsRef<str>>(fmt: S, ts: i64, tz: &Tz) -> String {
    if ts == 0 {
        TS_NONE.to_string()
    } else {
        DateTime::from_timestamp(ts, 0).map_or_else(
            || TS_NONE.to_string(),
            |ts| ts.with_timezone(tz).format(fmt.as_ref()).to_string(),
        )
    }
}

pub trait TimeStampFormats {
    fn ts_long(self) -> String;
    fn ts_short(self) -> String;
    fn ts_short_y(self) -> String;
    fn ts_long_tz(self, tz: &Tz) -> String;
    fn ts_short_tz(self, tz: &Tz) -> String;
    fn ts_short_y_tz(self, tz: &Tz) -> String;

}
impl TimeStampFormats for i64 {
    fn ts_long(self) -> String {
        ts_fmt(TS_FMT_LONG, self, &Tz::UTC)
    }

    fn ts_short(self) -> String {
        ts_fmt(TS_FMT_SHORT, self, &Tz::UTC)
    }

    fn ts_short_y(self) -> String {
        ts_fmt(TS_FMT_SHORT_YEAR, self, &Tz::UTC)
    }
    fn ts_long_tz(self, tz: &Tz) -> String {
        ts_fmt(TS_FMT_LONG, self, tz)
    }

    fn ts_short_tz(self, tz: &Tz) -> String {
        ts_fmt(TS_FMT_SHORT, self, tz)
    }

    fn ts_short_y_tz(self, tz: &Tz) -> String {
        ts_fmt(TS_FMT_SHORT_YEAR, self, tz)
    }

}

pub trait EscEtLtGt {
    fn esc_et_lt_gt(self) -> String;
}
impl<S> EscEtLtGt for S
where
    S: AsRef<str>,
{
    fn esc_et_lt_gt(self) -> String {
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

pub trait CollapseWhiteSpace {
    fn ws_collapse(self) -> String;
}
impl<S> CollapseWhiteSpace for S
where
    S: AsRef<str>,
{
    fn ws_collapse(self) -> String {
        self.as_ref()
            .split_whitespace()
            .collect::<Vec<&str>>()
            .join(" ")
    }
}

// EOF
