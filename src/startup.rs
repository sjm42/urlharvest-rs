// startup.rs

use log::*;
use serde::{Deserialize, Serialize};
use std::{env, fs::File, io::BufReader, net::SocketAddr};
use structopt::StructOpt;

// use super::*;

#[derive(Debug, Clone, StructOpt)]
pub struct OptsCommon {
    #[structopt(short, long)]
    pub verbose: bool,
    #[structopt(short, long)]
    pub debug: bool,
    #[structopt(short, long)]
    pub trace: bool,

    #[structopt(short, long, default_value = "$HOME/urlharvest/config/urlharvest.json")]
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
    pub fn start_pgm(&self, name: &str) {
        env_logger::Builder::new()
            // .filter_module(name, self.get_loglevel())
            .filter_level(self.get_loglevel())
            .format_timestamp_secs()
            .init();

        info!("Starting up {name} v{}...", env!("CARGO_PKG_VERSION"));
        debug!("Git branch: {}", env!("GIT_BRANCH"));
        debug!("Git commit: {}", env!("GIT_COMMIT"));
        debug!("Source timestamp: {}", env!("SOURCE_TIMESTAMP"));
        debug!("Compiler version: {}", env!("RUSTC_VERSION"));
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
    pub tpl_search_index: String,
    pub tpl_search_result_header: String,
    pub tpl_search_result_row: String,
    pub tpl_search_result_footer: String,
    pub url_blacklist: Vec<String>,
}
impl ConfigCommon {
    pub fn new(opts: &OptsCommon) -> anyhow::Result<Self> {
        debug!("Reading config file {}", &opts.config_file);
        let mut config: ConfigCommon =
            serde_json::from_reader(BufReader::new(File::open(&opts.config_file)?))?;
        config.db_file = shellexpand::full(&config.db_file)?.into_owned();
        config.irc_log_dir = shellexpand::full(&config.irc_log_dir)?.into_owned();
        config.template_dir = shellexpand::full(&config.template_dir)?.into_owned();
        config.html_dir = shellexpand::full(&config.html_dir)?.into_owned();
        Ok(config)
    }
}
// EOF
