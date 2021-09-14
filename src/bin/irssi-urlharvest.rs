// irssi-urlharvest.rs

use chrono::*;
use linemux::MuxedLines;
use log::*;
use regex::Regex;
use std::ffi::*;
use std::fs::{self, DirEntry, File};
use std::io::{BufRead, BufReader};
use std::{collections::HashMap, error::Error, time::Instant};
use structopt::StructOpt;

use urlharvest::*;

const TX_SZ: usize = 1024;
const VEC_SZ: usize = 64;

struct IrcCtx<'a> {
    re_nick: &'a Regex,
    re_url: &'a Regex,
    ts: i64,
    chan: &'a str,
    msg: &'a str,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut opts = OptsHarvest::from_args();
    opts.finish()?;
    start_pgm(&opts.c, "URL harvester");
    let mut db = start_db(&opts.c)?;

    let re_nick = &Regex::new(&opts.re_nick)?;
    let re_url = &Regex::new(&opts.re_url)?;

    let re_log = Regex::new(&opts.re_log)?;
    let mut chans: HashMap<OsString, String> = HashMap::with_capacity(VEC_SZ);
    let mut log_files: Vec<DirEntry> = Vec::with_capacity(VEC_SZ);

    debug!("Scanning dir {}", &opts.irc_log_dir);
    for log_f in fs::read_dir(&opts.irc_log_dir)? {
        let log_f = log_f?;
        let log_fn = log_f.file_name().to_string_lossy().into_owned();
        if let Some(chan_match) = re_log.captures(&log_fn) {
            let irc_chan = &chan_match[1];
            let log_nopath = log_f.path().file_name().unwrap().to_os_string();
            log_files.push(log_f);
            chans.insert(log_nopath, irc_chan.to_string());
        }
    }
    debug!("My logfiles: {:?}", log_files);
    debug!("My chans: {:?}", chans);

    let mut lmux = MuxedLines::new()?;
    let chan_unk = &"<UNKNOWN>".to_string();
    if opts.read_history {
        let mut tx_i: usize = 0;
        db.dbc.execute_batch("begin")?;

        // Save the start time to measure elapsed
        let start_ts = Instant::now();
        // Seed the database with all the old log lines too
        info!("Reading history...");
        // "--- Log opened Sun Aug 08 13:37:42 2021"
        let re_ts = Regex::new(r#"^--- Log opened \w+ (\w+) (\d+) (\d+):(\d+):(\d+) (\d+)"#)?;
        // "--- Day changed Fri Aug 13 2021"
        let re_daychange = Regex::new(r#"^--- Day changed \w+ (\w+) (\d+) (\d+)"#)?;
        // "13:37 <@sjm> 1337"
        let re_hourmin = Regex::new(r#"^(\d\d):(\d\d)\s"#)?;

        for log_f in &log_files {
            let log_nopath = log_f.path().file_name().unwrap().to_os_string();
            let chan = chans.get(&log_nopath).unwrap_or(chan_unk);
            let reader = BufReader::new(File::open(log_f.path())?);
            let mut current_ts = Local::now();
            for (_index, line) in reader.lines().enumerate() {
                let msg = &line?;
                tx_i += 1;
                if tx_i >= TX_SZ {
                    db.dbc.execute_batch("commit")?;
                    db.dbc.execute_batch("begin")?;
                    tx_i = 0;
                }
                if let Some(re_match) = re_hourmin.captures(msg) {
                    let hh = &re_match[1];
                    let mm = &re_match[2];
                    let s = format!("{}{}00", hh, mm);
                    if let Ok(new_localtime) = NaiveTime::parse_from_str(&s, "%H%M%S") {
                        current_ts = current_ts.date().and_time(new_localtime).unwrap();
                    }
                } else if let Some(re_match) = re_daychange.captures(msg) {
                    let mon = &re_match[1];
                    let day = &re_match[2];
                    let year = &re_match[3];
                    let s = format!("{}{}{}", year, mon, day);
                    if let Ok(naive_date) = NaiveDate::parse_from_str(&s, "%Y%b%d") {
                        let naive_ts = naive_date.and_hms(0, 0, 0);
                        if let LocalResult::Single(new_ts) = Local.from_local_datetime(&naive_ts) {
                            trace!("Found daychange {:?}", new_ts);
                            current_ts = new_ts;
                        }
                    }
                } else if let Some(re_match) = re_ts.captures(msg) {
                    let mon = &re_match[1];
                    let day = &re_match[2];
                    let hh = &re_match[3];
                    let mm = &re_match[4];
                    let ss = &re_match[5];
                    let year = &re_match[6];
                    let s = format!("{}{}{}-{}{}{}", year, mon, day, hh, mm, ss);
                    if let Ok(naive_ts) = NaiveDateTime::parse_from_str(&s, "%Y%b%d-%H%M%S") {
                        if let LocalResult::Single(new_ts) = Local.from_local_datetime(&naive_ts) {
                            trace!("Found TS {:?}", new_ts);
                            current_ts = new_ts;
                        }
                    }
                }

                let ctx = IrcCtx {
                    re_nick,
                    re_url,
                    ts: current_ts.timestamp(),
                    chan,
                    msg,
                };
                handle_ircmsg(&db, &ctx)?;
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
    while let Ok(Some(msg_line)) = lmux.next_line().await {
        let filename = msg_line
            .source()
            .file_name()
            .unwrap_or_else(|| OsStr::new("NONE"));
        let chan = chans.get(filename).unwrap_or(chan_unk);
        let msg = msg_line.line();
        let ctx = IrcCtx {
            re_nick,
            re_url,
            ts: Utc::now().timestamp(),
            chan,
            msg,
        };
        db.update_change = true;
        handle_ircmsg(&db, &ctx)?;
    }
    Ok(())
}

fn handle_ircmsg(db: &DbCtx, ctx: &IrcCtx) -> Result<(), Box<dyn Error>> {
    let nick = match ctx.re_nick.captures(ctx.msg) {
        Some(nick_match) => nick_match[1].to_owned(),
        None => "UNKNOWN".into(),
    };
    trace!("{} {}", ctx.chan, ctx.msg);

    for url_cap in ctx.re_url.captures_iter(ctx.msg) {
        let url = &url_cap[1];
        info!("Detected url: {} {} {}", ctx.chan, &nick, url);
        let u = UrlCtx {
            ts: ctx.ts,
            chan: ctx.chan,
            nick: &nick,
            url,
        };
        db_add_url(db, &u)?;
    }
    Ok(())
}
// EOF
