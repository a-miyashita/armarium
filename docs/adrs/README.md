# ADRs (Architecture Decision Records)

This directory records the design decisions of Armarium. For the format, see [ADR-0001](0001-record-architecture-decisions.md).

| # | Title | Status |
|---|---|---|
| [0001](0001-record-architecture-decisions.md) | Record design decisions in ADRs | Accepted |
| [0002](0002-tauri2-and-rust.md) | Use Tauri 2 and Rust as the application platform | Accepted |
| [0003](0003-sqlite-as-engine.md) | Use SQLite for data storage and queries | Accepted |
| [0004](0004-cargo-workspace-layout.md) | Put the domain logic in a core crate that does not depend on Tauri | Accepted |
| [0005](0005-frontend-stack.md) | Use React and CodeMirror 6 for the frontend, and select the grid with a spike | Accepted |
| [0006](0006-working-copy-edit-model.md) | Edit a working copy in SQLite and save explicitly | Accepted |
| [0007](0007-round-trip-safe-typing.md) | Infer column types that are round-trip safe | Accepted |
| [0008](0008-ingestion-in-rust.md) | Read and write files in Rust | Accepted |
| [0009](0009-sql-sandbox.md) | Limit SQL to queries and DML | Accepted |
| [0010](0010-sql-naming.md) | Make SQL names from file names and sheet names | Accepted |
| [0011](0011-excel-read-write.md) | Read Excel with calamine, and add write support in phases | Accepted |
| [0012](0012-workspace-persistence.md) | Store the workspace in a JSON file | Accepted |
| [0013](0013-mcp-server-mode.md) | Give the MCP server as a separate stdio binary | Accepted |
| [0014](0014-paged-grid-ipc.md) | The grid fetches only the visible rows through IPC | Accepted |
| [0015](0015-cargo-install-distribution.md) | Publish to crates.io so that `cargo install armarium` works | Accepted |

## Status values

- **Proposed**: Under review. After the review, the status changes to Accepted or Rejected.
- **Accepted**: Approved. Do not change the body text. To add information, append an "Amendments" section with the date.
- **Superseded by ADR-XXXX**: A newer ADR replaces this ADR.
- **Rejected**: Not adopted (kept as a record).
