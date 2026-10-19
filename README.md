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

Install the four service binaries to `$HOME/urlharvest/bin/`:

```bash
./install.sh
```

Deploy the templates and browser assets:

```bash
mkdir -p "$HOME/urlharvest/templates" "$HOME/urlharvest/html/static"
rsync -a templates/ "$HOME/urlharvest/templates/"
rsync -a static/ "$HOME/urlharvest/html/static/"
```

The generated pages use relative asset URLs, so the same HTML output directory can be exposed at paths such as
`/url/` and `/url2/`. Deploy the static files before the updated templates; the pages intentionally rely on the
shared stylesheet for their table layout as well as their colors.

## Development Checks

Common standard-library and dependency imports are explicitly re-exported from `src/lib.rs`, following the sibling
`sjmb` convention. Library modules use `use crate::*;` and binaries use `use urlharvest::*;`; specialized imports stay
local to the module that needs them. Library test-only imports are grouped under `#[cfg(test)]` in `src/lib.rs`.

Before submitting changes, run the standard Cargo checks:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test
```

Database tests are opt-in and use temporary tables or an isolated fixture schema. They check transactions, title
selection before and after a metadata refresh, and notification routing. Run them against a disposable PostgreSQL
database with
`URLHARVEST_TEST_DATABASE_URL=postgres:///urlharvest_test cargo test -- --ignored`.

Check direct dependency updates with:

```bash
cargo outdated --root-deps-only
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

`urllog_actions` relies on an authenticating reverse proxy. Keep its listener on loopback or a private interface,
and protect the search UI and both `/remove_url` and `/remove_meta` routes at the proxy. The application provides
no authentication or authorization. Its existing administration actions use GET requests; proxy authentication
alone does not provide CSRF protection against requests from an already authenticated browser. Configure the
proxy/browser access policy accordingly. Removing a URL deletes all sightings of the same URL.

Search accepts Unicode, channel prefixes such as `#`, and URL query punctuation. Each field is limited to 4096
UTF-8 bytes and cannot contain control characters. SQL parameters remain bound separately from the query text.
Each generated or search-result group displays the title with the highest metadata ID among its sightings.
Refreshing a sighting inserts a new metadata row, so its fetched title becomes the group's selected title.
Search applies its title filter after grouping; counts include all sightings with metadata matching the channel,
nick, and URL filters, even when their older titles differ.

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

Metadata fetching is best effort. A shared HTTP client checks HTTPS certificates, accepts only public HTTP/HTTPS
destinations without embedded credentials, and applies the same destination checks to DNS answers and every
redirect (at most five). Environment HTTP proxies are disabled so requests cannot bypass that policy. The overall
fetch deadline is ten seconds. Non-text responses are skipped before reading their bodies; text is limited to
the first 1 MiB of decompressed bytes and 1 MiB after charset decoding. A truncated page can still supply metadata.
HTML title, language, and description extraction uses a tokenizer without building or recursively walking a DOM.
Titles are limited to 400 UTF-8 bytes, language tags to 128, and descriptions to 4096.

Fetch failures are deliberately stored as metadata containing an error title. There are no automatic HTTP retries:
timeouts and server errors do not reliably predict when a site will recover. Use the refresh action to retry manually.
The metadata worker is intended to run as a single process.
Results for URLs removed while fetching are discarded, and an already existing metadata row is kept.

Each daemon uses a shared PostgreSQL pool with at most four connections, a three-second pool acquisition deadline,
a five-second statement timeout, and a one-second lock timeout. URL inserts have an eight-second overall deadline.
Failed inserts and log-reader errors terminate the harvester with an error instead of silently continuing. Inserts
are attempted once because an ambiguous connection failure could otherwise cause duplicate records.

History import uses transactions of up to 1024 log lines. Completed batches remain committed if a later batch fails;
replaying history can duplicate existing sightings and remains the operator's responsibility. Historical dates use
the process's local timezone (set `TZ` to match the original logger). During DST fall-back, ambiguous local times use
the first occurrence because the logs contain no UTC offset. Log files are discovered at startup.
The log harvester skips malformed URLs, non-HTTP/HTTPS URLs, URLs containing whitespace or control characters,
and URLs over 2048 UTF-8 bytes. Valid URL strings are preserved as posted, without normalization or truncation.

The legacy `migrate_db` utility imports `./url.db` in one PostgreSQL transaction and rolls back on an insertion error.
Run it offline without other database writers. It resets the URL ID sequence after copying explicit IDs and also
handles an empty source database.

## Database

The schema is automatically created/migrated on startup via [sqlx migrations](https://docs.rs/sqlx/latest/sqlx/migrate/index.html). The primary tables are:

- **url** — Each row is one sighting: `(id, seen, channel, nick, url)`
- **url_meta** — Fetched page metadata: `(url_id, lang, title, descr)`, one-to-one with url

PostgreSQL triggers publish all URL and metadata changes to `url_db_changed`, which wakes `urllog_meta`.
`urllog_generator` listens on `url_pages_changed`: metadata inserts/updates/deletes and URL updates/deletes wake
the generator, while URL inserts do not. New sightings only enter generated pages once their metadata is stored,
including fetch-error results, so a raw URL insert no longer causes a redundant generation round. URL deletion and
manual metadata refresh continue to update pages. Both workers reconcile against the latest database state on
startup and after listener reconnection. The generator retains a fixed two-second window to batch page-change
notifications; this batches completed writes rather than waiting for a title fetch to finish.

The notification migration is applied automatically at startup by updated binaries. Restart the updated generator
to switch its subscription. Existing harvesters need no changes, and the original notification channel remains
available to older listeners.

To check for legacy metadata rows with null titles:

```sql
SELECT count(*) AS null_title_rows FROM url_meta WHERE title IS NULL;
```

This counts existing metadata rows with a null title; URLs without a metadata row are a separate backlog.

## Templates

Two template engines are used:

- **Tera** (`.tera` files) — Used by `urllog_generator` for static HTML pages. Templates receive arrays of URL data with timestamps formatted per the configured timezone.
- **Handlebars** (`.hbs` files) — Used by `urllog_actions` for the search web UI (index page, result header/row/footer).

Custom templates can be added to the template directory. Tera templates are automatically discovered by `urllog_generator`; Handlebars templates are referenced by name in the config.

URL fields are HTML-escaped when constructing template data, including ampersands and quotes. This preserves literal
entity-like text such as `&copy;` in link destinations and prevents URL text from becoming HTML markup. Stored URLs
are unchanged.

The generated pages and search UI use `static/theme.css` and `static/theme.js`. Their horizontal theme control
follows the browser's light/dark preference by default and stores an explicit Light or Dark override in local
storage. The URL administration and search pages also use `static/url-actions.js` for their shared browser-side
actions. These files are deployment assets and are not rewritten during HTML generation.

## License

MIT OR Apache-2.0
