# 09. Roadmap and implementation plan

- Status: Draft
- Last updated: 2026-10-11

## 1. Milestones

```
M0 Foundation ──▶ M1 Viewer ──▶ M2 Edit and save ──▶ M3 SQL ──▶ M5 MCP ──▶ M6 Extensions
                      │                                  ▲
                      └──────▶ M4 Excel read ────────────┘
```

| Milestone | Goal | Completion criteria |
|---|---|---|
| M0 | Foundation | The Cargo workspace exists. The Tauri 2 application starts. `rusqlite` (bundled) works. CI (Windows) passes. We finalize the ADRs from the spike results. |
| M1 | Viewer | The user can register and unregister CSV and TSV files, open them from the tree, and scroll through 1 million rows. |
| M2 | Edit and save | Cell edits, row and column operations, undo and redo, save with the original dialect, detection of external changes |
| M3 | SQL | Run SELECT and DML statements, show and export results, sandbox |
| M4 | Excel read | Sheets in the tree, read sheets, save as (new XLSX or CSV) |
| M5 | MCP | `armarium-mcp` (stdio) gives the tools in [08. MCP server](08-mcp-server.md). |
| M6 | Extensions | Overwrite Excel files, GUI-attached MCP mode, automatic backup, Parquet support, and others |

## 2. Tasks

### M0 Foundation

| ID | Task | Depends on |
|---|---|---|
| M0-1 | Create the Cargo workspace (`armarium-core`, `armarium-mcp`, `src-tauri`). | - |
| M0-2 | Create React + TS + Vite in `ui/` and connect it to Tauri. Generate TypeScript types with `ts-rs`. | M0-1 |
| M0-3 | Add `rusqlite` (features `bundled`, `session`, `hooks`, `functions`). Make sure that the SQLite version is 3.42 or later. | M0-1 |
| M0-4 | GitHub Actions on `windows-latest`: `cargo test`, `clippy`, `fmt`, frontend lint and test, Tauri build. | M0-1, M0-2 |
| M0-5 | **Spike**: Record a changeset for a `rowid` table with the session extension (`SQLITE_SESSION_OBJCONFIG_ROWID`), and undo by applying the inverted changeset. If `rusqlite` has no API for this, call it through FFI or change to the trigger method ([ADR-0006](../adrs/0006-working-copy-edit-model.md)). | M0-3 |
| M0-6 | **Spike**: Compare Glide Data Grid (6.0.4 alpha, React 19) and RevoGrid. Criteria: scrolling with page fetches for 1 million rows, range selection, copy and paste, Japanese input (IME). Select one grid and update [ADR-0005](../adrs/0005-frontend-stack.md). | M0-2 |
| M0-7 | **Spike**: Make sure that the authorizer allow list allows DML statements and updates of internal tables through internal triggers. Make sure that it refuses DDL, `ATTACH`, `VACUUM`, updates of `rowid`, and references to internal tables ([ADR-0009](../adrs/0009-sql-sandbox.md)). | M0-3 |
| M0-8 | **Spike**: Measure the time to read a CSV file of 100 MB and 1 million rows through "temporary table → type inference → STRICT table" ([ADR-0008](../adrs/0008-ingestion-in-rust.md)). | M0-3 |

### M1 Viewer

| ID | Task | Depends on |
|---|---|---|
| M1-1 | `model`: Define the shared types. | M0-1 |
| M1-2 | `workspace`: Register and unregister, assign SQL names, read and write `workspace.json`. | M1-1 |
| M1-3 | `ingest`: Detect and convert the encoding (`chardetng`, `encoding_rs`). | M1-1 |
| M1-4 | `ingest`: Detect the CSV dialect, parse, and insert into the temporary table (all columns TEXT). | M1-3 |
| M1-5 | `ingest`: Round-trip-safe type inference, create the STRICT table, create the order table and triggers ([ADR-0006](../adrs/0006-working-copy-edit-model.md), [ADR-0007](../adrs/0007-round-trip-safe-typing.md)). | M1-4 |
| M1-6 | `engine`: Session database, write worker and read connection, page fetch (`fetch_rows`). | M0-3 |
| M1-7 | Tauri commands: `list_sources`, `add_sources`, `remove_source`, `open_table`, `fetch_rows`. | M1-2, M1-6 |
| M1-8 | UI: Layout with two panes, source tree, registration by drag and drop. | M0-2, M1-7 |
| M1-9 | UI: Data grid (read-only), status bar. | M1-7, M0-6 |
| M1-10 | File monitoring (`notify`) and reload. | M1-2 |
| M1-11 | Test data set (encodings, delimiters, incorrect rows, a script that makes very large files). | - |

### M2 Edit and save

| ID | Task | Depends on |
|---|---|---|
| M2-1 | `edit`: Apply `EditOp` (cell, row, and column operations). | M1-5 |
| M2-2 | `edit`: Undo and redo stacks with changesets, dirty state. | M2-1, M0-5 |
| M2-3 | `persist`: Write in the original dialect, atomic replacement, detection of encoding errors. | M1-4 |
| M2-4 | `persist`: Fingerprint and detection of external changes. | M2-3 |
| M2-5 | UI: Cell edit, row and column operations, copy and paste (TSV), keyboard operations. | M2-1 |
| M2-6 | UI: Dialogs for save, external changes, and exit. | M2-4 |
| M2-7 | Round-trip test: for all test data, a file after "open → save" is byte-for-byte the same as the original. | M2-3 |

### M3 SQL

| ID | Task | Depends on |
|---|---|---|
| M3-1 | `engine`: Authorizer and safe connection settings. | M1-6, M0-7 |
| M3-2 | `sql`: Prepare and run each statement, detect `SQL_FORBIDDEN`. | M3-1 |
| M3-3 | `sql`: Load tables that are not loaded (tokens, and retry after `no such table`). | M3-2, M1-2 |
| M3-4 | `sql`: Run DML, record the changeset, ask the user when the changeset is larger than the limit. | M3-2, M2-2 |
| M3-4b | `sql`: User-defined functions (`regexp`, `regexp_replace`, `normalize_nfkc`). | M3-2 |
| M3-5 | `sql`: Result tables, page fetch, cancel. | M3-2 |
| M3-6 | UI: SQL editor (CodeMirror 6), completion, result grid, history. | M3-5 |
| M3-7 | Export (CSV, TSV, XLSX). | M3-5 |
| M3-8 | Regression tests for the sandbox (refused statements and file access functions). | M3-2 |

### M4 Excel read

| ID | Task | Depends on |
|---|---|---|
| M4-1 | `ingest`: List sheets and read them with calamine, map the types. | M1-5 |
| M4-2 | SQL names for Excel (workbook name and sheet name). | M1-2 |
| M4-3 | UI: Sheets in the tree, hidden sheets, read-only display of formula cells. | M4-1 |
| M4-4 | Save as (rust_xlsxwriter). | M4-1, M3-7 |

### M5 MCP

| ID | Task | Depends on |
|---|---|---|
| M5-1 | `armarium-mcp`: stdio server with rmcp, start options. | M3 |
| M5-2 | Implement the tools (`list_sources` to `export_result`). | M5-1 |
| M5-3 | Resources and prompts. | M5-1 |
| M5-4 | Integration tests with MCP Inspector and Claude Desktop. | M5-2 |
| M5-5 | Include `armarium-mcp.exe` in the installer. | M5-1 |

## 3. Distribution

- Make an NSIS installer (per-user installation) and an MSI with the Tauri bundler.
- Install WebView2 with the bootstrapper (Windows 11 includes it).
- Code signing: after we get a signing certificate (not decided).
- Automatic update: we will consider `tauri-plugin-updater` in M6 or later.

## 4. Open questions

| # | Question | When to decide |
|---|---|---|
| Q1 | Undo method (session extension or triggers) | After the result of M0-5 |
| Q2 | Is more than one workspace necessary? | At the end of M2 |
| Q3 | Do we also give the difference between NULL and an empty string for CSV? | M2 |
| Q4 | Do we allow temporary views (`CREATE TEMP VIEW`) for work in a query tab? | M3 |
| Q5 | Check the license notices (SQLite: public domain; calamine, umya-spreadsheet, ts-rs: MIT; license of the selected grid) and generate a NOTICE file. | M0 |
| Q6 | Method of code signing | Before the first release |
