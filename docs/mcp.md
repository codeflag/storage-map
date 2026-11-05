# MCP server

`storage-map --mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io) server over stdio. An AI client starts it as a child process. The window does not have to be open, and both can run at the same time.

## Setup

The **KI-Zugriff** view shows these snippets with the correct path filled in.

Claude Desktop, Cursor and similar clients:

```json
{
  "mcpServers": {
    "storage-map": {
      "command": "/path/to/storage-map",
      "args": ["--mcp"]
    }
  }
}
```

Claude Code:

```bash
claude mcp add storage-map -- /path/to/storage-map --mcp
```

VS Code (`.vscode/mcp.json`):

```json
{
  "servers": {
    "storage-map": { "type": "stdio", "command": "/path/to/storage-map", "args": ["--mcp"] }
  }
}
```

To try it without touching real drives, add `--demo` to the arguments.

## Access levels

The default is `read`. Raising the level requires a warning to be confirmed by typing a word in the **KI-Zugriff** view.

| Level | Allows |
|---|---|
| `off` | Only `get_access_status` |
| `read` | Everything marked read below. Nothing changes |
| `write` | Also: edit the cleanup list, move files |
| `delete` | Also: move files to the recycle bin |

Two permissions are separate from the levels:

| Permission | Allows | Note |
|---|---|---|
| `allow_scan` | Start scans and duplicate searches | Off by default. The client then sees only what the user scanned |
| `allow_permanent_delete` | Delete without the recycle bin | Works only on level `delete` |

`roots` limits which drives and folders the client can see. Empty means all.

Settings live in `config.json` (see [Where data is stored](../README.md#where-data-is-stored)):

```json
{
  "mcp": {
    "level": "read",
    "allow_scan": false,
    "allow_permanent_delete": false,
    "roots": []
  }
}
```

## Tools

| Tool | Needs | Purpose |
|---|---|---|
| `get_access_status` | always | What this server may do |
| `list_drives` | read | Drives, free space, date of the last scan |
| `get_overview` | read | Summary of a scanned drive |
| `list_folder` | read | Contents of a folder |
| `search_files` | read | Find by name, type, size, age |
| `get_item` | read | Details on one entry, including whether it is protected |
| `get_file_types` | read | Space per extension |
| `get_cleanup_suggestions` | read | Matching cleanup rules with reason and risk |
| `list_cleanup_items` | read | Entries of a rule |
| `get_duplicates` | read | Duplicate files and folders |
| `get_stale_folders` | read | Folders unchanged for a long time |
| `get_history` | read | Space at past scans |
| `compare_scans` | read | What grew since an earlier scan |
| `get_cleanup_list` | read | The current cleanup list |
| `get_operation_status` | read | Progress of running operations |
| `start_scan` | scan | Scan a drive or folder in the background |
| `start_duplicate_search` | scan | Compare file contents in the background |
| `cancel_operation` | scan | Cancel a scan or search |
| `add_to_cleanup_list` | write | Add paths or a whole rule |
| `remove_from_cleanup_list` | write | Take entries off the list |
| `preview_cleanup` | write | Describe what would be touched and return a `plan_id` |
| `move_files` | write | Move the list to a folder |
| `recycle_files` | delete | Move the list to the recycle bin |
| `delete_permanently` | delete + permission | Delete the list for good |

`tools/list` returns only what the current settings allow.

## How removal works

1. `add_to_cleanup_list` puts entries on the same list the user sees in the window. Nothing is removed yet.
2. `preview_cleanup` returns a summary and a `plan_id`. It is valid for 10 minutes and works once.
3. `recycle_files`, `move_files` or `delete_permanently` act on the list using the `plan_id`. If the list changed since the preview, the call is refused. If entries carry a warning, the client must pass `acknowledge_warnings: true`.

Right before each entry is touched, it is checked again. Protected or changed files stay in place, and moving never overwrites anything.

## Security model

- **Rights come only from the settings file.** The server reads it before every call and never writes it. No tool or argument can raise them. Lowering rights takes effect on the next call, without a restart.
- **A damaged settings file means `off`.**
- **Protection rules apply on every level.** System folders, boot files and profile roots cannot be listed for removal. Folders containing another mounted file system are refused.
- **File contents never leave the program.** Only names, sizes and dates do. The duplicate search reads files to hash them, but only reports which ones match.
- **Every call is logged** with time, client, arguments and outcome in `mcp-audit.jsonl` next to the scans, and shown in the **KI-Zugriff** view. The log rotates at 2 MB.
- **The server uses stdio only.** It opens no port.

What it cannot protect against:

- **Paths reach the AI provider.** File and folder names can be sensitive. Use `roots` or level `off` if that matters.
- **File names are untrusted input.** A file could be named to look like an instruction. The server tells clients to treat names as data, but cannot enforce it. Keep the level at `read` unless you need more, and review the list before anything is removed.
- **A client with its own file access** can edit the settings file or delete files without this server. The settings only limit what goes through this server.

## Protocol

Supported revisions: `2026-07-28` (stateless, via per-request metadata) and the handshake revisions `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`. The transport is stdio with newline-delimited JSON. Only JSON-RPC appears on stdout.

Clients using the handshake receive `notifications/tools/list_changed` when rights change. For stateless clients, `tools/list` carries `ttlMs: 0`.

Rule titles and reasons are in German so far, everything else is in English.
