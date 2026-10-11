# ADR-0003: Use SQLite for data storage and queries

- Status: Accepted
- Date: 2026-10-11

## Context

- Armarium must keep the data that the user shows and edits (the working copies), and get it in pages.
- Edits in the grid (updates of one cell or one row) occur frequently.
- The user wants to aggregate and transform data across many files with standard SQL (SELECT and DML).
- Both the user and AI agents (through MCP) write SQL. Thus, we want to control in detail which operations SQL can do.
- The user must be able to undo edits and DML statements.
- The expected data size is up to approximately 1 million rows for each file.

## Decision

- Use **SQLite** through the `rusqlite` crate (`bundled` feature). SQLite keeps the working copies of all tables.
- Put the database in a temporary file for each session (`%LOCALAPPDATA%\Armarium\tmp\session-{pid}.db`).
  - Set `journal_mode = WAL`, `synchronous = OFF`, and a large `cache_size`. While the data fits in the cache, SQLite works in memory. Data that does not fit goes to the file.
  - With WAL, a read connection can fetch pages while a write occurs.
  - Delete the file when Armarium stops. At start, delete old files that remain after a crash.
- Build the design on these SQLite features:
  - The hidden `rowid` is the row ID ([ADR-0006](0006-working-copy-edit-model.md)).
  - `STRICT` tables enforce the column types ([ADR-0007](0007-round-trip-safe-typing.md)).
  - The authorizer callback limits SQL ([ADR-0009](0009-sql-sandbox.md)).
  - The session extension records changesets for undo and redo ([ADR-0006](0006-working-copy-edit-model.md)).
- If SQLite does not have a necessary function (for example, regular expressions), add it as a user-defined function in Rust.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| DuckDB | Column-oriented. Aggregation is 10 to 100 times faster, and the SQL has more extensions. But views are not updatable and `rowid` is not stable, so an edit model with hidden columns must rewrite DML statements. DuckDB limits SQL only with settings, not for each statement or table. The C++ build is slow, and the installer becomes 20 to 30 MB larger. Armarium does not use the DuckDB file readers and writers ([ADR-0008](0008-ingestion-in-rust.md)), so the only advantage is aggregation speed. |
| Apache DataFusion | Native Rust. Support for UPDATE and DELETE is weak, so we must make our own storage for editing. |
| Polars (SQLContext) | Fast, but it has no DML. |
| Our own in-memory tables and SQL engine | Too much work. |

## Consequences

### Good

- With `rowid`, `STRICT`, the authorizer, and the session extension, the edit model, the types, the sandbox, and undo are simple to make.
- Updates of single rows are fast. This is good for editor operations.
- SQLite is one C file (the amalgamation). The build is fast, and the installer becomes only 1 to 2 MB larger.
- SQLite is in the public domain. There are no license restrictions.
- Users and AI agents know SQLite SQL well.

### Bad

- Aggregation is slower than in a column-oriented engine. Reference value: approximately 0.6 seconds for SUM and AVG with `GROUP BY` on 1 million rows and 1,000 groups. For more than 10 million rows, the user must wait noticeably.
- SQLite has no DATE, DECIMAL, or BOOLEAN type. Armarium stores dates as ISO 8601 text and booleans as the integers 0 and 1. Armarium keeps the logical types in its own metadata ([ADR-0007](0007-round-trip-safe-typing.md)).
- SQLite does not have extended syntax such as `PIVOT` or `QUALIFY`. CTEs, window functions, `RETURNING`, and `FULL OUTER JOIN` (3.39 and later) are available.
- To use the session extension with `rowid` tables, SQLite 3.42 or later and `SQLITE_SESSION_OBJCONFIG_ROWID` are necessary. We will check in M0 whether `rusqlite` makes this available.
