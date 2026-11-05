# Contributing

Bug reports and pull requests are welcome. For larger changes, please open an issue first to discuss the idea.

## Setup

Install Rust with [rustup](https://rustup.rs). The toolchain version in `rust-toolchain.toml` is picked up automatically. There are no other dependencies. On Windows you need the MSVC build tools.

```bash
cargo run -- --demo --ui-dir ui
```

With `--ui-dir ui`, the interface loads from disk, so reloading the window is enough after changing it.

## Testing

Tests must never touch real drives or the user's files.

- Use `--demo` for invented data and `--sandbox` with a folder created by the test.
- Unit tests use data from `index::testutil::build`, or a temp folder they create.
- Path logic is testable on any system: Linux rules run on Windows and the other way round, because the path style selects the rules.

Before opening a pull request:

```bash
cargo fmt --check
```

```bash
cargo clippy --all-targets -- -D warnings
```

```bash
cargo test
```

The test `recycle_moves_file_away_and_leaves_others` puts a file into the real recycle bin and is therefore ignored by default. CI runs it on its disposable machines.

## Layout

| Path | Purpose |
|---|---|
| `src/platform/` | Everything that depends on the operating system |
| `src/paths.rs` | Path rules for both systems |
| `src/scan.rs` | Parallel scan, hard link detection, totals |
| `src/index.rs` | Compact data model, saving and loading |
| `src/analysis.rs` | Evaluations for dashboard, treemap and lists |
| `src/junk.rs` | Cleanup rules |
| `src/dupes.rs` | Duplicate files and folders |
| `src/history.rs` | History and comparison between scans |
| `src/guard.rs` | Protection rules |
| `src/cleanup.rs` | Cleanup list and removal |
| `src/server.rs` | Local server and run modes |
| `src/mcp.rs` | MCP server |
| `src/config.rs` | Settings |
| `src/demo.rs` | Invented data and test folders |
| `src/i18n.rs` | Texts of the backend in both languages |
| `ui/` | Interface without dependencies: HTML, CSS, Canvas |
| `ui/js/i18n.js` | Dictionary of the interface |
| `tests/` | Tests of the finished program from the outside |

## Guidelines

- No dependency without a good reason. The interface has none at all.
- Names of files and folders are untrusted data. In the interface they only reach the DOM through `textContent`.
- New removal paths must go through `guard` and `cleanup::precheck`.
- Interface text is in English and German. New text goes into `ui/js/i18n.js` or, for messages from the backend, into a `Text` with both languages. A test checks that every key used in `ui/js` exists.
- Comments are partly in German. New comments may be in English.
- Note user-visible changes in `CHANGELOG.md` under `[Unreleased]`.

By contributing, you agree that your contribution is licensed under the MIT license.
