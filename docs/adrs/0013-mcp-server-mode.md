# ADR-0013: Give the MCP server as a separate stdio binary

- Status: Accepted
- Date: 2026-10-11

## Context

The final goal is that Armarium is an MCP server, so that AI agents can read, aggregate, and transform the registered files.

- The main MCP clients (Claude Desktop, Claude Code, and others) support local servers that they start through stdio.
- Agents must be able to use the server when the GUI is not running.
- But in some cases, we also want to show the agent data that the user is editing in the GUI and did not save.

## Decision

1. **M5: Standalone stdio server.** We give `armarium-mcp.exe`.
   - It uses `rmcp`, the official Rust SDK.
   - It uses `armarium-core` and has its own SQLite session database. It shares only the workspace settings (`workspace.json`) and the files with the GUI.
   - Edits change the working copies in the MCP process. `save_table` saves them to the files.
   - stdout is only for the MCP protocol. Logs go to stderr and to a log file.
2. **M6 or later: GUI-attached mode** (`--attach`). We will consider it.
   - The server connects to the running GUI through a named pipe (`\\.\pipe\armarium-{user}`) and operates on the working copies of the GUI.
   - In the GUI, the user can select whether MCP can write.
   - We will write a separate ADR at implementation time.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| Start an HTTP server (Streamable HTTP) in the GUI process | Can share data that is not saved, but the GUI must run. We must design a local port and authentication. |
| A `--mcp` flag on the GUI executable | Only one file to distribute. It needs a branch that prevents the Tauri initialization, and the dependencies are large ([ADR-0004](0004-cargo-workspace-layout.md)). |
| MCP only reads files (no writes) | Safe, but the "transform" requirement fails. We give this as the `--read-only` option. |

## Consequences

### Good

- The server works without the GUI. The standard configuration of an MCP client is sufficient.
- The GUI and the MCP server are different processes. A crash of one process does not affect the other.
- They share the core, so the SQL behavior and the sandbox are the same as in the GUI.

### Bad

- The MCP server cannot see changes in the GUI that are not saved. If both edit the same file, the one that saves later detects an external change. (This prevents data loss, but it takes extra work.)
- The two processes each read the same file into memory, so they use more memory.
