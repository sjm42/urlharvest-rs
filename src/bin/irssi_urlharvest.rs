// bin/irssi_urlharvest.rs

use linemux::MuxedLines;
use url::Url;

use urlharvest::*;

const TX_SZ: usize = 1024;
const VEC_SZ: usize = 64;
const CHAN_UNK: &str = "UNKNOWN";
const NICK_UNK: &str = "UNKNOWN";
const MAX_HARVEST_URL_BYTES: usize = 2048;

const RE_HOURMIN: &str = r"^(\d\d):(\d\d)\s";
const RE_DAYCHANGE: &str = r"^--- Day changed \w+ (\w+)\s+(\d+) (\d+)";
const RE_TIMESTAMP: &str = r"^--- Log opened \w+ (\w+)\s+(\d+) (\d+):(\d+):(\d+) (\d+)";

struct IrcCtx {
    ts: i64,
    chan: String,
    msg: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::parse();
    opts.finalize()?;
    opts.start_pgm(env!("CARGO_BIN_NAME"));
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{:#?}", &cfg);

    let db = start_db(&cfg).await?;

    let mut chans: HashMap<ffi::OsString, String> = HashMap::with_capacity(VEC_SZ);
    let mut log_files: Vec<fs::DirEntry> = Vec::with_capacity(VEC_SZ);

    debug!("Scanning dir {}", &cfg.irc_log_dir);
    let re_log = capture_regex(&cfg.regex_log, "regex_log")?;
    for log_fd in fs::read_dir(&cfg.irc_log_dir)? {
        let log_f = log_fd?;
        if let Some(channel) = re_log
            .captures(log_f.file_name().to_string_lossy().as_ref())
            .and_then(|m| m.get(1).map(|c| c.as_str().to_owned()))
        {
            chans.insert(
                log_f
                    .path()
                    .file_name()
                    .ok_or_else(|| anyhow!("no filename"))?
                    .to_os_string(),
                channel,
            );
            log_files.push(log_f);
        }
    }
    debug!("My logfiles: {log_files:?}");
    debug!("My chans: {chans:?}");

    let re_nick = capture_regex(&cfg.regex_nick, "regex_nick")?;
    let re_url = capture_regex(&cfg.regex_url, "regex_url")?;
    let mut lmux = MuxedLines::new()?;
    let chan_unk = CHAN_UNK.to_string();
    if opts.read_history {
        // Seed the database with all the old log lines too
        info!("Reading history...");

        // Save the start time to measure elapsed
        let start_ts = time::Instant::now();

        // *** Pre-compile the regexes here for performance!

        // Match most message lines, example:
        // "13:37 <@sjm> 1337"
        let re_hourmin = Regex::new(RE_HOURMIN)?;

        // Match example line:
        // "--- Day changed Fri Aug 13 2021"
        let re_daychange = Regex::new(RE_DAYCHANGE)?;

        // Match example line:
        // "--- Log opened Sun Aug 08 13:37:42 2021"
        let re_timestamp = Regex::new(RE_TIMESTAMP)?;

        let mut tx_i: usize = 0;
        let mut tx = db.dbc.begin().await?;
        for log_f in &log_files {
            let log_nopath = log_f
                .path()
                .file_name()
                .ok_or_else(|| anyhow!("no filename"))?
                .to_os_string();
            let chan = chans.get(&log_nopath).unwrap_or(&chan_unk);
            let reader = io::BufReader::new(fs::File::open(log_f.path())?);
            let mut current_ts = Local::now();
            for line in reader.lines() {
                let msg = line?;
                // Most common case
                if let Some(new_ts) = detect_hourmin(&re_hourmin, &msg, current_ts) {
                    current_ts = new_ts;
                }
                // Second common case
                else if let Some(new_ts) = detect_daychange(&re_daychange, &msg) {
                    current_ts = new_ts;
                }
                // Least common case
                else if let Some(new_ts) = detect_timestamp(&re_timestamp, &msg) {
                    current_ts = new_ts;
                }

                for url in parse_ircmsg(
                    &cfg,
                    &re_nick,
                    &re_url,
                    IrcCtx {
                        ts: current_ts.timestamp(),
                        chan: chan.to_string(),
                        msg,
                    },
                ) {
                    db_add_url(&mut *tx, &url).await?;
                }
                tx_i += 1;
                if tx_i >= TX_SZ {
                    tx.commit().await?;
                    tx = db.dbc.begin().await?;
                    tx_i = 0;
                }
            }
            // OK all history processed, add the file for live processing from now onwards
            lmux.add_file(log_f.path()).await?;
        }
        tx.commit().await?;
        info!(
            "History read completed in {:.3} s",
            start_ts.elapsed().as_millis() as f64 / 1000.0
        );
    } else {
        for log_f in &log_files {
            lmux.add_file(log_f.path()).await?;
        }
    }

    info!("Starting live processing...");
    while let Some(msg_line) = lmux
        .next_line()
        .await
        .map_err(|e| anyhow!("IRC log reader failed: {e}"))?
    {
        let filename = msg_line.source().file_name().unwrap_or_else(|| ffi::OsStr::new("NONE"));
        let chan = chans.get(filename).unwrap_or(&chan_unk);
        let msg = msg_line.line();

        for url in parse_ircmsg(
            &cfg,
            &re_nick,
            &re_url,
            IrcCtx {
                ts: Utc::now().timestamp(),
                chan: chan.to_string(),
                msg: msg.to_string(),
            },
        ) {
            db_add_url(&db.dbc, &url).await?;
        }
    }

    Ok(())
}

fn detect_hourmin<S: AsRef<str>>(re: &Regex, msg: S, current: DateTime<Local>) -> Option<DateTime<Local>> {
    let m = re.captures(msg.as_ref())?;
    let (hh, mm) = m
        .iter()
        .skip(1)
        .filter_map(|m| m?.as_str().parse::<u32>().ok())
        .collect_tuple()?;

    let naive_ts = current.date_naive().and_time(NaiveTime::from_hms_opt(hh, mm, 0)?);
    // A minute-only log cannot disambiguate the repeated hour at DST fall-back.
    // Choose its first occurrence consistently; invalid spring-forward times are rejected.
    Local.from_local_datetime(&naive_ts).earliest()
}

fn detect_daychange<S: AsRef<str>>(re: &Regex, msg: S) -> Option<DateTime<Local>> {
    let m = re.captures(msg.as_ref())?;
    let date = NaiveDate::parse_from_str(&format!("{} {} {}", &m[3], &m[1], &m[2]), "%Y %b %d").ok()?;
    Local.from_local_datetime(&date.and_hms_opt(0, 0, 0)?).earliest()
}

fn detect_timestamp<S: AsRef<str>>(re: &Regex, msg: S) -> Option<DateTime<Local>> {
    let m = re.captures(msg.as_ref())?;
    let naive = NaiveDateTime::parse_from_str(
        &format!("{} {} {} {}:{}:{}", &m[6], &m[1], &m[2], &m[3], &m[4], &m[5]),
        "%Y %b %d %H:%M:%S",
    )
    .ok()?;
    Local.from_local_datetime(&naive).earliest()
}

fn capture_regex(pattern: &str, name: &str) -> anyhow::Result<Regex> {
    let regex = Regex::new(pattern)?;
    if regex.captures_len() < 2 {
        bail!("{name} requires capture group 1");
    }
    Ok(regex)
}

fn valid_harvest_url(value: &str) -> bool {
    if value.is_empty()
        || value.len() > MAX_HARVEST_URL_BYTES
        || value.chars().any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return false;
    }
    Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

fn parse_ircmsg(cfg: &ConfigCommon, re_nick: &Regex, re_url: &Regex, ctx: IrcCtx) -> Vec<UrlCtx> {
    let mut urls = Vec::new();
    // Do we have nick in the msg?
    let nick = re_nick
        .captures(&ctx.msg)
        .and_then(|m| m.get(1).map(|c| c.as_str().to_owned()))
        .unwrap_or_else(|| NICK_UNK.into());

    'outer: for url_cap in re_url.captures_iter(ctx.msg.as_ref()) {
        let Some(url) = url_cap.get(1).map(|c| c.as_str()).filter(|url| !url.is_empty()) else {
            continue;
        };
        if !valid_harvest_url(url) {
            warn!(
                url_bytes = url.len(),
                "Skipping invalid or oversized URL in {}", ctx.chan
            );
            continue;
        }
        info!("Detected url: {chan} {nick} {url}", chan = ctx.chan);
        for b in &cfg.url_blacklist {
            if url.starts_with(b) {
                info!("Blacklilsted URL.");
                continue 'outer;
            }
        }
        urls.push(UrlCtx {
            ts: ctx.ts,
            chan: ctx.chan.clone(),
            nick: nick.clone(),
            url: url.to_string(),
        });
    }
    urls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_irssi_history_markers_with_named_months_and_padded_days() {
        let day = Regex::new(RE_DAYCHANGE).unwrap();
        let opened = Regex::new(RE_TIMESTAMP).unwrap();
        assert_eq!(
            detect_daychange(&day, "--- Day changed Fri Aug 13 2021")
                .unwrap()
                .date_naive(),
            NaiveDate::from_ymd_opt(2021, 8, 13).unwrap()
        );
        let dt = detect_timestamp(&opened, "--- Log opened Sun Aug  8 13:37:42 2021").unwrap();
        assert_eq!(dt.date_naive(), NaiveDate::from_ymd_opt(2021, 8, 8).unwrap());
        assert_eq!(dt.time(), NaiveTime::from_hms_opt(13, 37, 42).unwrap());
        assert!(detect_daychange(&day, "--- Day changed Fri Feb 30 2021").is_none());
        assert!(detect_timestamp(&opened, "--- Log opened Sun Aug 08 25:37:42 2021").is_none());
    }

    #[test]
    fn updates_minutes_without_changing_history_date() {
        let re = Regex::new(RE_HOURMIN).unwrap();
        let current = Local.with_ymd_and_hms(2021, 8, 8, 13, 0, 0).unwrap();
        let dt = detect_hourmin(&re, "13:37 <nick> hello", current).unwrap();
        assert_eq!(dt.date_naive(), current.date_naive());
        assert_eq!(dt.time(), NaiveTime::from_hms_opt(13, 37, 0).unwrap());
        assert!(detect_hourmin(&re, "25:00 invalid", current).is_none());
    }

    #[test]
    fn validates_capture_groups_and_skips_unmatched_optional_urls() {
        assert!(capture_regex("https?://.+", "regex_url").is_err());
        let cfg: ConfigCommon = serde_json::from_str(include_str!("../../config/urlharvest.json")).unwrap();
        let urls = parse_ircmsg(
            &cfg,
            &Regex::new("(nick)?").unwrap(),
            &Regex::new("(https)?www").unwrap(),
            IrcCtx {
                ts: 1,
                chan: "#test".into(),
                msg: "www".into(),
            },
        );
        assert!(urls.is_empty());
    }

    #[test]
    fn validates_harvest_url_scheme_host_controls_and_storage_byte_limit() {
        for value in [
            "",
            "https://",
            "https://?",
            "file:///tmp/example",
            "https://example.com/a\nb",
            "https://example.com/\0",
            "https://exa mple.com/",
        ] {
            assert!(!valid_harvest_url(value), "{value:?}");
        }
        let prefix = "https://example.com/";
        let exact = format!("{prefix}{}", "a".repeat(MAX_HARVEST_URL_BYTES - prefix.len()));
        assert!(valid_harvest_url(&exact));
        assert!(!valid_harvest_url(&format!("{exact}a")));
        assert!(!valid_harvest_url(&format!("{prefix}{}", "é".repeat(1024))));
    }

    #[test]
    fn skips_bad_urls_and_preserves_the_valid_original_url() {
        let cfg: ConfigCommon = serde_json::from_str(include_str!("../../config/urlharvest.json")).unwrap();
        let good = "https://EXAMPLE.com/é?a=1&b=2";
        let urls = parse_ircmsg(
            &cfg,
            &Regex::new("(nick)").unwrap(),
            &Regex::new(r"(\S+)").unwrap(),
            IrcCtx {
                ts: 1234,
                chan: "#test".into(),
                msg: format!("https://? https://example.com/{} {good}", "x".repeat(2048)),
            },
        );
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, good);
        assert_eq!(urls[0].ts, 1234);
        assert_eq!(urls[0].chan, "#test");
    }
}

// EOF
