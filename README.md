# Armarium

Your cabinet of tables — view, edit, and query CSV, TSV, and Excel files with SQL.

Armarium is a Windows desktop application. You register tabular files in a workspace. Then you can
show them, edit cells, save them in their original format, and aggregate them with SQL (SQLite).
AI agents can use the same functions through an MCP server (`armarium-mcp`).

- Specifications: [docs/specs/](docs/specs/README.md)
- Design decisions: [docs/adrs/](docs/adrs/README.md)

## Repository

| Path | Contents |
|---|---|
| `crates/armarium-core` | Domain logic: workspace, file reading, the SQLite engine, editing and undo, saving, and SQL |
| `crates/armarium-mcp` | The MCP server (stdio) |
| `src-tauri` | The desktop application (Tauri 2) |
| `ui` | The frontend (React, TypeScript, Vite) |

## Development

Requirements: Rust (stable, 1.89 or later) and Node.js 22. On Windows, the WebView2 runtime.
On Linux, the Tauri system packages (for example, `libwebkit2gtk-4.1-dev`).

```sh
# Frontend dependencies (one time)
npm ci --prefix ui

# Tests. `cargo test` also writes the TypeScript types to ui/src/bindings.
cargo test --workspace
npm --prefix ui test
npm --prefix ui run lint

# Run the application in development mode (from the repository root)
ui/node_modules/.bin/tauri dev

# Build the installer (Windows). The MCP server is included as a sidecar.
cargo build --release -p armarium-mcp
mkdir -p src-tauri/binaries
cp target/release/armarium-mcp.exe src-tauri/binaries/armarium-mcp-x86_64-pc-windows-msvc.exe
ui/node_modules/.bin/tauri build --config src-tauri/tauri.bundle.json
```

Performance check (spike M0-8). The example makes a CSV file with 1 million rows if necessary:

```sh
cargo run --release -p armarium-core --example load_bench -- 1000000
```

### Environment variables

| Variable | Meaning |
|---|---|
| `ARMARIUM_CONFIG_DIR` | Directory of `workspace.json` and `ui-state.json` (default: `%APPDATA%\Armarium`) |
| `ARMARIUM_DATA_DIR` | Directory of the session database and the logs (default: `%LOCALAPPDATA%\Armarium`) |
| `ARMARIUM_LOG` | Log filter, for example `debug` |
| `ARMARIUM_PROFILE` | If set, the core prints the time of each phase of a file read to stderr |

## MCP server

```json
{
  "mcpServers": {
    "armarium": {
      "command": "C:\\Program Files\\Armarium\\armarium-mcp.exe",
      "args": ["--allow-dir", "C:\\Users\\me\\data"]
    }
  }
}
```

Options: `--workspace <path>`, `--read-only`, `--allow-dir <path>` (more than one is allowed),
`--query-timeout <seconds>`, and `--max-rows <n>`. See [08. MCP server](docs/specs/08-mcp-server.md).

## License

MIT. See [LICENSE](LICENSE).
