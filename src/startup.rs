// startup.rs

use log::*;
use rusqlite::Connection;
use std::{env, net::SocketAddr};
use structopt::StructOpt;

use super::*;

#[derive(Debug, Clone, StructOpt)]
pub struct OptsCommon {
    #[structopt(short, long)]
    pub debug: bool,
    #[structopt(short, long)]
    pub trace: bool,
    #[structopt(long, default_value = "$HOME/urllog/data/urllog2.db")]
    pub db_file: String,
    #[structopt(long, default_value = "urllog2")]
    pub table_url: String,
    #[structopt(long, default_value = "urlmeta")]
    pub table_meta: String,
}
impl OptsCommon {
    pub fn finish(&mut self) -> anyhow::Result<()> {
        self.db_file = shellexpand::full(&self.db_file)?.into_owned();
        Ok(())
    }
    pub fn get_loglevel(&self) -> LevelFilter {
        if self.trace {
            LevelFilter::Trace
        } else if self.debug {
            LevelFilter::Debug
        } else {
            LevelFilter::Info
        }
    }
}

#[derive(Debug, Clone, StructOpt)]
pub struct OptsGenerator {
    #[structopt(flatten)]
    pub c: OptsCommon,
    #[structopt(long, default_value = "$HOME/urllog/templates2")]
    pub template_dir: String,
    #[structopt(long, default_value = "$HOME/urllog/html2")]
    pub html_dir: String,
}
impl OptsGenerator {
    pub fn finish(&mut self) -> anyhow::Result<()> {
        self.c.finish()?;
        self.template_dir = shellexpand::full(&self.template_dir)?.into_owned();
        self.html_dir = shellexpand::full(&self.html_dir)?.into_owned();
        Ok(())
    }
}

#[derive(Debug, Clone, StructOpt)]
pub struct OptsHarvest {
    #[structopt(flatten)]
    pub c: OptsCommon,
    #[structopt(short, long)]
    pub read_history: bool,
    #[structopt(long, default_value = "$HOME/irclogs/ircnet")]
    pub irc_log_dir: String,
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
impl OptsHarvest {
    pub fn finish(&mut self) -> anyhow::Result<()> {
        self.c.finish()?;
        self.irc_log_dir = shellexpand::full(&self.irc_log_dir)?.into_owned();
        Ok(())
    }
}

#[derive(Debug, Clone, StructOpt)]
pub struct OptsMeta {
    #[structopt(flatten)]
    pub c: OptsCommon,
    #[structopt(short, long)]
    pub backlog: bool,
}
impl OptsMeta {
    pub fn finish(&mut self) -> anyhow::Result<()> {
        self.c.finish()
    }
}

#[derive(Debug, Clone, StructOpt)]
pub struct OptsSearch {
    #[structopt(flatten)]
    pub c: OptsCommon,
    #[structopt(short, long, default_value = "127.0.0.1:8080")]
    pub listen: SocketAddr,
}
impl OptsSearch {
    pub fn finish(&mut self) -> anyhow::Result<()> {
        self.c.finish()?;
        Ok(())
    }
}

pub fn start_pgm(c: &OptsCommon, desc: &str) {
    env_logger::Builder::new()
        .filter_level(c.get_loglevel())
        .format_timestamp_secs()
        .init();
    info!("Starting up {}...", desc);
    debug!("Git branch: {}", env!("GIT_BRANCH"));
    debug!("Git commit: {}", env!("GIT_COMMIT"));
    debug!("Source timestamp: {}", env!("SOURCE_TIMESTAMP"));
    debug!("Compiler version: {}", env!("RUSTC_VERSION"));
}

pub fn start_db(c: &OptsCommon) -> anyhow::Result<DbCtx> {
    let dbc = Connection::open(&c.db_file)?;
    let table_url = c.table_url.as_str();
    let table_meta = c.table_meta.as_str();
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
