# ADR-0010: Make SQL names from file names and sheet names

- Status: Accepted
- Date: 2026-10-11

## Context

SQL needs names to refer to the registered tables.

- File names can contain spaces, symbols, and Japanese characters.
- An Excel workbook has many sheets, and sheet names are frequently the same in different workbooks (`Sheet1`).
- SQLite cannot divide one database into schemas (namespaces). In SQLite, a schema is an attached database.
- Names must be short, predictable, and easy to complete.

## Decision

| Source | SQL name | Example |
|---|---|---|
| CSV, TSV | The file name without the extension | `sales.csv` → `sales` |
| Excel | The workbook name, `_`, and the sheet name | Sheet `2026` in `budget.xlsx` → `budget_2026` |

- Normalization: replace all characters that are not Unicode letters, digits, or `_` with `_`. Replace a sequence of `_` with one `_`. Remove `_` at the start and the end. If the result is empty, use `t`.
- Uniqueness: all SQL names are unique in one namespace. If a name is a duplicate, add `_2`, `_3`, and so on.
- Names that start with `__arm_` or `sqlite_` are not allowed (internal use).
- Names that are SQL keywords or start with a digit are allowed. Completion and templates add double quotes automatically.
- The user can change an SQL name. Armarium saves it in `workspace.json`. If the user moves or renames the file (and connects the source to the new path), the SQL name does not change.
- Column names: use the header value as it is. If it is empty, use `column{n}`. If it is a duplicate, add `_2`. Armarium keeps the display name (the original header text) separately, and writes the original text when it saves.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| Attach a database for each Excel workbook, and write `budget."2026"` | This agrees with the tree structure. But the number of attached databases is limited (10 by default). Undo and transactions across workbooks become complicated. It also conflicts with the rule that the sandbox refuses `ATTACH`. |
| A name with a dot, for example `"budget.2026"` | Always needs quotes. It is easy to write it incorrectly as `budget."2026"`. |
| Use the file path as the name | Unique, but long, and SQL becomes difficult to read. A change of the path breaks the SQL. |
| Sequential numbers (`t1`, `t2`) | Unique and short, but the names have no meaning. |

## Consequences

- In most cases, the file name gives a natural name, and the user can write it without quotes.
- An Excel sheet also gets a one-word name, and frequently needs no quotes.
- From the name alone, the user cannot always see where the workbook name ends and the sheet name starts. The tooltip in the tree and the completion description show the original workbook name and sheet name.
