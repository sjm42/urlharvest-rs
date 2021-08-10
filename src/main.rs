// main.rs
#![feature(once_cell)]

use chrono::*;
use linemux::MuxedLines;
use log::*;
use regex::Regex;
use rusqlite::{named_params, Connection};
use std::{collections::HashMap, env, error::Error, ffi::*, fs};
use structopt::StructOpt;

#[derive(Debug, Clone, StructOpt)]
pub struct GlobalOptions {
    #[structopt(short, long)]
    pub debug: bool,
    #[structopt(short, long)]
    pub trace: bool,
    #[structopt(long, default_value = "$HOME/irclogs/ircnet")]
    pub irc_log_dir: String,
    #[structopt(long, default_value = "$HOME/urllog/log")]
    pub log_dir: String,
    #[structopt(long, default_value = "$HOME/urllog/data/urllog.db")]
    pub db_file: String,
    #[structopt(long, default_value = "urllog")]
    pub db_table: String,
    #[structopt(long, default_value = r#"^(#\S*)\.log$"#)]
    pub re_log: String,
    #[structopt(long, default_value = r#"^[:\d]+\s+[<\*][%@\~\&\+\s]*([^>\s]+)>?\s+"#)]
    pub re_nick: String,
    #[structopt(
        short,
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
            timestamp integer,
            channel text,
            nick text,
            url text);
            create index {table}_timestamp on {table}(timestamp);
            create index {table}_channel on {table}(channel);
            create index {table}_nick on {table}(nick);
            create unique index {table}_unique on {table}(channel, nick, url);
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
    table: &str,
    ts: i64,
    channel: &str,
    nick: &str,
    url: &str,
) -> Result<(), Box<dyn Error>> {
    let sql = &format!(
        "insert into {} (id, timestamp, channel, nick, url) \
        values (null, :ts, :ch, :ni, :ur)",
        table
    );
    let mut st = c.prepare_cached(sql)?;
    st.execute(named_params! {":ts": ts, ":ch": channel, ":ni": nick, ":ur": url})?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let home = env::var("HOME")?;
    let mut opt = GlobalOptions::from_args();
    opt.irc_log_dir = opt.irc_log_dir.replace("$HOME", &home);
    opt.log_dir = opt.log_dir.replace("$HOME", &home);
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

    let re_log = Regex::new(&opt.re_log)?;
    let re_nick = Regex::new(&opt.re_nick)?;
    let re_url = Regex::new(&opt.re_url)?;

    let mut chans: HashMap<OsString, String> = HashMap::with_capacity(16);
    let mut lines = MuxedLines::new()?;

    debug!("Scanning dir {}", &opt.irc_log_dir);
    for log_f in fs::read_dir(&opt.irc_log_dir)? {
        let log_f = log_f?;
        let log_fn = log_f.file_name().to_string_lossy().into_owned();
        if let Some(chan_match) = re_log.captures(&log_fn) {
            let p = log_f.path().file_name().unwrap().to_os_string();
            let irc_chan = chan_match.get(1).unwrap().as_str();
            chans.insert(p, irc_chan.to_string());
            lines.add_file(log_f.path()).await?;
        }
    }
    debug!("My hash: {:?}", chans);

    while let Ok(Some(line)) = lines.next_line().await {
        let msg = line.line();
        let filename = line.source().file_name().unwrap();
        let chan = match chans.get(filename) {
            Some(c) => c,
            None => {
                error!("Unknown source filename: {:?}", filename);
                error!("Offending msg: {}", msg);
                continue;
            }
        };

        let nick = match re_nick.captures(msg) {
            Some(nick_match) => nick_match[1].to_owned(),
            None =>  "UNKNOWN".into(),
        };
        debug!("{} {}", chan, msg);

        for cap in re_url.captures_iter(msg) {
            let url = &cap[1];
            info!("Detected url: {}", url);
            // This may fail because of the unique index and then we don't care.
            let _ = add_url(&sqc, table, Utc::now().timestamp(), chan, &nick, url);
        }
    }
    sqc.close().unwrap();
    Ok(())
}
// EOF
