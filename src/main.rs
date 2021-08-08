// main.rs
#![feature(once_cell)]

use log::*;
use regex::Regex;
use structopt::StructOpt;
use std::{env, fs, io, error::Error, lazy::*};
use tokio::sync::RwLock;

// TODO: sqlite handling, IRC log tailing with linemux

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
    pub log_re: String,
    #[structopt(short, long, default_value = r#"([a-z][a-z0-9\.\*\-]+)://([\w\-\.\,\~\:\;\/\?\#\[\]\@\!\$\&\'\(\)\*\+\%\=]+)"#)]
    pub url_re: String,
    #[structopt(long, default_value = r#"^[\:\d]+\s+[<\*][\~\&\@\%\+\s]*([^>\s]+)>?\s+"#)]
    pub nick_re: String,
}

static CFG: SyncLazy<RwLock<GlobalOptions>> =
    SyncLazy::new(|| RwLock::new(GlobalOptions {
        debug: false,
        trace: false,
        irc_log_dir: "".into(),
        log_dir: "".into(),
        db_file: "".into(),
        db_table: "".into(),
        log_re: "".into(),
        url_re: "".into(),
        nick_re: "".into(),
    }));

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

    let log_re = Regex::new(&opt.log_re)?;
    debug!("Scanning dir {}", &opt.irc_log_dir);
    for log_f in fs::read_dir(&opt.irc_log_dir)? {
        let log_f = log_f?;
        let log_path = log_f.path().to_string_lossy().into_owned();
        let log_fn = log_f.file_name().to_string_lossy().into_owned();

        if let Some(chan_match) =  log_re.captures(&log_fn) {
            let irc_chan = match chan_match.get(1) {
                Some(chan) => chan.as_str(),
                None => "<UNKNOWN>",
            };
            debug!("Found channel {} in {}", irc_chan, &log_path);
         }
    }
    Ok(())
}
// EOF
