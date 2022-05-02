// irssi-urlharvest.rs

use chrono::*;
use linemux::MuxedLines;
use log::*;
use regex::Regex;
use std::fs::{self, DirEntry, File};
use std::io::{BufRead, BufReader};
use std::{collections::HashMap, ffi::*, time::Instant};
use structopt::StructOpt;

use urlharvest::*;

const TX_SZ: usize = 1024;
const VEC_SZ: usize = 64;
const CHAN_UNK: &str = "UNKNOWN";
const NICK_UNK: &str = "UNKNOWN";

struct IrcCtx<'a> {
    re_nick: &'a Regex,
    re_url: &'a Regex,
    ts: i64,
    chan: &'a str,
    msg: &'a str,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::from_args();
    opts.finish()?;
    start_pgm(&opts, "URL harvester");
    let cfg = ConfigCommon::new(&opts)?;
    let mut db = start_db(&cfg)?;

    let mut chans: HashMap<OsString, String> = HashMap::with_capacity(VEC_SZ);
    let mut log_files: Vec<DirEntry> = Vec::with_capacity(VEC_SZ);

    debug!("Scanning dir {}", &cfg.irc_log_dir);
    let re_log = Regex::new(&cfg.regex_log)?;
    for log_fd in fs::read_dir(&cfg.irc_log_dir)? {
        let log_f = log_fd?;
        if let Some(re_match) = re_log.captures(log_f.file_name().to_string_lossy().as_ref()) {
            chans.insert(
                log_f.path().file_name().unwrap().to_os_string(),
                re_match[1].to_owned(),
            );
            log_files.push(log_f);
        }
    }
    debug!("My logfiles: {log_files:?}");
    debug!("My chans: {chans:?}");

    let re_nick = &Regex::new(&cfg.regex_nick)?;
    let re_url = &Regex::new(&cfg.regex_url)?;
    let mut lmux = MuxedLines::new()?;
    let chan_unk = CHAN_UNK.to_owned();
    if opts.read_history {
        // Seed the database with all the old log lines too
        info!("Reading history...");

        // Save the start time to measure elapsed
        let start_ts = Instant::now();

        // *** Pre-compile the regexes here for performance!

        // Match most message lines, example:
        // "13:37 <@sjm> 1337"
        let re_hourmin = Regex::new(r#"^(\d\d):(\d\d)\s"#)?;

        // Match example line:
        // "--- Day changed Fri Aug 13 2021"
        let re_daychange = Regex::new(r#"^--- Day changed \w+ (\w+) (\d+) (\d+)"#)?;

        // Match example line:
        // "--- Log opened Sun Aug 08 13:37:42 2021"
        let re_timestamp =
            Regex::new(r#"^--- Log opened \w+ (\w+) (\d+) (\d+):(\d+):(\d+) (\d+)"#)?;

        let mut tx_i: usize = 0;
        db.dbc.execute_batch("begin")?;
        for log_f in &log_files {
            let log_nopath = log_f.path().file_name().unwrap().to_os_string();
            let chan = chans.get(&log_nopath).unwrap_or(&chan_unk);
            let reader = BufReader::new(File::open(log_f.path())?);
            let mut current_ts = Local::now();
            for (_index, line) in reader.lines().enumerate() {
                let msg = line?;
                tx_i += 1;
                if tx_i >= TX_SZ {
                    db.dbc.execute_batch("commit")?;
                    db.dbc.execute_batch("begin")?;
                    tx_i = 0;
                }

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

                handle_ircmsg(
                    &db,
                    IrcCtx {
                        re_nick,
                        re_url,
                        ts: current_ts.timestamp(),
                        chan,
                        msg: &msg,
                    },
                )?;
            }
            // OK all history processed, add the file for live processing from now onwards
            lmux.add_file(log_f.path()).await?;
        }
        db.dbc.execute_batch("commit")?;
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
    db.update_change = true;
    while let Ok(Some(msg_line)) = lmux.next_line().await {
        let filename = msg_line
            .source()
            .file_name()
            .unwrap_or_else(|| OsStr::new("NONE"));
        let chan = chans.get(filename).unwrap_or(&chan_unk);
        let msg = msg_line.line();

        handle_ircmsg(
            &db,
            IrcCtx {
                re_nick,
                re_url,
                ts: Utc::now().timestamp(),
                chan,
                msg,
            },
        )?;
    }
    Ok(())
}

fn detect_hourmin<S: AsRef<str>>(
    re: &Regex,
    msg: S,
    current: DateTime<Local>,
) -> Option<DateTime<Local>> {
    if let Some(re_match) = re.captures(msg.as_ref()) {
        let hh = &re_match[1];
        let mm = &re_match[2];
        let s = format!("{}{}00", hh, mm);
        if let Ok(new_localtime) = NaiveTime::parse_from_str(&s, "%H%M%S") {
            return current.date().and_time(new_localtime);
        }
    }
    None
}

fn detect_daychange<S: AsRef<str>>(re: &Regex, msg: S) -> Option<DateTime<Local>> {
    if let Some(re_match) = re.captures(msg.as_ref()) {
        let mon = &re_match[1];
        let day = &re_match[2];
        let year = &re_match[3];
        let s = format!("{}{}{}", year, mon, day);
        if let Ok(naive_date) = NaiveDate::parse_from_str(&s, "%Y%b%d") {
            let naive_ts = naive_date.and_hms(0, 0, 0);
            if let LocalResult::Single(new_ts) = Local.from_local_datetime(&naive_ts) {
                trace!("Found daychange {new_ts:?}");
                return Some(new_ts);
            }
        }
    }
    None
}

fn detect_timestamp<S: AsRef<str>>(re: &Regex, msg: S) -> Option<DateTime<Local>> {
    if let Some(re_match) = re.captures(msg.as_ref()) {
        let mon = &re_match[1];
        let day = &re_match[2];
        let hh = &re_match[3];
        let mm = &re_match[4];
        let ss = &re_match[5];
        let year = &re_match[6];
        let s = format!("{}{}{}-{}{}{}", year, mon, day, hh, mm, ss);
        if let Ok(naive_ts) = NaiveDateTime::parse_from_str(&s, "%Y%b%d-%H%M%S") {
            if let LocalResult::Single(new_ts) = Local.from_local_datetime(&naive_ts) {
                trace!("Found timestamp {new_ts:?}");
                return Some(new_ts);
            }
        }
    }
    None
}

fn handle_ircmsg(db: &DbCtx, ctx: IrcCtx) -> anyhow::Result<()> {
    // Do we have nick in the msg?
    let nick = &match ctx.re_nick.captures(ctx.msg) {
        Some(nick_match) => nick_match[1].to_owned(),
        None => NICK_UNK.into(),
    };

    for url_cap in ctx.re_url.captures_iter(ctx.msg.as_ref()) {
        let url = &url_cap[1];
        info!("Detected url: {chan} {nick} {url}", chan = ctx.chan);
        db_add_url(
            db,
            UrlCtx {
                ts: ctx.ts,
                chan: ctx.chan,
                nick,
                url,
            },
        )?;
    }
    Ok(())
}
// EOF
