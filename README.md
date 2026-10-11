# Armarium

Your cabinet of tables — view, edit, and query CSV, TSV, and Excel files with SQL.

Armarium is a Windows desktop application. You register tabular files in a workspace. Then you can
show them, edit cells, save them in their original format, and aggregate them with SQL (SQLite).
AI agents can use the same functions through an MCP server (`armarium-mcp`).

- Specifications: [docs/specs/](docs/specs/README.md)
- Design decisions: [docs/adrs/](docs/adrs/README.md)

## Install

Windows: download the installer from GitHub Releases.

With Cargo (Windows; Linux is unofficial):

```sh
cargo install armarium        # the desktop application
cargo install armarium-mcp    # the MCP server (put it in the same directory as armarium)
```

Requirements:

- Rust 1.89 or later.
- LLVM (`libclang`). The build needs it, because the `session` feature of `rusqlite` generates the
  SQLite bindings at build time.
  - Windows: `winget install LLVM.LLVM`, then set the environment variable `LIBCLANG_PATH` to the folder
    that has `libclang.dll`, for example `C:\Program Files\LLVM\bin`. Open a new terminal.
  - Linux: `libclang-dev`.
- Windows: the WebView2 runtime and the MSVC build tools.
- Linux: the Tauri system packages (for example, `libwebkit2gtk-4.1-dev`, `libxdo-dev`, and `libssl-dev`).

## Repository

| Path | Contents |
|---|---|
| `crates/armarium-core` | Domain logic: workspace, file reading, the SQLite engine, editing and undo, saving, and SQL |
| `crates/armarium-mcp` | The MCP server (stdio) |
| `src-tauri` | The desktop application (Tauri 2) |
| `ui` | The frontend (React, TypeScript, Vite) |

## Development

Requirements: Rust (stable, 1.89 or later), Node.js 22, and LLVM (`libclang`, see [Install](#install)).
On Windows, the WebView2 runtime. On Linux, the Tauri system packages (for example, `libwebkit2gtk-4.1-dev`).

```sh
# Frontend dependencies (one time)
npm ci --prefix ui

# Tests. `cargo test` also writes the TypeScript types to ui/src/bindings.
cargo test --workspace
npm --prefix ui test
npm --prefix ui run lint

# Build the frontend. The application embeds it from src-tauri/dist, so Rust needs it
# (cargo build, test, and clippy fail without it).
npm --prefix ui run build

# Run the application in development mode (from the repository root).
# --no-default-features loads the dev server instead of the embedded frontend.
ui/node_modules/.bin/tauri dev -- --no-default-features

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

### CI and releases

- `.github/workflows/ci.yml` runs the checks for each pull request and each push to `main`.
  It does not build the installer.
- `.github/workflows/release.yml` builds the installer. Run it by hand from the Actions tab, or push
  a tag such as `v0.1.0`. A tag also creates a draft GitHub Release with the installers, and
  publishes `armarium-core`, `armarium-mcp`, and `armarium` to crates.io after the approval of the
  `release` environment ([ADR-0015](docs/adrs/0015-cargo-install-distribution.md)).
  Before a release, set the same version in `Cargo.toml` (`[workspace.package]` and the `armarium-core`
  dependency) and in `src-tauri/tauri.conf.json`. The workflow checks that the tag matches them.

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
