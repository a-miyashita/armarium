# ADR-0006: Edit a working copy in SQLite and save explicitly

- Status: Accepted
- Date: 2026-10-11

## Context

- Edits in the grid and SQL DML statements both change the data. Both must see the same data.
- To prevent damage to files by mistakes, the user must decide when a change goes to the file.
- Armarium must keep the identity of each row (for undo and for the selection in the grid). It must also keep the order of the rows in the file, including rows that the user inserts in the middle.
- SQL must show only the columns of the file. Management columns confuse users and AI agents.

## Decision

### 1. Working copy

- When the user opens a table, Armarium reads the file and creates a **working table** in SQLite.
- All edits change the working table. When the user saves, Armarium writes the working table to the file. There is no automatic save.

### 2. Table structure

- The name of the working table is the SQL name. The working table is a `STRICT` table that has **only the columns of the file**.
  ```sql
  CREATE TABLE "sales" ("id" INTEGER, "name" TEXT, "amount" INTEGER) STRICT;
  ```
- The row ID is the hidden SQLite `rowid`. `SELECT *` does not show it.
  - Only `VACUUM` changes `rowid` values. Thus, Armarium never runs `VACUUM`, and the authorizer refuses it ([ADR-0009](0009-sql-sandbox.md)).
  - Armarium does not declare `INTEGER PRIMARY KEY` on a user column (it would become an alias of `rowid`).
- An internal table keeps the order of the rows:
  ```sql
  CREATE TABLE "__arm_ord_{table_id}" (rid INTEGER PRIMARY KEY, ord REAL NOT NULL);
  CREATE INDEX ... ON "__arm_ord_{table_id}"(ord);
  ```
  - When Armarium reads a file, it creates all rows with `ord = rowid`.
  - Triggers keep the table up to date: an `INSERT` into the working table adds a row at the end (`max(ord) + 1`), and a `DELETE` removes the row.
  - When the user inserts a row in the middle in the grid, Armarium sets `ord` to a value between the values of the rows before and after it. When the precision is not sufficient, Armarium assigns new values to all rows.
- As a result, Armarium can run user SQL (SELECT and DML) **without changes**.

### 3. Undo and redo

- Armarium runs one operation (a grid operation or one DML statement) in one transaction. The session extension records the changes of the transaction as a changeset.
- To undo, Armarium inverts the changeset and applies it. To redo, Armarium applies the changeset. The `rowid` is the key in the changeset, so a row that comes back after an undo has the same row ID.
- The same changeset also contains the changes to the order table (from the triggers).
- The session extension does not record column operations (add, delete, rename, change type with `ALTER TABLE` or a new table). For these operations, Armarium records the reverse operation and the column data before the change.
- If the changeset of a DML statement is larger than the limit, Armarium asks the user before it commits the transaction: "You cannot undo this operation. Do you want to continue?" If the user says no, Armarium rolls back the transaction.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| Put management columns (`__rid`, `__ord`) in the working table, and make the SQL name a view without them | DML on a view needs `INSTEAD OF` triggers. In a trigger, there is no reliable method to find the original row (the behavior of `rowid` on a view is not documented). |
| Put management columns in the working table and show them | `SELECT *` shows the management columns. |
| Use the order of `rowid` as the row order | Insertion in the middle is not possible. |
| Undo with a change log that triggers write | A fallback if the session extension is not available. Armarium must create three triggers for each table, and DML becomes slower. |
| Edit the file directly (no save) | A mistake changes the file immediately. Writes of large files are slow. |

## Consequences

### Good

- Grid edits and SQL see the same data. Armarium runs user SQL without changes, and `SELECT *` returns only the columns of the file.
- The file does not change until the user saves. Together with the detection of external changes, this is safe.
- Row IDs are stable, so undo and the grid selection are stable.
- Grid operations and DML use the same undo mechanism.

### Bad

- The stability of `rowid` depends on the rule "never run `VACUUM`". The authorizer and tests protect this rule.
- Each `INSERT` into the working table runs a trigger on the order table. (The index keeps the cost for each row small.)
- If `rusqlite` cannot use the session extension with `rowid` tables, we change to the trigger method for undo (we check this in M0).
