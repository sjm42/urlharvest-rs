// main.rs

use chrono::*;
use linemux::MuxedLines;
use log::*;
use regex::Regex;
use rusqlite::{named_params, Connection, Statement};
use std::{
    collections::HashMap,
    env,
    error::Error,
    ffi::*,
    fs::{self, DirEntry, File},
    io::{BufRead, BufReader},
    time::Instant,
};
use structopt::StructOpt;

const TX_SZ: usize = 1024;
const VEC_SZ: usize = 64;

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
    #[structopt(long, default_value = "urllog2")]
    pub db_table: String,
    #[structopt(long, default_value = r#"^(#\S*)\.log$"#)]
    pub re_log: String,
    #[structopt(long, default_value = r#"^[:\d]+\s+[<\*][%@\~\&\+\s]*([^>\s]+)>?\s+"#)]
    pub re_nick: String,
    #[structopt(
        long,
        default_value = r#"(https?://[\w/',":;!%@=\-\.\~\?\#\[\]\{\}\$\&\(\)\*\+]+[^\s'"\)\]\}])"#
    )]
    pub re_url: String,
}

struct MyContext {
    re_nick: Regex,
    re_url: Regex,
}

fn check_table(dbc: &Connection, table: &str) -> Result<(), Box<dyn Error>> {
    let mut st = dbc.prepare(
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
            seen integer,
            channel text,
            nick text,
            url text);
            create index {table}_seen on {table}(seen);
            create index {table}_channel on {table}(channel);
            create index {table}_nick on {table}(nick);
            create index {table}_url on {table}(url);
            commit;",
            table = table
        );
        info!("Creating new DB table+indexes.");
        debug!("SQL:\n{}", &sql);
        dbc.execute_batch(&sql)?;
    }
    Ok(())
}

async fn handle_msg(ctx: &MyContext, st_i: &mut Statement<'_>, ts: i64, chan: &str, msg: &str) {
    let nick = match ctx.re_nick.captures(msg) {
        Some(nick_match) => nick_match[1].to_owned(),
        None => "UNKNOWN".into(),
    };
    trace!("{} {}", chan, msg);

    for url_cap in ctx.re_url.captures_iter(msg) {
        let url = &url_cap[1];
        info!("Detected url: {} {} {}", chan, &nick, url);
        if let Ok(n) = st_i.execute(named_params! {":ts": ts, ":ch": chan, ":ni": nick, ":ur": url})
        {
            debug!("Inserted {} row", n);
        } else {
            error!("Insert failed, WTF?");
        }
    }
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

    let ctx = MyContext {
        re_nick: Regex::new(&opt.re_nick)?,
        re_url: Regex::new(&opt.re_url)?,
    };

    let dbc = Connection::open(&opt.db_file)?;
    let table = &opt.db_table;
    check_table(&dbc, table)?;

    let sql_i = format!(
        "insert into {} (id, seen, channel, nick, url) \
        values (null, :ts, :ch, :ni, :ur)",
        table
    );
    let mut st_i = dbc.prepare(&sql_i)?;

    let re_log = Regex::new(&opt.re_log)?;
    let mut chans: HashMap<OsString, String> = HashMap::with_capacity(VEC_SZ);
    let mut log_files: Vec<DirEntry> = Vec::with_capacity(VEC_SZ);

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

    let mut lmux = MuxedLines::new()?;
    let chan_unk = &"<UNKNOWN>".to_string();
    if opt.read_history {
        let mut tx_i: usize = 0;
        dbc.execute_batch("begin")?;

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
                let msg = line?;
                tx_i += 1;
                if tx_i >= TX_SZ {
                    dbc.execute_batch("commit")?;
                    dbc.execute_batch("begin")?;
                    tx_i = 0;
                }
                if let Some(re_match) = re_hourmin.captures(&msg) {
                    let hh = &re_match[1];
                    let mm = &re_match[2];
                    let s = format!("{}{}00", hh, mm);
                    if let Ok(new_localtime) = NaiveTime::parse_from_str(&s, "%H%M%S") {
                        current_ts = current_ts.date().and_time(new_localtime).unwrap();
                    }
                } else if let Some(re_match) = re_daychange.captures(&msg) {
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
                } else if let Some(re_match) = re_ts.captures(&msg) {
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
                handle_msg(&ctx, &mut st_i, current_ts.timestamp(), chan, &msg).await;
            }
            // OK all history processed, add the file for live processing from now onwards
            lmux.add_file(log_f.path()).await?;
        }
        dbc.execute_batch("commit")?;
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
        handle_msg(&ctx, &mut st_i, Utc::now().timestamp(), chan, msg).await;
    }
    Ok(())
}
// EOF
