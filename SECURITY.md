# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately through [GitHub's vulnerability reporting](https://github.com/codeflag/storage-map/security/advisories/new), not in a public issue. Include the version, the system and the steps to reproduce.

You can expect a first response within two weeks. Only the latest release receives fixes.

## What counts

Storage Map can remove files, so these are treated as vulnerabilities:

- A way to remove or move a protected path, or a file that changed since the scan.
- A website reaching the local server, or a request working without the token.
- An MCP client gaining more rights than the settings allow.
- File contents reaching an MCP client.
- A crafted file name causing code execution, for example through the user interface or a CSV export.

The limits of the MCP security model are described in [docs/mcp.md](docs/mcp.md#security-model). A client with its own access to the file system is outside of what this program can control.
