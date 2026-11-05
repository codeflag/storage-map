# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.1.0] - 2026-11-05

First public version.

### Added

- Scan of drives and folders on Windows and Linux, reading names, sizes and dates only.
- Treemap, sunburst, folder tree, lists of large files and file types, and an overview with charts.
- Cleanup rules with a reason and risk level for each, for Windows and Linux.
- Duplicate search by content, duplicate folders by structure, old and unused files.
- History across scans with a forecast and a comparison of what grew.
- Cleanup list with recycle bin, moving and permanent deletion, protected by fixed rules and a check for changes right before removal.
- MCP server (`--mcp`) with access levels off, read, write and delete, a separate permission for starting scans and for permanent deletion, and a log of all calls.
- Demo and sandbox modes for trying the program without touching real drives.
- English and German interface. It follows the browser language, and a switch in the header changes it.

### Known limitations

- Windows builds are not code-signed, so SmartScreen may warn on first start.
- On Linux, the window, the file manager integration and drive health have not been tested on a real desktop yet. Scan, cleanup, trash and the MCP server are covered by automated tests.

[Unreleased]: https://github.com/codeflag/storage-map/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/codeflag/storage-map/releases/tag/v0.1.0
