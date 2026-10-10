# 07. SQL

- Status: Draft
- Last updated: 2026-10-11

## 1. Overview

- The SQL engine is SQLite ([ADR-0003](../adrs/0003-sqlite-as-engine.md)). The user writes SQL in the SQLite dialect.
- Armarium allows only **queries and DML statements** ([ADR-0009](../adrs/0009-sql-sandbox.md)).
- SQL operates on the tables in the workspace (the working copies).

## 2. Allowed statements

| Type | Statements | Notes |
|---|---|---|
| Query | `SELECT`, `WITH ... SELECT`, `VALUES` | CTEs (also recursive), window functions, and `FULL OUTER JOIN` are available. |
| DML | `INSERT`, `UPDATE`, `DELETE` | Each statement changes one registered table. `RETURNING` and `UPDATE ... FROM` are available. |
| Query plan | `EXPLAIN QUERY PLAN`, `EXPLAIN` | Shows the plan or the bytecode. Does not run the query. |

### 2.1 Refused operations

- DDL: `CREATE`, `ALTER`, `DROP`. To change columns or structure, the user uses the grid operations.
- `ATTACH`, `DETACH`, `PRAGMA`, `VACUUM`, `ANALYZE`, `REINDEX`
- Transaction control (`BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`). Armarium controls the transactions.
- References to internal tables (`__arm_*`, `sqlite_*`)
- Updates of `rowid`
- Calls to functions that are not on the allow list, and table-valued functions other than `json_each` and `json_tree`
- `VACUUM` (a statement that is not read-only must change exactly one working table)

### 2.2 How Armarium checks statements

- Armarium enables the authorizer when it prepares user SQL. The SQLite parser asks the authorizer about each operation in the statement (the tables and columns that the statement reads or writes, the functions, the DDL, and others). Armarium compares each operation with the allow list.
- If the authorizer refuses an operation, the prepare step fails with `SQL_FORBIDDEN`. Nothing runs.
- Armarium prepares and runs more than one statement one at a time.

## 3. Table references

### 3.1 Names

For details, see [ADR-0010](../adrs/0010-sql-naming.md).

- CSV and TSV: `sales`
- Excel: `budget_2026` (workbook name and sheet name)
- Column names: SQL names that Armarium makes from the header row in the file. Japanese column names are also available (`SELECT 金額 FROM sales`).
- Identifiers are not case-sensitive (for ASCII letters). Put names that contain spaces or symbols in double quotes.

### 3.2 Loading tables that are not loaded

- A table that is only registered is not loaded yet. Armarium loads it when SQL refers to it.
- Steps:
  1. Before Armarium runs the SQL, it splits the text into tokens. It loads each table whose SQL name matches a token. Loading too many tables does not change the result, so Armarium prefers to load too many rather than miss one.
  2. If the prepare step fails with `no such table: X`, Armarium checks whether `X` is in the workspace. If it is, Armarium loads the table and prepares the statement again.
- While Armarium loads a table, the UI shows the progress. The user can cancel it.

## 4. Running statements and results

### 4.1 SELECT

- Armarium keeps the result in an internal table, `__arm_res_{n}`. First, Armarium prepares the SELECT statement alone with the authorizer enabled, to check it. Then Armarium runs `INSERT INTO __arm_res_{n} SELECT * FROM (<statement>)`; the authorizer allows only this insert. The grid fetches the result in pages.
  - Armarium knows the total number of rows immediately, and does not run the query again when the user scrolls.
- Armarium sets the logical type of each result column from the value types (`typeof`). If the result contains an original column as it is, the result column gets the logical type of the original column.
- When the user closes the query tab, Armarium drops the result table.
- Operations on a result: export to CSV or XLSX, save as a new source (write a file and register it in the workspace), copy to the clipboard.

### 4.2 DML

- Armarium runs each statement in one transaction and pushes the changeset on the undo stack ([06. Editing](06-editing.md), section 3).
- Armarium shows the number of changed rows and makes the target table dirty.
- If the target table is open in the grid, the grid updates.
- The user can also change the working copy of a read-only source (for example, `.xls`). For these sources, the user can save only by export.

### 4.3 Cancel and timeout

- The user can cancel a running statement (`sqlite3_interrupt`).
- The GUI has no timeout. The MCP server has a default timeout of 60 seconds ([08. MCP server](08-mcp-server.md)).

## 5. Additional functions

In addition to the built-in SQLite functions, Armarium adds user-defined functions in Rust. M3 adds a minimum set. We add more functions later when users request them.

| Function | Purpose |
|---|---|
| `regexp(pattern, text)` | Makes the `REGEXP` operator available (`regex` crate) |
| `regexp_replace(text, pattern, repl)` | Replace text with a regular expression |
| `normalize_nfkc(text)` | Normalize full-width and half-width characters (to match Japanese data) |

## 6. Help in the SQL editor

- Completion: keywords, SQL names in the workspace, column names of loaded tables, and function names.
- Templates: right-click → "Create SELECT statement" in the tree inserts `SELECT * FROM sales LIMIT 100;`.
- Display of syntax errors.

## 7. Examples

```sql
-- Join and aggregate many files
SELECT u.部署, SUM(s.金額) AS 合計
FROM sales s
JOIN users u ON u.id = s.user_id
GROUP BY u.部署
ORDER BY 合計 DESC;

-- Compare with an Excel sheet
SELECT b.科目, b.予算, a.実績, a.実績 - b.予算 AS 差異
FROM budget_2026 b
LEFT JOIN actual a USING (科目);

-- Monthly totals (a Date column is ISO 8601 text, so string functions and date functions work)
SELECT strftime('%Y-%m', date) AS month, SUM(amount) FROM sales GROUP BY 1;

-- Update many rows (this changes the working copy; a save writes the change to the file)
UPDATE sales SET status = 'done' WHERE date < '2026-04-01';
```
