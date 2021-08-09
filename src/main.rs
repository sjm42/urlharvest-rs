// main.rs
#![feature(once_cell)]

use linemux::MuxedLines;
use log::*;
use regex::Regex;
use std::{collections::HashMap, env, error::Error, ffi::*, fs, lazy::*};
use structopt::StructOpt;
use tokio::sync::RwLock;

// TODO: URL matching, sqlite handling

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
        default_value = r#"([a-z][a-z0-9\.\*\-]+)://([\w/',":;!%@=\-\.\~\?\#\[\]\$\&\(\)\*\+]+)"#
    )]
    pub re_url: String,
}

static CFG: SyncLazy<RwLock<GlobalOptions>> = SyncLazy::new(|| {
    RwLock::new(GlobalOptions {
        debug: false,
        trace: false,
        irc_log_dir: "".into(),
        log_dir: "".into(),
        db_file: "".into(),
        db_table: "".into(),
        re_log: "".into(),
        re_url: "".into(),
        re_nick: "".into(),
    })
});

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

    {
        let mut o = CFG.write().await;
        *o = opt.clone();
    }
    debug!("Global config: {:?}", CFG.read().await);

    let re_log = Regex::new(&opt.re_log)?;
    let re_nick = Regex::new(&opt.re_nick)?;
    let re_url = Regex::new(&opt.re_url)?;

    let mut chans: HashMap<OsString, String> = HashMap::with_capacity(16);
    let mut lines = MuxedLines::new()?;

    debug!("Scanning dir {}", &opt.irc_log_dir);
    for log_f in fs::read_dir(&opt.irc_log_dir)? {
        let log_f = log_f?;
        lines.add_file(log_f.path()).await?;
        let log_fn = log_f.file_name().to_string_lossy().into_owned();
        if let Some(chan_match) = re_log.captures(&log_fn) {
            let p = log_f.path().file_name().unwrap().to_os_string();
            let irc_chan = chan_match.get(1).unwrap().as_str();
            chans.insert(p, irc_chan.to_string());
        }
    }
    debug!("My hash: {:?}", chans);

    while let Ok(Some(line)) = lines.next_line().await {
        let chan = chans.get(line.source().file_name().unwrap()).unwrap();
        let msg = line.line();
        debug!("{} -> {}", chan, msg);

        match re_nick.captures(msg) {
            None => debug!("Ignored: {}", msg),
            Some(nick_match) => {
                let nick = nick_match.get(1).unwrap().as_str();
                debug!("Nick: {}", nick);
            }
        }
    }
    Ok(())
}
// EOF
