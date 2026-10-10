# ADR-0009: Limit SQL to queries and DML

- Status: Accepted
- Date: 2026-10-11

## Context

- The user wants to "aggregate and transform data with standard SQL (DML)".
- Through MCP, AI agents write SQL. Their input can possibly contain a prompt injection.
- `DROP`, `ALTER`, `VACUUM`, and writes to internal tables must not break the edit model ([ADR-0006](0006-working-copy-edit-model.md)).
- If SQL can create a database file at any path with `ATTACH`, SQL can write to the file system.

## Decision

We limit SQL in several layers. The main layer is the SQLite authorizer callback (`sqlite3_set_authorizer`).

### 1. Allow list in the authorizer

Armarium enables the authorizer when it prepares user SQL. The authorizer allows only these operations:

| Action code | Condition |
|---|---|
| `SQLITE_SELECT`, `SQLITE_RECURSIVE` | Always allowed |
| `SQLITE_READ` | The target is a working table or a query result table. |
| `SQLITE_INSERT`, `SQLITE_UPDATE`, `SQLITE_DELETE` | The target is a working table. An `UPDATE` of the `rowid` column is refused. |
| `SQLITE_FUNCTION` | The function is on the allow list. (Armarium does not include `load_extension` and similar functions at all.) |
| The operations above, when the trigger name argument is an Armarium internal trigger | Reads and writes of internal tables (`__arm_*`) are allowed. |

- The authorizer refuses all other operations: `CREATE`, `DROP`, `ALTER`, `ATTACH`, `DETACH`, `PRAGMA`, `TRANSACTION`, `SAVEPOINT`, `ANALYZE`, `REINDEX`, creation of virtual tables, and others.
- The authorizer also refuses `VACUUM` (to protect `rowid`).
- A refusal causes an SQLite error (`not authorized`). Armarium returns it as `SQL_FORBIDDEN`.

### 2. Connection settings

- Enable `SQLITE_DBCONFIG_DEFENSIVE` (prevents writes to `sqlite_schema` and similar operations).
- Disable `SQLITE_DBCONFIG_TRUSTED_SCHEMA`.
- Set `SQLITE_LIMIT_ATTACHED` to 0.
- Do not use the `load_extension` feature of `rusqlite` (this removes extension loading from the build).

### 3. Resource limits

- The user can cancel a query (`sqlite3_interrupt`).
- The MCP server has a timeout (a progress handler checks it) and a maximum number of result rows.
- `cache_size` and `soft_heap_limit` limit the memory use.

### 4. Internal SQL of the application

- Armarium runs its own SQL (reading files, creating tables, applying undo, and others) with the authorizer disabled.
- The same worker thread runs user SQL and application SQL one at a time on the same connection. Thus, the authorizer switching cannot mix.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| No limits | Too dangerous with MCP. Internal structures can break. |
| Check for forbidden keywords with regular expressions | Easy to get around with comments, quotes, and letter case. |
| Allow only read-only statements with `sqlite3_stmt_readonly` | DML is not possible, so the "transform" requirement fails. We use it as an additional check in the `--read-only` mode of MCP and in the `query` tool. |

## Consequences

### Good

- The SQL parser itself makes the decision, so it is not possible to get around it.
- The control works for each table and each column. Internal tables and `rowid` are safe.
- The authorizer tells Armarium which tables SQL reads and changes. Armarium uses this for the dirty state.
- SQL has no method to access files or the network.

### Bad

- The user cannot use familiar methods such as `CREATE TABLE AS` or temporary views. "Save as new source" and "Export" in the UI replace them.
- We must maintain the list of allowed functions. The regression tests for refused operations (M3-8) detect problems.
