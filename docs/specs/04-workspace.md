# 04. Workspace

- Status: Draft
- Last updated: 2026-10-11

## 1. Overview

The workspace keeps the list of registered sources and the settings of each source. The application has one workspace. (We can possibly add more than one workspace later.)

## 2. Registration

### 2.1 Files that the user can register

| Extension | Type | Notes |
|---|---|---|
| `.csv` | CSV | Armarium detects the delimiter (the default is a comma). |
| `.tsv`, `.tab` | TSV | The delimiter is a tab. |
| `.txt` | Text | Armarium detects the delimiter. If it cannot detect the delimiter, it shows the read settings dialog. |
| `.xlsx`, `.xlsm` | Excel | Read and write |
| `.xls` | Excel (old format) | Read only |
| `.ods` | OpenDocument | Read only (calamine supports it, so the extra cost is small) |

### 2.2 Steps during registration

1. Normalize the path (absolute path, long paths with `\\?\`, keep the letter case).
2. If the same path is already registered, select the existing source and stop. (Armarium does not register a file two times.)
3. Create a `Source` and assign SQL names (see section 4).
4. For an Excel workbook, read only the list of sheets and create the child nodes. Do not read the data.
5. Save `workspace.json`.
6. Registration does not read the data. Armarium reads the data when the user opens the table, or when SQL refers to it (lazy loading).

### 2.3 Unregistration

- Armarium removes the `Source` from the workspace and drops its tables (working copies) in the session database.
- Armarium does not delete the file.
- If there are changes that are not saved, Armarium shows a dialog and asks the user to confirm.

### 2.4 When Armarium cannot find a file

- At start, or when the user opens the table, Armarium shows `⚠` if the file does not exist. The registration stays.
- With right-click → "Change file location", the user can connect the source to a different path. The SQL names and settings do not change.

## 3. Monitoring external changes

- Armarium monitors the registered files with the `notify` crate. It monitors each parent directory one time for all the files in it.
- When Armarium detects a change:
  - Table not loaded: Armarium does nothing. It reads the latest file when the user opens the table.
  - Table loaded and not dirty: The tree shows `↻`, and Armarium reloads the table automatically. (A setting can change this to a manual reload.)
  - Table dirty: The tree shows `↻`. Armarium shows a warning when the user saves ([06. Editing](06-editing.md)).
- For an Excel workbook, Armarium also updates the list of sheets (added, deleted, and renamed sheets).

## 4. SQL names

For details, see [ADR-0010](../adrs/0010-sql-naming.md).

- CSV and TSV: the file name without the extension
  - Example: `売上 2026.csv` → `売上_2026`
- Excel: the workbook name, `_`, and the sheet name
  - Example: sheet `2026` in `budget.xlsx` → `budget_2026`
- Rules:
  - Letters, digits, underscores, and non-ASCII characters (for example, Japanese) are allowed. Armarium replaces all other characters with `_`.
  - All SQL names are unique in one namespace. If a name is a duplicate, Armarium adds `_2`, `_3`, and so on.
  - Names that start with `__arm_` or `sqlite_` are not allowed (they are for internal use).
  - A name can be the same as an SQL keyword. In SQL, the user then puts the name in double quotes. Completion adds the quotes automatically.
  - The user can change a name at any time. Armarium saves the change in the workspace.

## 5. Persistence

For details, see [ADR-0012](../adrs/0012-workspace-persistence.md).

- Location: `%APPDATA%\Armarium\workspace.json`
- Armarium writes the file with atomic replacement. Both the GUI and the MCP server read and write the file. Thus, before each write, Armarium reads the file again and merges the changes.

### 5.1 Schema (version 1)

```json
{
  "version": 1,
  "sources": [
    {
      "id": "0192f1c4-...",
      "path": "C:\\Users\\me\\data\\sales.csv",
      "kind": "csv",
      "sqlName": "sales",
      "addedAt": "2026-10-10T12:00:00Z",
      "options": {
        "encoding": null,
        "delimiter": null,
        "quote": null,
        "hasHeader": null,
        "skipRows": 0,
        "typeInference": true,
        "columnTypes": { "zip": "text" }
      }
    },
    {
      "id": "0192f1c5-...",
      "path": "C:\\Users\\me\\data\\budget.xlsx",
      "kind": "excel",
      "options": { "hasHeader": true },
      "sheets": [
        { "name": "2026", "sqlName": "budget_2026", "options": { "headerRow": 1 } }
      ]
    }
  ]
}
```

- `null` means "detect automatically".
- Armarium ignores unknown fields when it reads the file, and keeps them when it writes the file (forward compatibility).

### 5.2 Window state

- Armarium saves the pane widths, the open tabs, the text in the SQL editor, and the query history in a separate file, `%APPDATA%\Armarium\ui-state.json`. (The MCP server does not use this data, so it is in a separate file.)
