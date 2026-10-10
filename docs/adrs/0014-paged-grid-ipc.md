# ADR-0014: The grid fetches only the visible rows through IPC

- Status: Accepted
- Date: 2026-10-11

## Context

- If Armarium sends a table of 1 million rows and tens of columns to the WebView at one time, JSON serialization and JavaScript memory fail.
- Tauri IPC uses JSON (or raw bytes), and each call has a cost.
- Scrolling must be smooth.

## Decision

- The grid fetches only the visible rows and a buffer before and after them, with `fetch_rows(table_id, view_id, offset, limit)`.
  - Default: 200 rows for each page, and 2 pages read ahead before and after. The frontend keeps a maximum of 50 pages in an LRU cache.
- For the default order (the order in the file), Armarium uses the index of the order table:
  ```sql
  SELECT o.rid, t.* FROM "__arm_ord_{id}" o JOIN "sales" t ON t.rowid = o.rid
  ORDER BY o.ord LIMIT ? OFFSET ?;
  ```
  Reference value: approximately 30 ms near row 900,000 of 1 million rows.
- Rust manages sort and filter as a "view" (`view_id`):
  - Armarium creates an internal table `__arm_view_{n}(pos INTEGER PRIMARY KEY, rid)` with the `rowid` values of the matching rows in order, and fetches with `pos BETWEEN ? AND ?`.
  - This is fast also deep in the table, and the relation between the displayed row number and `rowid` is stable.
  - When the table changes, Armarium creates the view again (or asks the user to apply it again).
- Page fetches use a read-only connection that is separate from the write connection. (With WAL, fetches work while heavy operations run.)
- Transfer format: column-oriented JSON (`{ columns: [...], rids: [...], data: [[col0...], [col1...]] }`). If performance is not sufficient, change to a binary format with the raw response of Tauri 2 (`tauri::ipc::Response`).
- When a table changes, Armarium sends a `table-changed { table_id, version }` event. The frontend discards its cache and fetches again.
  - For a cell edit, the frontend replaces only the changed rows.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| Send all data at one time | The simplest method for small files. It fails for large files. |
| For sort and filter also, fetch each time with `ORDER BY ... OFFSET` | Simple to make. A sort on a column without an index sorts all rows each time, and the order of rows with equal values is not stable. |

## Consequences

- Armarium sends only the data that the display needs, independent of the number of rows.
- Sort and filter are also fast on large tables.
- Armarium must manage the creation and removal of views and the cache invalidation.
