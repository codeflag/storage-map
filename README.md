# Storage Map

[![CI](https://github.com/codeflag/storage-map/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/codeflag/storage-map/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/codeflag/storage-map)](https://github.com/codeflag/storage-map/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/codeflag/storage-map/total)](https://github.com/codeflag/storage-map/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Platform: Windows | Linux](https://img.shields.io/badge/platform-Windows%20%7C%20Linux-lightgrey)](#download)
[![Rust 1.85+](https://img.shields.io/badge/Rust-1.85%2B-orange)](CONTRIBUTING.md)
[![MCP server](https://img.shields.io/badge/MCP-server-8A2BE2)](docs/mcp.md)
[![Buy Me a Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-FFDD00?logo=buymeacoffee&logoColor=black)](https://buymeacoffee.com/codeflag)

[Deutsche Version](README.de.md)

Storage Map shows what fills a drive and helps you clean it up. It runs on Windows and Linux as a single program and needs no installation.

![Treemap of a drive](docs/img/map.png)

*All screenshots use the built-in demo with invented files.*

The interface is available in English and German. It follows the browser language, and a switch in the header changes it. The MCP interface and the command line are in English.

## Download

Get the latest version from the [releases page](https://github.com/codeflag/storage-map/releases).

| System | File |
|---|---|
| Windows 10/11, x86-64 | `storage-map-VERSION-windows-x86_64.zip` |
| Linux, x86-64 | `storage-map-VERSION-linux-x86_64.tar.gz`, `.deb` or `.rpm` |
| Linux, ARM64 | `storage-map-VERSION-linux-aarch64.tar.gz`, `.deb` or `.rpm` |

On Windows the program is called `StorageMap.exe`, on Linux `storage-map`. Commands in this documentation use the Linux name.

The Linux builds are statically linked and work on any distribution. `SHA256SUMS` lists the checksums of all files.

Windows builds are not code-signed, so SmartScreen may warn on first start. The window needs Edge, Chrome or another Chromium browser. Without one, Storage Map opens in your default browser.

## Start

Start `StorageMap.exe` or `storage-map`, pick a drive and click **Scan**. The scan reads names, sizes and dates. Files are only read during a duplicate search, and nothing is removed until you confirm it.

## Views

![Overview of a drive](docs/img/overview.png)

| View | Purpose |
|---|---|
| **Overview** | Usage, drive health, cleanup potential, largest folders and files, age and size distribution |
| **Storage map** | Treemap: each area is as large as the space it takes. Colored by file type or age, zoom by double-click or mouse wheel |
| **Rings** | The same data as a sunburst, which shows nesting well |
| **Folders** | Expandable tree with size bars. "Follow largest path" leads straight to the main cause |
| **Large files** | Sortable list with filters for type, size, age and search, plus CSV export |
| **File types** | Categories and extensions, one click leads to the files |
| **Cleanup** | Fixed rules with a reason and a risk level: temp files, caches, build folders, old installers, logs and more |
| **Duplicates** | Compares size, then start and end, then full content. Also finds duplicate folders |
| **Old & unused** | Files unchanged for a long time and forgotten folders |
| **History** | Usage across scans, a forecast of when the drive is full, and what grew since the last scan |
| **AI access** | Permissions and log of the MCP server |

Keyboard: `1` to `9` switch views, `Ctrl+K` searches, `Del` puts the selection on the cleanup list, `Backspace` goes up in the treemap. In the search, `*.mp4` filters by extension and a word containing `/` searches the whole path.

## Removing files

- Everything first goes on the **cleanup list**. There you review it and choose: recycle bin, move, or delete permanently.
- Permanent deletion must be confirmed by typing a word.
- System folders, boot files, swap files and profile root folders are locked. The program checks this before every removal, not just in the interface.
- Every file is checked again right before removal. If it changed since the scan, it stays.
- Network drives have no recycle bin. The program says so and offers only moving or permanent deletion.
- Space in the recycle bin becomes free only when the bin is emptied.

## MCP server

`storage-map --mcp` makes the results available to AI clients such as Claude Desktop, Claude Code or Cursor over the [Model Context Protocol](https://modelcontextprotocol.io). The default is read-only. Writing and deleting must be enabled in the **AI access** view, each with a warning that has to be confirmed.

File and folder names go to the AI client and usually on to its provider. File contents never do. See [docs/mcp.md](docs/mcp.md) for setup, tools and the security model.

![Permission settings](docs/img/mcp.png)

## Command line

```bash
storage-map --help
```

`storage-map --scan PATH` scans without a window and adds the result to the history. Run from the Windows Task Scheduler or cron, it builds up a history on its own. The **History** view shows the matching command.

## Where data is stored

| | Windows | Linux |
|---|---|---|
| Scans and history | `%LOCALAPPDATA%\StorageMap` | `~/.local/share/storage-map` |
| Settings | `%LOCALAPPDATA%\StorageMap\config.json` | `~/.config/storage-map/config.json` |

Storage Map makes no network connections except to itself on `127.0.0.1`. The local server requires a random token, so other websites cannot reach it. For each drive, the last scan and up to 40 compact folder lists are kept. Deleting the folder only loses the history and saved scans.

## Differences between Windows and Linux

| | Windows | Linux |
|---|---|---|
| Scan | One drive letter, including junctions as links | One file system. Other mounted file systems are shown but not entered |
| Recycle bin | Windows recycle bin | [freedesktop trash](https://specifications.freedesktop.org/trash-spec/latest/), as used by GNOME, KDE and others |
| More access | Restart as administrator from the program | Start with `sudo` |
| Drive health | Windows storage API | Model from `/sys`, health from `smartctl` if installed and permitted |
| "Created" date | Always available | Not available in the static builds |
| "Last opened" date | Unreliable by default on Windows, the program says so | Depends on mount options, unavailable with `noatime` |

## Trying it without real drives

```bash
storage-map --demo
```

Invented drives held in memory. No user file is read and nothing is deleted. A banner points this out. `--demo-scale 14` does the same with over a million files.

```bash
storage-map --make-fixture /tmp/fixture
storage-map --sandbox /tmp/fixture
```

Creates a test folder with known contents (about 80 MB) and exercises the real scan, hashing and removal. Everything outside the folder is refused, and the recycle bin and file manager are simulated.

## Building

Requires Rust. The version is pinned in `rust-toolchain.toml`, which `rustup` follows automatically.

```bash
cargo build --release
```

```bash
cargo test
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for how the code is organized and how to test it.

## Support

Storage Map is free and open source. If it got a few gigabytes back for you, I'd be happy about a coffee. That's entirely up to you: a bug report or a kind word makes my day just as much. Thanks for trying it!

<a href="https://buymeacoffee.com/codeflag"><img src="docs/img/buymeacoffee-qr.png" alt="QR code for buymeacoffee.com/codeflag" width="160"></a>

[buymeacoffee.com/codeflag](https://buymeacoffee.com/codeflag)

## License

[MIT](LICENSE). Licenses of the libraries used are listed in `THIRD_PARTY_LICENSES.md` in each release.
