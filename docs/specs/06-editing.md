# 06. Editing

- Status: Draft
- Last updated: 2026-10-11

## 1. Edit model

For details, see [ADR-0006](../adrs/0006-working-copy-edit-model.md).

- When the user opens a table, Armarium reads the file into a **working table** in SQLite.
- Edits in the grid and SQL DML statements both change the working table.
- The file does not change until the user saves.

### 1.1 Working table

- The name of the working table is the SQL name (for example, `sales`). It is a `STRICT` table that has only the columns of the file.
- The row ID is the hidden SQLite `rowid`. `SELECT *` does not show it.
- The order of the rows is in an internal table, `__arm_ord_{table_id}(rid, ord)`.
  - Triggers keep it up to date: an `INSERT` into the working table adds a row at the end, and a `DELETE` removes the row.
  - When the user inserts a row in the middle in the grid, Armarium sets `ord` to a value between the values of the rows before and after it.
- Armarium runs user SQL as it is, without changes.

## 2. Edit operations

| Operation (`EditOp`) | Processing in SQLite | Undo method |
|---|---|---|
| `SetCells { cells: [(rid, col, value)] }` | `UPDATE ... WHERE rowid = ?` | Changeset |
| `InsertRows { after_rid, count }` | `INSERT`, and update the order | Changeset |
| `DeleteRows { rids }` | `DELETE WHERE rowid IN (...)` | Changeset |
| `Paste { anchor, tsv }` | Split into `SetCells` (and `InsertRows` if necessary) | Changeset |
| `Sql { statement }` | Run the user SQL | Changeset |
| `InsertColumn { name, type, position }` | `ALTER TABLE ADD COLUMN`, and update the column order metadata | Reverse operation (delete the column) |
| `DeleteColumn { name }` | `ALTER TABLE DROP COLUMN` | Reverse operation (add the column and restore the values). Armarium records the column data before the change. |
| `RenameColumn { from, to }` | `ALTER TABLE RENAME COLUMN` | Reverse operation |
| `ChangeColumnType { name, type }` | Create the table again (copy the data into a new STRICT table and keep the `rowid` values) | Reverse operation. Armarium records the column data before the change. |

- Armarium runs one UI operation (for example, a paste) in one transaction. One UI operation is one undo unit.
- If a value does not agree with the column type (for example, `abc` in an `INTEGER` column), SQLite returns an error (STRICT). In the grid, the user then selects "Change the column to `Text` and enter the value" or "Cancel the input".
- Armarium keeps the column order in metadata (the order of the `ColumnSchema` entries), not in the SQLite column order. Reason: `ALTER TABLE ADD COLUMN` can add a column only at the end.

### 2.1 Value input

- The grid gets each input as a string. Armarium converts the string to the logical type of the column.
- Armarium stores an empty input as NULL. (CSV does not make a difference between NULL and an empty string. Excel also does not.)
- A `Date` column accepts ISO 8601 (`2026-01-02`). It also accepts `2026/01/02` and converts it to ISO 8601.

## 3. Undo and redo

### 3.1 Mechanism

- For each operation, the session extension records a changeset. To undo, Armarium inverts the changeset and applies it. To redo, Armarium applies the changeset.
- The `rowid` is the key in the changeset. Thus, when the user undoes a delete, the restored rows get the same row IDs.
- The same changeset also contains the changes to the order table.

### 3.2 Stacks

- Each table has its own undo stack and redo stack. (There is no global stack.)
  - A DML statement can read other tables, but it changes only one table.
- Limits: 1,000 operations by default, or 256 MB of changesets in total. When a stack is full, Armarium discards the oldest entries.
- If the changeset of one operation is larger than the limit, Armarium asks the user before it commits the transaction: "You cannot undo this operation. Do you want to continue?" If the user says no, Armarium rolls back the transaction.
- Saving does not clear the undo history. (If the user undoes an operation after a save, the table becomes dirty again.)
- A reload or a discard of the working table clears the history.

## 4. Dirty state

- For each operation, Armarium increments a change counter of the table. A table is dirty when its counter is different from the counter at the last save. (If the user undoes back to the state at the last save, the table is not dirty.)
- To find the tables that SQL changed, Armarium uses the authorizer callbacks (the target tables of `SQLITE_INSERT`, `SQLITE_UPDATE`, and `SQLITE_DELETE`).
- The tree and the tab of a dirty table show `●`.

## 5. Save

### 5.1 Save operations

| Operation | Behavior |
|---|---|
| Save (Ctrl+S) | Write the active table to the original file. For an Excel sheet in M4, this changes to "Save as". |
| Save all | Save all dirty tables. |
| Save as | Write to a different file, and connect the source to the new file. |
| Export | Write to a different file. The source does not change. |

### 5.2 Detection of external changes

- When Armarium reads a file, it records a fingerprint: the size, the date of the last change, and the xxHash3 of the first and last 64 KB.
- Immediately before a save, Armarium calculates the fingerprint again. If it is different, Armarium returns `EXTERNAL_MODIFICATION` and shows a dialog:
  - Overwrite / Reload (discard changes) / Save as / Cancel
- For an overwrite of an Excel file (M6), Armarium does not allow the overwrite if there is an external change. Reason: the overwrite could delete changes in other sheets.

### 5.3 Write failure

- If the file is locked (for example, because Excel has it open), Armarium returns `FILE_LOCKED` and shows "Retry" and "Save as".
- If a write fails before it is complete, the original file does not change (atomic replacement).

## 6. Exit

- If there are dirty tables, Armarium shows a list of them and asks the user to confirm.
- Recovery after a crash is a possible later feature: if Armarium keeps the session database after an abnormal exit, it can possibly restore the changes that were not saved at the next start.
