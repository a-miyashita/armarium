# ADR-0011: Read Excel with calamine, and add write support in phases

- Status: Accepted
- Date: 2026-10-11

## Context

- The user wants to show and edit Excel files, and use them in SQL.
- Excel files contain more than values: formatting, formulas, merged cells, charts, macros (.xlsm), and others.
- If Armarium writes only the values and overwrites the file, all of these are lost. This is a serious data loss for the user.
- The Rust Excel libraries have different purposes: read only (calamine), create new files only (rust_xlsxwriter), and edit existing files (umya-spreadsheet).

## Decision

1. **Read**: use `calamine`. It supports `.xlsx`, `.xlsm`, `.xlsb`, `.xls`, and `.ods`, and it is fast.
   - For formula cells, Armarium shows the cached value. These cells are read-only.
   - Armarium identifies dates from the number format.
2. **Write (first phase: M4)**: Armarium does **not** overwrite the original file. With "Save as", Armarium writes a new `.xlsx` file (`rust_xlsxwriter`) or a CSV file.
3. **Write (later phase: M6)**: add overwrite of `.xlsx` and `.xlsm` files with `umya-spreadsheet`.
   - Armarium opens the original file, updates **only the changed cell values**, and saves it. Formatting, formulas, other sheets, and VBA stay.
   - Row and column insertion and deletion move the cell references in Excel formulas. Thus, the first implementation of overwrite does not include them (only "Save as").
   - Before an overwrite, if the file contains elements that Armarium cannot keep (for example, pivot tables or charts), Armarium shows a warning.
   - Before the implementation, we examine which elements a round trip through umya-spreadsheet loses. We record that decision in a separate ADR.
4. `.xls`, `.xlsb`, and `.ods` files are always read-only.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| Use umya-spreadsheet for read and write from the start | Reads more slowly than calamine and uses more memory. Does not support `.xls`. |
| Write only the values and overwrite | Formatting and formulas are lost. Not acceptable. |
| Control Excel through COM | Excel must be installed. Not usable for MCP or CI. |

## Consequences

### Good

- In the first phase, there is no risk of damage to Excel files.
- Reading supports many formats and is fast.

### Bad

- Until M6, the user cannot overwrite Excel files. This can be different from what users expect. The UI must tell the user why the save changes to "Save as".
- Reading and writing use different libraries, so the handling of types and dates must agree.
