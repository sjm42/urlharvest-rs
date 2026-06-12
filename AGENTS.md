# Repository Guidelines

## Project Structure & Module Organization

This repository contains one Rust crate, `urlharvest`, for harvesting IRC log URLs, storing them in PostgreSQL, and generating/searching HTML output.

- `src/lib.rs` and `src/*.rs` hold shared utilities for config, database access, hashing, strings, and web helpers.
- `src/bin/` contains the runnable tools: `irssi_urlharvest`, `urllog_meta`, `urllog_generator`, `urllog_actions`, and `migrate_db`.
- `migrations/` contains SQLx database migrations applied at startup.
- `templates/` contains Tera (`.tera`) and Handlebars (`.hbs`) HTML templates.
- `config/urlharvest.json` is the example runtime configuration.

## Build, Test, and Development Commands

- `cargo build` builds all binaries in debug mode.
- `cargo build --release` builds optimized deployment binaries.
- `cargo test` runs the Rust test suite.
- `cargo fmt --check` verifies formatting without changing files.
- `cargo clippy --all-targets --all-features` runs Rust lints across library and binaries.
- `cargo outdated --root-deps-only` checks for direct dependency updates.
- `cargo run --bin urllog_actions -- -c config/urlharvest.json` runs the search server with the sample config.
- `./install.sh` installs release binaries under `$HOME/urlharvest/bin/`.

PostgreSQL is required for runtime behavior. The schema is managed by SQLx migrations in `migrations/`.

## Coding Style & Naming Conventions

Use Rust 2024 on the stable toolchain defined in `rust-toolchain.toml`. Follow `rustfmt.toml`: 120-column width, crate-granular imports, and grouped standard/external/crate imports. Use idiomatic Rust names: `snake_case` for functions, variables, and modules; `CamelCase` for types; `SCREAMING_SNAKE_CASE` for constants.

Keep shared logic in `src/*.rs`; put binary-specific orchestration in `src/bin/<name>.rs`. Prefer `anyhow::Result` for application-level errors and typed errors where callers need to branch on failure.

## Testing Guidelines

There are currently no dedicated test files. Add focused unit tests near the code under `#[cfg(test)] mod tests` and use descriptive names such as `parses_irssi_log_line_with_prefix`. For database behavior, prefer integration-style tests that create isolated test data and document required `DATABASE_URL` setup. Always run `cargo test` before submitting behavioral changes.

## Commit & Pull Request Guidelines

Recent history uses short, direct commit subjects such as `cargo update`. Keep subjects concise and imperative, e.g. `add url parser tests` or `fix search result escaping`.

Pull requests should include a clear description, commands run (`cargo test`, `cargo clippy`, etc.), configuration or migration notes, and linked issues when applicable. Include screenshots only for visible template or search UI changes.

## Security & Configuration Tips

Do not commit real IRC logs, production database URLs, or generated HTML output. Keep `config/urlharvest.json` as an example and pass local secrets through a private config file with `-c <path>`.
