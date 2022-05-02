// startup.rs

use log::*;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::{env, fs::File, io::BufReader, net::SocketAddr};
use structopt::StructOpt;

use super::*;

pub const TABLE_URL: &str = "urllog";
pub const TABLE_META: &str = "urlmeta";

#[derive(Debug, Clone, StructOpt)]
pub struct OptsCommon {
    #[structopt(short, long)]
    pub verbose: bool,
    #[structopt(short, long)]
    pub debug: bool,
    #[structopt(short, long)]
    pub trace: bool,

    #[structopt(short, long, default_value = "$HOME/urllog/cfg/urlharvest.json")]
    pub config_file: String,
    #[structopt(short, long)]
    pub read_history: bool,
    #[structopt(short, long)]
    pub meta_backlog: bool,
}
impl OptsCommon {
    pub fn finish(&mut self) -> anyhow::Result<()> {
        self.config_file = shellexpand::full(&self.config_file)?.into_owned();
        Ok(())
    }
    pub fn get_loglevel(&self) -> LevelFilter {
        if self.trace {
            LevelFilter::Trace
        } else if self.debug {
            LevelFilter::Debug
        } else if self.verbose {
            LevelFilter::Info
        } else {
            LevelFilter::Error
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigCommon {
    pub irc_log_dir: String,
    pub db_file: String,
    pub template_dir: String,
    pub html_dir: String,
    pub regex_log: String,
    pub regex_nick: String,
    pub regex_url: String,
    pub search_listen: SocketAddr,
}
impl ConfigCommon {
    pub fn new(opts: &OptsCommon) -> anyhow::Result<Self> {
        let mut config: ConfigCommon =
            serde_json::from_reader(BufReader::new(File::open(&opts.config_file)?))?;
        config.db_file = shellexpand::full(&config.db_file)?.into_owned();
        config.irc_log_dir = shellexpand::full(&config.irc_log_dir)?.into_owned();
        config.template_dir = shellexpand::full(&config.template_dir)?.into_owned();
        config.html_dir = shellexpand::full(&config.html_dir)?.into_owned();
        Ok(config)
    }
}

pub fn start_pgm(c: &OptsCommon, desc: &str) {
    env_logger::Builder::new()
        .filter_level(c.get_loglevel())
        .format_timestamp_secs()
        .init();
    info!("Starting up {desc}...");
    debug!("Git branch: {}", env!("GIT_BRANCH"));
    debug!("Git commit: {}", env!("GIT_COMMIT"));
    debug!("Source timestamp: {}", env!("SOURCE_TIMESTAMP"));
    debug!("Compiler version: {}", env!("RUSTC_VERSION"));
}

pub fn start_db(c: &ConfigCommon) -> anyhow::Result<DbCtx> {
    let dbc = Connection::open(&c.db_file)?;
    let table_url = TABLE_URL;
    let table_meta = TABLE_META;
    let db = DbCtx {
        dbc,
        table_url,
        table_meta,
        update_change: false,
    };
    db_init(&db)?;
    Ok(db)
}
// EOF
