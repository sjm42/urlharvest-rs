# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
cargo build                  # debug build
cargo build --release        # release build (fat LTO, opt-level 3)
cargo fmt                    # format code (max_width=120, see rustfmt.toml)
cargo clippy                 # lint
```

No test suite exists. The project uses Rust stable toolchain (edition 2024).

Install release binaries to `$HOME/urlharvest/bin/` with `./install.sh`.

## Architecture

URL harvester for IRC that tails irssi log files on disk (no IRC bot). Extracts URLs via regex, stores them in PostgreSQL, fetches page metadata, generates static HTML pages, and serves a search UI.

### Binaries (`src/bin/`)

- **irssi_urlharvest** — Core daemon. Tails irssi log files using `linemux`, extracts URLs via configurable regexes, inserts into PostgreSQL. Supports `--read-history` to backfill from existing logs.
- **urllog_meta** — Polls DB for URLs missing metadata, fetches page title/language/description via `reqwest` + `webpage` crate. Runs in live (polling) or `--meta-backlog` mode.
- **urllog_generator** — Polls DB for changes, renders static HTML pages from Tera templates (`.tera` files in template dir). Generates per-channel and unique-URL views.
- **urllog_actions** — Axum HTTP server providing search UI (Handlebars templates `.hbs`) with endpoints: `/` (index), `/search`, `/remove_url`, `/remove_meta`.
- **migrate_db** — One-time SQLite-to-PostgreSQL migration tool.

### Shared Library (`src/lib.rs`)

Re-exports common std/crate types as a prelude. All binaries `use urlharvest::*`.

- **config.rs** — CLI args (`OptsCommon` via clap derive) and JSON config (`ConfigCommon`). Config file defaults to `$HOME/urlharvest/config/urlharvest.json`.
- **db_util.rs** — PostgreSQL connection (`DbCtx`), schema auto-migration via `sqlx::migrate!()`, URL/meta insert functions with retry logic.
- **str_util.rs** — Extension traits on `&str`/`i64` for HTML escaping, timestamp formatting, SQL search wildcards, whitespace collapsing.
- **web_util.rs** — HTTP client for fetching URL bodies (reqwest with rustls, 5s connect / 10s request timeouts).
- **hash_util.rs** — Wildcard HashMap lookup (`get_wild`) used for per-template timezone resolution.

### Database

PostgreSQL with three tables (see `migrations/`): `url` (id, seen, channel, nick, url), `url_meta` (url_id, lang, title, descr), `url_changed` (change tracking timestamp). Schema is auto-applied via sqlx migrations on startup.

### Configuration

JSON config at `$HOME/urlharvest/config/urlharvest.json` defines: irc log directory, database URL, template paths, regexes for log/nick/URL parsing, search server listen address, URL blacklist. Paths support `$HOME` shell expansion.

### Templates

Two template engines: **Tera** (`.tera`) for static HTML page generation by `urllog_generator`, **Handlebars** (`.hbs`) for search UI by `urllog_actions`. All templates live in the configured `template_dir`.