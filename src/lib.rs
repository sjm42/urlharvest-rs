// lib.rs

pub use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    env, ffi, fmt, fs,
    io::{self, BufRead},
    net::{self, IpAddr, Ipv4Addr, Ipv6Addr},
    path,
    sync::Arc,
    time,
};

pub use anyhow::{Context, anyhow, bail};
pub use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
pub use chrono_tz::Tz;
pub use clap::Parser;
pub use futures::TryStreamExt;
pub use itertools::Itertools;
pub use regex::Regex;
pub use serde::{Deserialize, Serialize};
pub use tokio::{
    net::lookup_host,
    time::{Duration, sleep, timeout},
};
pub use tracing::{Level, debug, error, info, trace, warn};
pub use tracing_subscriber::fmt as tracing_fmt;

#[cfg(test)]
pub use std::{future::pending, io::Write as IoWrite};
#[cfg(test)]
pub use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    spawn,
    task::JoinHandle,
};

pub use crate::{config::*, db_util::*, hash_util::*, html_meta::*, str_util::*, web_util::*};

pub mod config;
pub mod db_util;
pub mod hash_util;
pub mod html_meta;
pub mod str_util;
pub mod web_util;

// EOF
