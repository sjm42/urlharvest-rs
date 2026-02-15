# urlharvest

A URL harvester for IRC that works by tailing [irssi](https://irssi.org/) log files on disk. No IRC bot needed.

URLs are extracted from chat logs via regex, stored in PostgreSQL with channel/nick/timestamp metadata, enriched with fetched page titles, and served as generated HTML pages and a searchable web UI.

## How It Works

The system is a pipeline of four cooperating daemons:

1. **irssi_urlharvest** — Tails irssi log files in real time using [linemux](https://crates.io/crates/linemux). When a URL is detected via regex, it is inserted into PostgreSQL along with the channel, nick, and timestamp. Can also backfill from existing log history with `--read-history`.

2. **urllog_meta** — Polls the database for URLs that lack metadata. Fetches each page and extracts the title, language, and description. Runs continuously in live mode, or processes the entire backlog with `--meta-backlog`.

3. **urllog_generator** — Watches for database changes and regenerates static HTML pages from [Tera](https://keats.github.io/tera/) templates. Produces per-channel and deduplicated URL listings with configurable per-template timezones.

4. **urllog_actions** — An [Axum](https://github.com/tokio-rs/axum) web server that provides a search interface using [Handlebars](https://crates.io/crates/handlebars) templates. Supports searching by channel, nick, URL, and title. Also exposes endpoints for removing URLs and refreshing metadata.

A fifth binary, **migrate_db**, is a one-time tool for migrating data from a legacy SQLite database to PostgreSQL.

## Prerequisites

- Rust stable toolchain (edition 2024)
- PostgreSQL server with a database created (e.g. `createdb url`)
- irssi log files on disk (one `.log` file per channel)

## Building

```bash
cargo build --release
```

Install the release binaries to `$HOME/urlharvest/bin/`:

```bash
./install.sh
```

## Configuration

All binaries read a shared JSON config file, defaulting to `$HOME/urlharvest/config/urlharvest.json`. Override with `-c <path>`.

An example config is provided in `config/urlharvest.json`:

```json
{
    "irc_log_dir": "$HOME/irclogs/ircnet",
    "db_url": "postgres:///url",
    "template_dir": "$HOME/urlharvest/templates",
    "template_timezone": {
        "*": "UTC",
        "url.html": "EET"
    },
    "html_dir": "$HOME/urlharvest/html",
    "regex_log": "^(#\\S*)\\.log$",
    "regex_nick": "^[:\\d]+\\s+[<\\*][%@\\~\\&\\+\\s]*([^>\\s]+)>?\\s+",
    "regex_url": "(https?://...)",
    "search_listen": "127.0.0.1:8080",
    "tpl_search_index": "search_index.html.hbs",
    "tpl_search_result_header": "search_result_header.html.hbs",
    "tpl_search_result_row": "search_result_row.html.hbs",
    "tpl_search_result_footer": "search_result_footer.html.hbs",
    "url_blacklist": []
}
```

| Field | Description |
|---|---|
| `irc_log_dir` | Directory containing irssi channel log files |
| `db_url` | PostgreSQL connection string |
| `template_dir` | Directory with Tera (`.tera`) and Handlebars (`.hbs`) templates |
| `template_timezone` | Per-template timezone overrides; `*` is the default |
| `html_dir` | Output directory for generated static HTML |
| `regex_log` | Regex to match log filenames and extract channel name (capture group 1) |
| `regex_nick` | Regex to extract nickname from a log line (capture group 1) |
| `regex_url` | Regex to extract URLs from a log line (capture group 1) |
| `search_listen` | Address and port for the search web server |
| `tpl_search_*` | Handlebars template filenames for the search UI |
| `url_blacklist` | URL prefixes to ignore |

Paths support shell expansion (e.g. `$HOME`).

## Usage

All binaries share common CLI flags:

```
-v, --verbose       Info-level logging
-d, --debug         Debug-level logging
-t, --trace         Trace-level logging
-c, --config-file   Config file path (default: $HOME/urlharvest/config/urlharvest.json)
```

### Typical deployment

Run all four daemons, for example as systemd services or in tmux:

```bash
# Harvest URLs from irssi logs in real time
irssi_urlharvest

# Or first backfill existing history, then continue live
irssi_urlharvest --read-history

# Fetch page metadata for new URLs
urllog_meta

# Or first process the entire backlog of old URLs
urllog_meta --meta-backlog

# Generate static HTML pages when the DB changes
urllog_generator

# Serve the search UI
urllog_actions
```

## Database

The schema is automatically created/migrated on startup via [sqlx migrations](https://docs.rs/sqlx/latest/sqlx/migrate/index.html). Three tables:

- **url** — Each row is one sighting: `(id, seen, channel, nick, url)`
- **url_meta** — Fetched page metadata: `(url_id, lang, title, descr)`, one-to-one with url
- **url_changed** — Single-row table tracking the last modification timestamp, used by `urllog_meta` and `urllog_generator` to detect changes

## Templates

Two template engines are used:

- **Tera** (`.tera` files) — Used by `urllog_generator` for static HTML pages. Templates receive arrays of URL data with timestamps formatted per the configured timezone.
- **Handlebars** (`.hbs` files) — Used by `urllog_actions` for the search web UI (index page, result header/row/footer).

Custom templates can be added to the template directory. Tera templates are automatically discovered by `urllog_generator`; Handlebars templates are referenced by name in the config.

## License

MIT OR Apache-2.0