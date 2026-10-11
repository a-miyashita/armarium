# 05. File formats

- Status: Draft
- Last updated: 2026-10-11

This document specifies how Armarium reads (ingest) and writes (persist) files. Rust code does both ([ADR-0008](../adrs/0008-ingestion-in-rust.md)).

## 1. CSV and TSV

### 1.1 Dialect detection

Armarium uses the start of the file (1 MB by default) to detect the items below. If `SourceOptions` overrides an item, Armarium does not detect that item.

| Item | Detection method | Candidates |
|---|---|---|
| Encoding | BOM → valid as UTF-8? → detection with `chardetng` | UTF-8, UTF-8 BOM, Shift_JIS (CP932), EUC-JP, UTF-16 LE/BE, ISO-8859-1 |
| Delimiter | For each candidate, check that the number of fields is the same in each row | `,` `\t` `;` `\|` |
| Quote character | How often it occurs, and whether it is next to a delimiter | `"` `'` none |
| Escape | Doubled quote characters (`""`) by default. Change to `\"` if `\"` occurs. | `""`, `\"` |
| Line ending | The first line ending in the file | CRLF, LF, CR |
| Header row | The types in row 1 differ from the types in the other rows. Row 1 has no duplicate or empty values. | Yes / no |
| Final line ending | Whether the file ends with a line ending | Yes / no |

- Armarium attaches the result to the working copy as a `Dialect`. When Armarium saves the file, it uses the same format.
- For `.tsv` and `.tab`, the delimiter is always a tab.
- If Armarium cannot detect the encoding, it returns `ENCODING_UNDETECTED`, and the UI shows the read settings dialog.

### 1.2 Read

1. Read the file and convert it to UTF-8 with `encoding_rs` (the whole file in memory; see the amendment of ADR-0008).
2. Parse the data with the Armarium parser (`ingest/csv.rs`). The parser records which fields were quoted, so that a save can quote them again. If a row has the wrong number of fields, fill the missing fields with NULL, and add columns for the extra fields. (No data is lost.)
   - The status bar shows a warning with the number of rows that had the wrong number of fields.
3. Decide the column names. If there is a header row, use row 1. If not, use `column1`, `column2`, and so on.
   - For an empty name, the SQL name is `column{n}`. For a duplicate name, the SQL name is, for example, `name_2`. The display name stays the same as the value in the file.
4. Insert all rows into a temporary table where all columns are `TEXT`. (One transaction, with a prepared multi-row `INSERT`.)
5. Infer the column types ([ADR-0007](../adrs/0007-round-trip-safe-typing.md)). Create a STRICT working table and move the data into it.
6. Create the order table (`ord = rowid`) and the triggers ([ADR-0006](../adrs/0006-working-copy-edit-model.md)).

### 1.3 Write (save)

- Armarium reads the working table in the order of the order table and writes the rows in the `Dialect` of the source.
  - Armarium reproduces the delimiter, the quote character, the line ending, the encoding, the BOM, the header row, and the final line ending.
  - By default, Armarium quotes a value only when it is necessary (when the value contains a delimiter, a quote character, or a line ending). If the original file quoted all fields, Armarium quotes all fields.
- Text of the values: a value that the user did not edit has the same text as in the original file (type inference makes sure of this). For an edited value, Armarium uses the standard format of the type.
- If the data contains characters that Shift_JIS cannot encode (for example, emoji), Armarium stops with an error before it saves, and shows the positions of the cells. The user can select to save in UTF-8.
- Atomic replacement: Armarium writes to a temporary file in the same directory, and replaces the original file with `ReplaceFileW` (Windows). Armarium keeps the attributes and the ACL of the original file.

## 2. Excel

For details, see [ADR-0011](../adrs/0011-excel-read-write.md).

### 2.1 Read

- Armarium reads Excel files with `calamine` (`.xlsx`, `.xlsm`, `.xlsb`, `.xls`, `.ods`).
- Armarium reads the list of sheets at registration. It reads the cells when the user opens a sheet.
- Armarium uses the used range (dimension) of the sheet as the table. By default, row 1 is the header row. (The user can change this in the read settings, or select "no header row".)
- Cell types:

| Excel | Logical type (if all values in the column have the same type) | SQLite column type |
|---|---|---|
| Number (integers only) | `Integer` | `INTEGER` |
| Number | `Real` | `REAL` |
| String | `Text` | `TEXT` |
| Boolean | `Boolean` | `INTEGER` (0 or 1) |
| Date or date and time (from the number format) | `Date` or `DateTime` | `TEXT` (ISO 8601) |
| Error (for example, `#N/A`) | NULL. Armarium keeps the error type as cell metadata and shows it in the grid. | - |
| Empty | NULL | - |

- If a column has values of different types, the column type is `Text`. (Armarium converts each value to a string that is similar to the Excel display format.)
- Formula cells: Armarium shows the cached value. Armarium keeps the formula, and the cell is **read-only**.
- Merged cells: the top-left cell has the value. The other cells are NULL.
- Armarium also reads hidden rows, columns, and sheets. The tree shows hidden sheets in a lighter color.

### 2.2 Write

We add support in phases.

| Phase | Behavior |
|---|---|
| M4 (first) | The user can edit the working copy of an Excel sheet. The user can save only with "Save as" (to a new `.xlsx` file or a CSV file). Armarium cannot overwrite the original file. "Save as" writes the new file and registers it as a new source. The sheet stays connected to the workbook (and stays dirty until the user discards the changes). |
| M6 (later) | Armarium can overwrite `.xlsx` and `.xlsm` files. It updates only the cell values and keeps the formatting, the formulas, and the other sheets (`umya-spreadsheet`). |

- `.xls`, `.ods`, and `.xlsb` files are always read-only. (The user can export them.)
- To write a new `.xlsx` file, Armarium uses `rust_xlsxwriter`. The header row is bold. Each column gets a number format or a date format from its type.

## 3. Export

| Output format | Data | Options |
|---|---|---|
| CSV, TSV | Table, query result | Encoding, BOM, delimiter, line ending, quote style, header row |
| XLSX | Table, query result | Sheet name |

- Rust code also writes the exported files. SQL cannot write files ([ADR-0009](../adrs/0009-sql-sandbox.md)).
