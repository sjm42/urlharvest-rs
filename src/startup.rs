// startup.rs

use log::*;
use rusqlite::Connection;
use std::{env, error::Error, net::SocketAddr};
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
    pub fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        expand_home(&mut self.db_file)?;
        Ok(())
    }
    fn get_loglevel(&self) -> LevelFilter {
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
    pub fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        self.c.finish()?;
        expand_home(&mut self.template_dir)?;
        expand_home(&mut self.html_dir)?;
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
    pub fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        self.c.finish()?;
        expand_home(&mut self.irc_log_dir)?;
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
    pub fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        self.c.finish()?;
        Ok(())
    }
}

#[derive(Debug, Clone, StructOpt)]
pub struct OptsSearch {
    #[structopt(flatten)]
    pub c: OptsCommon,
    #[structopt(short, long, default_value = "127.0.0.1:8080")]
    pub listen: String,
}
impl OptsSearch {
    pub fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        self.c.finish()?;
        let _ = self.listen.parse::<SocketAddr>()?;
        Ok(())
    }
}

pub fn expand_home(pathname: &mut String) -> Result<(), Box<dyn Error>> {
    let home = env::var("HOME")?;
    *pathname = pathname.replace("$HOME", &home);
    Ok(())
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

pub fn start_db(c: &OptsCommon) -> Result<DbCtx, Box<dyn Error>> {
    let dbc = Connection::open(&c.db_file)?;
    let table_url = &c.table_url;
    let table_meta = &c.table_meta;
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
