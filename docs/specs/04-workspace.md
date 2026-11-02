# 04. Workspace

- Status: Draft
- Last updated: 2026-11-02

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
- Armarium does not delete the file. The exception is a scratch source: Armarium deletes its file (see section 6.6).
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
  "workspaceId": "0192f1c3-...",
  "nextScratchNumber": 3,
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
- `workspaceId` and `nextScratchNumber` are for scratch sources (section 6). Both are optional. Armarium adds `workspaceId` (a random UUID) when the user saves the first scratch source. A missing `nextScratchNumber` means 1.
- A source can have `"scratch": true`. The default is `false`. The path of a scratch source is in the scratch directory.
- These are new optional fields, so the schema version stays 1. An older Armarium ignores them and keeps them ([ADR-0012](../adrs/0012-workspace-persistence.md), amendment 2026-11-02).
- Armarium ignores unknown fields when it reads the file, and keeps them when it writes the file (forward compatibility).

### 5.2 Window state

- Armarium saves the pane widths, the open tabs, the text in the SQL editor, and the query history in a separate file, `%APPDATA%\Armarium\ui-state.json`. (The MCP server does not use this data, so it is in a separate file.)

## 6. Scratch sources

### 6.1 Purpose

A scratch source keeps the result of a SELECT statement so that the user can open it, edit it, and use it in later SQL. The user does not choose a file name or a folder. Armarium owns the file.

SQL cannot create tables ([ADR-0009](../adrs/0009-sql-sandbox.md)). A scratch source is the replacement for `CREATE TABLE AS`. The UI action is "Save as scratch" ([03. UI](03-ui.md), section 4).

A scratch source is a normal source in most ways. It has a table, an SQL name, undo and redo, and it appears in the workspace file. The differences are in sections 6.2 to 6.7.

### 6.2 Location and names

| Item | Rule |
|---|---|
| Directory | `%LOCALAPPDATA%\Armarium\scratch\<workspaceId>\`. Armarium creates it when needed. |
| File name | `scratch_NN.csv`. `NN` has at least two digits: `scratch_01`, `scratch_02`, … `scratch_99`, `scratch_100`. |
| SQL name | The file name without the extension (`scratch_01`), by the normal rule of [ADR-0010](../adrs/0010-sql-naming.md). If the name is already used, Armarium adds `_2`. |
| Format | CSV, UTF-8 without BOM, comma, LF, with a header row. Armarium quotes a field only when it needs to. |

- Each workspace file has its own `workspaceId`. Two workspace files (for example, with the MCP option `--workspace`) never share a scratch directory.
- The number comes from `nextScratchNumber` in `workspace.json`. Armarium takes the number and increments the counter while it holds the workspace lock ([ADR-0012](../adrs/0012-workspace-persistence.md)), and then it writes the file. Because the counter only grows, a number is not used again after a delete or a restart.
- If a file with the chosen name already exists, Armarium takes the next number.

### 6.3 Create: "Save as scratch"

1. Armarium takes the next number (section 6.2).
2. Armarium writes all rows of the result to a temporary file in the scratch directory, and then replaces it with the final name (atomic replacement). The UI shows progress and a cancel button, as for an export.
3. Armarium registers the file as a source with `scratch: true`. It follows the steps in section 2.2.
4. The tree shows the new source in the "Scratch" group.

Rules:

- The scratch source is a snapshot. Armarium does not keep the query. It does not run the query again.
- After the save, the scratch source and the result are not connected. Closing the query tab does not change the scratch source.
- If the user cancels or the write fails, Armarium deletes the temporary file and registers nothing. The number stays used, so there can be a gap in the numbers.
- There is no limit on the number or the size of scratch sources in the first release.

Known limit: a scratch file is a CSV file. It cannot keep the difference between NULL and an empty string (an empty field is NULL; see [09. Roadmap](09-roadmap.md), Q3). When Armarium reads the file, it infers the column types again ([ADR-0007](../adrs/0007-round-trip-safe-typing.md)). A column type can be different from the type in the result.

### 6.4 Edit and save

- The user edits a scratch table in the same way as other tables ([06. Editing](06-editing.md)).
- Armarium saves a scratch table to its file by itself. It saves when 2 seconds pass after the last change, when the tab is closed, and when the application exits. Ctrl+S saves at once. The tree and the tab do not show `●`, and the exit dialog does not list the table.
- If the automatic save fails (for example, the disk is full or the file is locked), the tree shows `⚠`. The table then works as a normal dirty table: its tab shows `●`, and it appears in the exit dialog until the user saves it. After a failure, Armarium tries again only after the next change, or when the user saves.
- Armarium checks for external changes before each save, as for other files ([06. Editing](06-editing.md), section 5.2). If another program changed the file, the automatic save stops, and the normal warning dialog appears at the next manual save.

### 6.5 Save as

"Save as…" turns a scratch source into a normal source. It **moves** the data. It does not copy it.

1. The dialog starts in the user's Documents folder. The default file name is the SQL name plus `.csv`. The user can choose CSV or TSV. (For XLSX, the user uses "Export…", as for other CSV sources.) Armarium writes the file with the export defaults for the extension (UTF-8 with BOM, CRLF; a tab as the delimiter for `.tsv`). The scratch file itself has no BOM and uses LF.
2. Armarium writes the file and connects the source to the new path. This is the same operation as "Save as" for a CSV source ([06. Editing](06-editing.md), section 5.1). It refuses a path that is registered as another source.
3. Armarium sets `scratch` to `false`. The source moves to the "CSV / TSV" group.
4. Armarium deletes the old file in the scratch directory.
5. The SQL name does not change ([ADR-0010](../adrs/0010-sql-naming.md)). SQL that uses `scratch_01` still works. The user can rename the SQL name.

If Armarium cannot delete the old file, it ignores the error. The cleanup at the next start (section 6.7) deletes the file.

### 6.6 Export and delete

- "Export…" writes a copy of the table to a file. The scratch source does not change. It works as for other tables.
- "Delete scratch" removes the source and **deletes its file**. The GUI asks the user to confirm. Changes of the working copy are lost.
- Unregistration of a scratch source always deletes the file. This is true for the MCP tool `unregister_source` too. The MCP server does not ask for confirmation, as for its other write tools.
- "Change file location" is not available for a scratch source. The user uses "Save as…" instead.

### 6.7 Cleanup at start

When the GUI starts, Armarium looks at the scratch directory of the workspace. It deletes each file that matches `scratch_NN.csv` (or is an unfinished temporary file) and is not registered as a source. It does not touch files with other names. The MCP server does not run this cleanup.

If a registered scratch file is missing, Armarium shows `⚠` ([section 2.4](#24-when-armarium-cannot-find-a-file)). This can happen when the user uses a roaming profile: `workspace.json` is in `%APPDATA%`, but the scratch directory is in `%LOCALAPPDATA%` and does not move to another PC. The user can delete such a source.
