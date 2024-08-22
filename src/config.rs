// config.rs

use std::{env, fs::File, io::BufReader, net::SocketAddr};
use std::collections::HashMap;
use anyhow::bail;
use clap::Parser;
use serde::{Deserialize, Serialize};
use chrono_tz::Tz;

use crate::*;

#[derive(Debug, Clone, Parser)]
pub struct OptsCommon {
    #[arg(short, long)]
    pub verbose: bool,
    #[arg(short, long)]
    pub debug: bool,
    #[arg(short, long)]
    pub trace: bool,

    #[arg(short, long, default_value = "$HOME/urlharvest/config/urlharvest.json")]
    pub config_file: String,
    #[arg(short, long)]
    pub read_history: bool,
    #[arg(short, long)]
    pub meta_backlog: bool,
}

impl OptsCommon {
    pub fn finalize(&mut self) -> anyhow::Result<()> {
        self.config_file = shellexpand::full(&self.config_file)?.into_owned();
        Ok(())
    }

    pub fn get_loglevel(&self) -> Level {
        if self.trace {
            Level::TRACE
        } else if self.debug {
            Level::DEBUG
        } else if self.verbose {
            Level::INFO
        } else {
            Level::ERROR
        }
    }


    pub fn start_pgm(&self, name: &str) {
        tracing_subscriber::fmt()
            .with_max_level(self.get_loglevel())
            .with_target(false)
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
    pub db_url: String,
    pub template_dir: String,
    pub template_timezone: HashMap<String, String>,
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

    #[serde(skip)]
    pub template_tz: Option<HashMap<String, Tz>>,

}


impl ConfigCommon {
    pub fn new(opts: &OptsCommon) -> anyhow::Result<Self> {
        debug!("Reading config file {}", &opts.config_file);
        let mut config: ConfigCommon =
            serde_json::from_reader(BufReader::new(File::open(&opts.config_file)?))?;
        config.irc_log_dir = shellexpand::full(&config.irc_log_dir)?.into_owned();
        config.template_dir = shellexpand::full(&config.template_dir)?.into_owned();
        config.html_dir = shellexpand::full(&config.html_dir)?.into_owned();

        let mut template_tz = HashMap::new();
        // Parse the timezone strings
        for (k, v) in &config.template_timezone {
            match v.as_str().parse::<Tz>() {
                Ok(tz) => {
                    template_tz.insert(k.to_string(), tz);
                }
                Err(e) => {
                    bail!("error parsing url_dup_timezone \"{k}\": \"{v}\" - {e}");
                }
            }
        }
        config.template_tz = Some(template_tz);

        Ok(config)
    }
}
// EOF
