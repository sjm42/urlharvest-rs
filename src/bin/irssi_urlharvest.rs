// irssi-urlharvest.rs

use anyhow::anyhow;
use chrono::*;
use itertools::Itertools;
use linemux::MuxedLines;
use log::*;
use regex::Regex;
use sqlx::Executor;
use std::fs::{self, DirEntry, File};
use std::io::{BufRead, BufReader};
use std::{collections::HashMap, ffi::*, time::Instant};
use structopt::StructOpt;

use urlharvest::*;

const TX_SZ: usize = 1024;
const VEC_SZ: usize = 64;
const CHAN_UNK: &str = "UNKNOWN";
const NICK_UNK: &str = "UNKNOWN";

struct IrcCtx {
    ts: i64,
    chan: String,
    msg: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = OptsCommon::from_args();
    opts.finish()?;
    opts.start_pgm(env!("CARGO_BIN_NAME"));
    let cfg = ConfigCommon::new(&opts)?;
    debug!("Config:\n{:#?}", &cfg);

    let mut db = start_db(&cfg).await?;

    let mut chans: HashMap<OsString, String> = HashMap::with_capacity(VEC_SZ);
    let mut log_files: Vec<DirEntry> = Vec::with_capacity(VEC_SZ);

    debug!("Scanning dir {}", &cfg.irc_log_dir);
    let re_log = Regex::new(&cfg.regex_log)?;
    for log_fd in fs::read_dir(&cfg.irc_log_dir)? {
        let log_f = log_fd?;
        if let Some(re_match) = re_log.captures(log_f.file_name().to_string_lossy().as_ref()) {
            chans.insert(
                log_f
                    .path()
                    .file_name()
                    .ok_or_else(|| anyhow!("no filename"))?
                    .to_os_string(),
                re_match[1].to_owned(),
            );
            log_files.push(log_f);
        }
    }
    debug!("My logfiles: {log_files:?}");
    debug!("My chans: {chans:?}");

    let re_nick = Regex::new(&cfg.regex_nick)?;
    let re_url = Regex::new(&cfg.regex_url)?;
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
        db.dbc.execute("BEGIN").await?;
        for log_f in &log_files {
            let log_nopath = log_f
                .path()
                .file_name()
                .ok_or_else(|| anyhow!("no filename"))?
                .to_os_string();
            let chan = chans.get(&log_nopath).unwrap_or(&chan_unk);
            let reader = BufReader::new(File::open(log_f.path())?);
            let mut current_ts = Local::now();
            for (_index, line) in reader.lines().enumerate() {
                let msg = line?;
                tx_i += 1;
                if tx_i >= TX_SZ {
                    db.dbc.execute("COMMIT").await?;
                    db.dbc.execute("BEGIN").await?;
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
                    &cfg,
                    &mut db,
                    &re_nick,
                    &re_url,
                    IrcCtx {
                        ts: current_ts.timestamp(),
                        chan: chan.to_owned(),
                        msg,
                    },
                )
                .await?;
            }
            // OK all history processed, add the file for live processing from now onwards
            lmux.add_file(log_f.path()).await?;
        }
        db.dbc.execute("COMMIT").await?;
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
            &cfg,
            &mut db,
            &re_nick,
            &re_url,
            IrcCtx {
                ts: Utc::now().timestamp(),
                chan: chan.to_owned(),
                msg: msg.to_owned(),
            },
        )
        .await?;
    }

    Ok(())
}

fn detect_hourmin<S: AsRef<str>>(
    re: &Regex,
    msg: S,
    current: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let m = re.captures(msg.as_ref())?;
    let (hh, mm) = m
        .iter()
        .skip(1)
        .filter_map(|m| Some(m?.as_str().parse::<u32>().ok()?))
        .collect_tuple()?;
    current.date().and_time(NaiveTime::from_hms_opt(hh, mm, 0)?)
}

fn detect_daychange<S: AsRef<str>>(re: &Regex, msg: S) -> Option<DateTime<Local>> {
    let m = re.captures(msg.as_ref())?;
    let (mon, day, year) = m
        .iter()
        .skip(1)
        .filter_map(|m| Some(m?.as_str().parse::<u32>().ok()?))
        .collect_tuple()?;
    let naive_ts = NaiveDate::from_ymd_opt(year as i32, mon, day)?.and_hms(0, 0, 0);
    if let LocalResult::Single(new_ts) = Local.from_local_datetime(&naive_ts) {
        trace!("Found daychange {new_ts:?}");
        return Some(new_ts);
    }
    None
}

fn detect_timestamp<S: AsRef<str>>(re: &Regex, msg: S) -> Option<DateTime<Local>> {
    let m = re.captures(msg.as_ref())?;
    let (mon, day, hh, mm, ss, year) = m
        .iter()
        .skip(1)
        .filter_map(|m| Some(m?.as_str().parse::<u32>().ok()?))
        .collect_tuple()?;
    let naive_ts = NaiveDate::from_ymd_opt(year as i32, mon, day)?.and_hms_opt(hh, mm, ss)?;
    if let LocalResult::Single(new_ts) = Local.from_local_datetime(&naive_ts) {
        trace!("Found timestamp {new_ts:?}");
        return Some(new_ts);
    }
    None
}

async fn handle_ircmsg(
    cfg: &ConfigCommon,
    db: &mut DbCtx,
    re_nick: &Regex,
    re_url: &Regex,
    ctx: IrcCtx,
) -> anyhow::Result<()> {
    // Do we have nick in the msg?
    let nick = &match re_nick.captures(&ctx.msg) {
        Some(nick_match) => nick_match[1].to_owned(),
        None => NICK_UNK.into(),
    };

    'outer: for url_cap in re_url.captures_iter(ctx.msg.as_ref()) {
        let url = &url_cap[1];
        info!("Detected url: {chan} {nick} {url}", chan = ctx.chan);
        for b in &cfg.url_blacklist {
            if url.starts_with(b) {
                info!("Blacklilsted URL.");
                continue 'outer;
            }
        }
        info!(
            "Inserted {} row(s)",
            db_add_url(
                db,
                &UrlCtx {
                    ts: ctx.ts,
                    chan: ctx.chan.to_string(),
                    nick: nick.to_owned(),
                    url: url.to_owned(),
                },
            )
            .await?
        );
    }
    Ok(())
}
// EOF
