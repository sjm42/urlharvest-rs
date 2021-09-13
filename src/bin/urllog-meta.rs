// urllog-meta.rs

use log::*;
use rusqlite::Connection;
use std::env;
use std::error::Error;
use structopt::StructOpt;

use urlharvest::*;

#[derive(Debug, Clone, StructOpt)]
pub struct GlobalOptions {
    #[structopt(short, long)]
    pub debug: bool,
    #[structopt(short, long)]
    pub trace: bool,
    #[structopt(short, long)]
    pub backlog: bool,
    #[structopt(long, default_value = "$HOME/urllog/data/urllog2.db")]
    pub db_file: String,
    #[structopt(long, default_value = "urllog2")]
    pub table_url: String,
    #[structopt(long, default_value = "urlmeta")]
    pub table_meta: String,
}

#[allow(unreachable_code)]
fn main() -> Result<(), Box<dyn Error>> {
    let home = env::var("HOME")?;
    let mut opt = GlobalOptions::from_args();
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
    info!("Starting up URL metadata updater");
    debug!("Git branch: {}", env!("GIT_BRANCH"));
    debug!("Git commit: {}", env!("GIT_COMMIT"));
    debug!("Source timestamp: {}", env!("SOURCE_TIMESTAMP"));
    debug!("Compiler version: {}", env!("RUSTC_VERSION"));
    debug!("Global config: {:?}", opt);

    let dbc = &Connection::open(&opt.db_file)?;
    let table_url = &opt.table_url;
    let table_meta = &opt.table_meta;
    let db = DbCtx {
        dbc,
        table_url,
        table_meta,
        update_change: true,
    };
    db_init(&db)?;

    if opt.backlog {
        process_backlog(&db)
    } else {
        process_live(&db)
    }
}
// EOF
