# 01. Overview

- Status: Draft
- Last updated: 2026-10-11

## 1. Purpose

We make a desktop application. The user registers many tabular files (CSV, TSV, and Excel) in one workspace. Then the user can:

1. Open and show a file quickly.
2. Edit the cells and save the file in its original format.
3. Aggregate and transform data across files with standard SQL.
4. Give the same functions to AI agents through an MCP server.

## 2. Users and use cases

| Use case | Example |
|---|---|
| Look at data | Open a received CSV file in Shift_JIS without broken characters, and look at its contents. |
| Make small corrections | Correct values in some rows. Save the file with the same delimiter, encoding, and line ending. |
| Aggregate | Join many CSV files and Excel sheets, and aggregate them with GROUP BY (about 1 second for 1 million rows). |
| Change many rows | Update all rows that match a condition with `UPDATE ... SET status = 'done' WHERE ...`. |
| Work with AI | An agent such as Claude examines the files through MCP and returns aggregated results. |

## 3. Scope

### 3.1 In scope (goals)

- File formats: CSV, TSV (and text files with any delimiter), and Excel (.xlsx and .xlsm; .xls is read-only)
- A UI with two panes (left: source tree; right: grid and SQL)
- Register and unregister sources (Armarium does not delete the files)
- Show the sheets of an Excel workbook in the tree
- Edit cells; add and delete rows; add, delete, and rename columns
- Undo and redo
- Aggregate and transform data with standard SQL (SELECT and the DML statements INSERT, UPDATE, and DELETE)
- Export query results, and save query results as a new source
- MCP server (stdio)

### 3.2 Out of scope (at least for the first release)

- Calculate or edit Excel formulas (Armarium shows only the cached values)
- Edit Excel formatting, charts, or pivot tables
- Co-editing, and direct connection to cloud storage
- Connection to databases (for example, PostgreSQL)
- Official support for macOS and Linux (Armarium can possibly run there, but we do not test it)

## 4. Terms

| Term | Meaning |
|---|---|
| Workspace | The set of registered sources and their settings. The application has one workspace. |
| Source | One registered file: a CSV file, a TSV file, or an Excel workbook. |
| Table | The unit that the user shows, edits, and queries with SQL. A CSV or TSV source has one table. In an Excel workbook, each sheet is one table. |
| Working copy | The data that Armarium reads from a file and keeps in SQLite for editing. The file does not change until the user saves. |
| Dirty | A working copy is dirty when it has changes that are not saved. |
| SQL name | The identifier that SQL uses to refer to a table (for example, `sales` or `budget_Sheet1`). |
| Dialect | The format of a CSV file: delimiter, quote character, line ending, encoding, BOM, and header row. |

## 5. Non-functional requirements

| Item | Target |
|---|---|
| Operating system | Windows 10 (22H2) and Windows 11, x64. The WebView2 runtime is necessary. |
| Start time | 2 seconds or less (cold start, 50 registered sources) |
| Read performance | Show a CSV file of 100 MB and 1 million rows in 5 seconds or less (reference machine: 8 cores, 16 GB, SSD) |
| Scrolling | Scroll at approximately 60 fps, also with 1 million rows |
| Memory | Armarium keeps working copies in a session database (a temporary file). Data that does not fit in the cache goes to the disk. Less than 1 GB for 1 million rows. |
| Data integrity | When Armarium saves a file, each cell that the user did not edit has exactly the same text as before ([ADR-0007](../adrs/0007-round-trip-safe-typing.md)). |
| Safe save | Armarium writes to a temporary file and then replaces the original file (atomic replacement). Armarium shows a warning when another program changed the file. |
| Encoding | Read and write UTF-8 (with or without BOM), Shift_JIS (CP932), EUC-JP, and UTF-16 LE/BE. |
| Localization | The UI is available in Japanese and English. |
| Security | SQL cannot access the file system or the network ([ADR-0009](../adrs/0009-sql-sandbox.md)). |

## 6. Technology stack (summary)

| Layer | Technology | ADR |
|---|---|---|
| Application platform | Tauri 2 and Rust | [ADR-0002](../adrs/0002-tauri2-and-rust.md) |
| Data storage and queries | SQLite (`rusqlite` crate, bundled) | [ADR-0003](../adrs/0003-sqlite-as-engine.md) |
| Frontend | React, TypeScript, Vite, CodeMirror 6. Grid: Glide Data Grid or RevoGrid (we decide after a spike) | [ADR-0005](../adrs/0005-frontend-stack.md) |
| Excel | calamine (read), rust_xlsxwriter and umya-spreadsheet (write) | [ADR-0011](../adrs/0011-excel-read-write.md) |
| MCP | rmcp (official Rust SDK), stdio | [ADR-0013](../adrs/0013-mcp-server-mode.md) |
