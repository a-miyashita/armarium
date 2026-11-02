# 09. Roadmap and implementation plan

- Status: Draft
- Last updated: 2026-11-02

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
| M7 | Scratch sources | "Save as new source" is replaced by "Save as scratch". The user can save a SELECT result in one click, find it in the "Scratch" group, edit it, export it, and move it to a normal file with "Save as". |

M7 depends on M3 (results and export) and M2 (save). It does not depend on M5 or M6, and the order of M6 and M7 is free.

## 2. Tasks

### M0 Foundation

| ID | Task | Depends on |
|---|---|---|
| M0-1 | Create the Cargo workspace (`armarium-core`, `armarium-mcp`, `src-tauri`). | - |
| M0-2 | Create React + TS + Vite in `ui/` and connect it to Tauri. Generate TypeScript types with `ts-rs`. | M0-1 |
| M0-3 | Add `rusqlite` (features `bundled`, `session`, `hooks`, `functions`). Make sure that the SQLite version is 3.42 or later. | M0-1 |
| M0-4 | GitHub Actions. CI for pull requests and `main`: `fmt`, `clippy`, and `cargo test` on `windows-latest`, frontend lint and test on `ubuntu-latest`. A release workflow builds the installer for a tag `v*` or a manual run. | M0-1, M0-2 |
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

### M7 Scratch sources

For the behavior, see [04. Workspace](04-workspace.md), section 6.

| ID | Task | Depends on |
|---|---|---|
| M7-1 | `workspace`: Add `workspaceId`, `nextScratchNumber`, and `scratch` to the schema (all optional, version stays 1). Keep unknown fields. Take the next number under the workspace lock. Provide the path of the scratch directory (`%LOCALAPPDATA%\Armarium\scratch\<workspaceId>\`). | M1-2 |
| M7-2 | `engine`: Replace `save_result_as_source` with `save_result_as_scratch`: take a number, write the result to a temporary file and replace it, register the source with `scratch: true`, delete the temporary file on cancel or error. Reuse the CSV export of M3-7. | M7-1, M3-7 |
| M7-3 | `engine`: Automatic save of scratch tables (2 seconds after the last change, on tab close, on exit). On failure, treat the table as dirty. Stop the automatic save after `EXTERNAL_MODIFICATION`. | M7-2, M2-4 |
| M7-4 | `engine`: "Save as" for a scratch source. Reuse `save_table_as` (CSV and TSV only), then clear `scratch` and delete the old file. Ignore a delete error. | M7-2 |
| M7-5 | `workspace`: Unregistration of a scratch source deletes its file (also for the MCP `unregister_source`). Disable "Change file location" for it. | M7-1 |
| M7-6 | Cleanup at GUI start: delete unregistered `scratch_NN.csv` files and unfinished temporary files in the scratch directory. | M7-1 |
| M7-7 | Tauri commands and `ui/src/api.ts`: add the scratch command, remove `save_result_as_source` and its UI code. | M7-2 |
| M7-8 | UI: "Save as scratch" button and message with "Open". "Scratch" group in the tree. Context menu: Save as…, Export…, Delete scratch. Delete dialog. No `●` and no exit-dialog entry for scratch tables. Texts in English and Japanese (`ui/src/i18n.ts`). | M7-7, M7-3, M7-4 |
| M7-9 | MCP: add `scratch` to the `list_sources` output. `unregister_source` follows M7-5. | M7-5, M5-2 |
| M7-10 | Tests: number is never used again (delete, restart), cancel leaves no file, automatic save and its failure, "Save as" moves the data and keeps the SQL name, delete removes the file, cleanup, a query on `scratch_01` after the save. | M7-2 to M7-6 |

## 3. Distribution

- Make an NSIS installer (per-user installation) and an MSI with the Tauri bundler.
- Install WebView2 with the bootstrapper (Windows 11 includes it).
- Code signing: not done for now. The installers are unsigned (ADR-0016).
- Automatic update: we will consider `tauri-plugin-updater` in M6 or later.

## 4. Open questions

| # | Question | When to decide |
|---|---|---|
| Q1 | Undo method (session extension or triggers) | **Decided**: session extension (ADR-0006, amendment 2026-10-11) |
| Q2 | Is more than one workspace necessary? | At the end of M2 |
| Q3 | Do we also give the difference between NULL and an empty string for CSV? | **Decided for now**: no. An empty field is NULL. The display setting can show NULL. |
| Q4 | Do we allow temporary views (`CREATE TEMP VIEW`) for work in a query tab? | **Decided for now**: no (ADR-0009). |
| Q5 | Check the license notices (SQLite: public domain; calamine, umya-spreadsheet, ts-rs: MIT; license of the selected grid) and generate a NOTICE file. | M0 |
| Q6 | Method of code signing | **Decided for now**: no signing. Distribute an unsigned MSI and `cargo install` (ADR-0016). |

## 5. Implementation status (2026-10-11)

| Milestone | Status | Notes |
|---|---|---|
| M0 | Done | Spikes M0-5, M0-7, and M0-8 are tests or examples. M0-6: Glide Data Grid (IME on Windows is not checked yet). CI runs on `windows-latest`. |
| M1 | Done | `notify` monitors the parent directories. Test data is in the tests; `examples/load_bench.rs` makes large files. |
| M2 | Done | The round-trip test (M2-7) covers 15 files (encodings, delimiters, quotes, line endings). |
| M3 | Done | SELECT, DML, `RETURNING`, results, export, history, cancel. |
| M4 | Done | Sheets in the tree, formula cells are read-only, "Save as" to XLSX or CSV. |
| M5 | Done except M5-4 | All tools, resources, and the prompt. M5-4 (tests with MCP Inspector and Claude Desktop) is manual. The installer includes `armarium-mcp` as a sidecar (CI). |
| M6 | Not started | |
| M7 | Done | M7-1 to M7-10 are implemented and tested in `armarium-core` and `armarium-mcp`. The UI passes the type check. The maintainer checked the screens by hand (2026-11-02). |

Known limits of the first implementation:

- The core error messages are in English. The UI translates the titles of the dialogs.
- Files are decoded in memory (approximately three times the file size).
- Code signing (Q6, unsigned for now) and the NOTICE file (Q5) are not done.

