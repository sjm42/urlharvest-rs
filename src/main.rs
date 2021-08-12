// main.rs
#![feature(once_cell)]

use chrono::*;
use linemux::MuxedLines;
use log::*;
use regex::Regex;
use rusqlite::{named_params, Connection};
use std::{
    collections::HashMap,
    env,
    error::Error,
    ffi::*,
    fs::{self, DirEntry, File},
    io::{BufRead, BufReader},
    lazy::*,
};
use structopt::StructOpt;
use tokio::sync::RwLock;

#[derive(Debug, Clone, StructOpt)]
pub struct GlobalOptions {
    #[structopt(short, long)]
    pub debug: bool,
    #[structopt(short, long)]
    pub trace: bool,
    #[structopt(short, long)]
    pub read_history: bool,
    #[structopt(long, default_value = "$HOME/irclogs/ircnet")]
    pub irc_log_dir: String,
    #[structopt(long, default_value = "$HOME/urllog/data/urllog2.db")]
    pub db_file: String,
    #[structopt(long, default_value = "urllog")]
    pub db_table: String,
    #[structopt(long, default_value = r#"^(#\S*)\.log$"#)]
    pub re_log: String,
    #[structopt(long, default_value = r#"^[:\d]+\s+[<\*][%@\~\&\+\s]*([^>\s]+)>?\s+"#)]
    pub re_nick: String,
    #[structopt(
        long,
        default_value = r#"(https?://[\w/',":;!%@=\-\.\~\?\#\[\]\$\&\(\)\*\+]+)"#
    )]
    pub re_url: String,
}

fn check_table(c: &Connection, table: &str) -> Result<(), Box<dyn Error>> {
    let mut st = c.prepare(
        "select count(name) from sqlite_master \
        where type='table' and name=?",
    )?;
    let n: i32 = st.query([table])?.next()?.unwrap().get(0)?;
    if n == 1 {
        info!("DB table exists.");
    } else {
        let sql = format!(
            "begin;
            create table {table} (
            id integer primary key autoincrement,
            first_seen integer,
            last_seen integer,
            num_seen integer,
            channel text,
            nick text,
            url text);
            create index {table}_last_seen on {table}(last_seen);
            create index {table}_num_seen on {table}(num_seen);
            create index {table}_channel on {table}(channel);
            create index {table}_nick on {table}(nick);
            create unique index {table}_unique on {table}(channel, url);
            commit;",
            table = table
        );
        info!("Creating new DB table+indexes.");
        debug!("SQL:\n{}", &sql);
        c.execute_batch(&sql)?;
    }
    Ok(())
}

fn add_url(
    c: &Connection,
    ta: &str,
    ts: i64,
    ch: &str,
    ni: &str,
    ur: &str,
) -> Result<(), Box<dyn Error>> {
    let sql_i = &format!(
        "insert into {} (id, first_seen, last_seen, num_seen, channel, nick, url) \
        values (null, :ts, :ts, 1, :ch, :ni, :ur)",
        ta
    );
    let mut st_i = c.prepare_cached(sql_i)?;
    if let Ok(n) = st_i.execute(named_params! {":ts": ts, ":ch": ch, ":ni": ni, ":ur": ur}) {
        debug!("Inserted {} row", n);
        return Ok(());
    }
    // Insert failed, we must already have it. Channel+URL must be unique.
    // Do an update instead.
    let sql_u = &format!(
        "update {} set last_seen=:ts, num_seen=num_seen+1, nick=:ni \
        where channel=:ch and url=:ur",
        ta
    );
    let mut st_u = c.prepare_cached(sql_u)?;
    if let Ok(n) = st_u.execute(named_params! {":ts": ts, ":ch": ch, ":ni": ni, ":ur": ur}) {
        debug!("Updated {} row(s)", n);
        return Ok(());
    }
    Err("Insert AND update failed, WTF?".into())
}

static RE_NICK: SyncLazy<RwLock<Regex>> = SyncLazy::new(|| RwLock::new(Regex::new("").unwrap()));
static RE_URL: SyncLazy<RwLock<Regex>> = SyncLazy::new(|| RwLock::new(Regex::new("").unwrap()));

async fn handle_msg(
    c: &Connection,
    table: &str,
    ts: i64,
    chan: &str,
    msg: &str,
) -> Result<(), Box<dyn Error>> {
    let re_nick = RE_NICK.read().await;
    let re_url = RE_URL.read().await;
    let nick = match re_nick.captures(msg) {
        Some(nick_match) => nick_match[1].to_owned(),
        None => "UNKNOWN".into(),
    };
    trace!("{} {}", chan, msg);

    for url_cap in re_url.captures_iter(msg) {
        let url = &url_cap[1];
        info!("Detected url: {} {} {}", chan, &nick, url);
        let _ = add_url(c, table, ts, chan, &nick, url);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let home = env::var("HOME")?;
    let mut opt = GlobalOptions::from_args();
    opt.irc_log_dir = opt.irc_log_dir.replace("$HOME", &home);
    opt.db_file = opt.db_file.replace("$HOME", &home);
    let loglevel = if opt.trace {
        LevelFilter::Trace
    } else if opt.debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };

    env_logger::Builder::new()
        .filter_level(loglevel)
        .format_timestamp_secs()
        .init();
    info!("Starting up URL harvester...");
    debug!("Git branch: {}", env!("GIT_BRANCH"));
    debug!("Git commit: {}", env!("GIT_COMMIT"));
    debug!("Source timestamp: {}", env!("SOURCE_TIMESTAMP"));
    debug!("Compiler version: {}", env!("RUSTC_VERSION"));
    debug!("Global config: {:?}", opt);

    let sqc = Connection::open(&opt.db_file)?;
    let table = &opt.db_table;
    check_table(&sqc, table)?;

    // pre-compile our regexes
    let re_log = Regex::new(&opt.re_log)?;
    {
        let mut n = RE_NICK.write().await;
        *n = Regex::new(&opt.re_nick)?;
        let mut u = RE_URL.write().await;
        *u = Regex::new(&opt.re_url)?;
    }

    let mut lmux = MuxedLines::new()?;
    let mut chans: HashMap<OsString, String> = HashMap::with_capacity(16);
    let mut log_files: Vec<DirEntry> = Vec::with_capacity(16);

    debug!("Scanning dir {}", &opt.irc_log_dir);
    for log_f in fs::read_dir(&opt.irc_log_dir)? {
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

    let chan_unk = &"<UNKNOWN>".to_string();

    if opt.read_history {
        // Seed the database with all the old log lines too
        info!("Reading history...");
        // "--- Log opened Sun Aug 08 13:37:42 2021"
        let re_timestamp =
            Regex::new(r#"^--- Log opened \w+ (\w+) (\d+) (\d+):(\d+):(\d+) (\d+)"#)?;
        // "--- Day changed Fri Aug 13 2021"
        let re_daychange = Regex::new(r#"^--- Day changed \w+ (\w+) (\d+) (\d+)"#)?;
        // "13:37 <@sjm> 1337"
        let re_hourmin = Regex::new(r#"^(\d\d):(\d\d)"#)?;

        for log_f in &log_files {
            let log_nopath = log_f.path().file_name().unwrap().to_os_string();
            let chan = chans.get(&log_nopath).unwrap_or(chan_unk);
            let reader = BufReader::new(File::open(log_f.path())?);
            let mut current_ts = Local::now();
            for (_index, line) in reader.lines().enumerate() {
                let msg = line?;
                if let Some(re_match) = re_timestamp.captures(&msg) {
                    let mon = &re_match[1];
                    let day = &re_match[2];
                    let hh = &re_match[3];
                    let mm = &re_match[4];
                    let ss = &re_match[5];
                    let year = &re_match[6];
                    let s = format!("{}{}{}-{}{}{}", year, mon, day, hh, mm, ss);
                    let new_local_ts = NaiveDateTime::parse_from_str(&s, "%Y%b%d-%H%M%S")
                        .unwrap_or_else(|_| NaiveDateTime::from_timestamp(0, 0));
                    current_ts = Local.from_local_datetime(&new_local_ts).unwrap();
                    trace!("Found TS {:?}", current_ts);
                }
                if let Some(re_match) = re_daychange.captures(&msg) {
                    let mon = &re_match[1];
                    let day = &re_match[2];
                    let year = &re_match[3];
                    let s = format!("{}{}{}", year, mon, day);

                    let new_localdate = NaiveDate::parse_from_str(&s, "%Y%b%d")
                        .unwrap_or_else(|_| NaiveDate::from_yo(1970, 1))
                        .and_hms(0, 0, 0);
                    current_ts = Local.from_local_datetime(&new_localdate).unwrap();
                    trace!("Found daychange {:?}", current_ts);
                }
                if let Some(re_match) = re_hourmin.captures(&msg) {
                    let hh = &re_match[1];
                    let mm = &re_match[2];
                    let s = format!("{}{}00", hh, mm);
                    let new_localtime = NaiveTime::parse_from_str(&s, "%H%M%S")
                        .unwrap_or_else(|_| NaiveTime::from_hms(0, 0, 0));
                    current_ts = current_ts.date().and_time(new_localtime).unwrap();
                }
                let _ = handle_msg(&sqc, table, current_ts.timestamp(), chan, &msg).await;
            }
            // OK all history processed, add the file for live processing from now onwards
            lmux.add_file(log_f.path()).await?;
        }
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
        let _ = handle_msg(&sqc, table, Utc::now().timestamp(), chan, msg).await;
    }
    sqc.close().unwrap();
    Ok(())
}
// EOF
