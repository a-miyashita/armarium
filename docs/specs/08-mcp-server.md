# 08. MCP server

- Status: Draft
- Last updated: 2026-10-11

## 1. Overview

Armarium gives its functions as a [Model Context Protocol](https://modelcontextprotocol.io/) server.
MCP clients (AI agents) such as Claude Desktop and Claude Code can then read, aggregate, and transform the registered files.

For details, see [ADR-0013](../adrs/0013-mcp-server-mode.md).

## 2. Operation modes

| Mode | How it starts | State | Phase |
|---|---|---|---|
| Standalone (stdio) | The MCP client starts `armarium-mcp.exe`. | A session database that is independent of the GUI. The workspace settings are shared. | M5 |
| GUI-attached (later) | `armarium-mcp.exe --attach` connects to the running GUI through a named pipe. | Operates on the same working copies as the GUI (including changes that are not saved). | M6 or later |

### 2.1 Example client configuration

```json
{
  "mcpServers": {
    "armarium": {
      "command": "C:\\Program Files\\Armarium\\armarium-mcp.exe",
      "args": []
    }
  }
}
```

- Later, "Settings → MCP" in the GUI will have a button that adds this configuration to the Claude Desktop configuration file.

### 2.2 Start options

| Option | Meaning | Default |
|---|---|---|
| `--workspace <path>` | Path of `workspace.json` | `%APPDATA%\Armarium\workspace.json` |
| `--read-only` | Disables the write tools (`execute`, `save_table`, `register_source`, `unregister_source`) | Off |
| `--allow-dir <path>` | A directory from which `register_source` can register files. You can give this option more than one time. | None. Without this option, registration is not possible. |
| `--query-timeout <sec>` | Query timeout | 60 |
| `--max-rows <n>` | Maximum number of rows in one response | 500 |

## 3. Tools

| Tool | Description | Writes |
|---|---|---|
| `list_sources` | List the registered sources and tables (SQL name, path, type, load state) | - |
| `describe_table` | Column names, logical types, number of rows, and a sample of the first 5 rows | - |
| `read_rows` | Get rows of a table with `offset` and `limit` | - |
| `query` | Run a query (SELECT) and return the result | - |
| `execute` | Run a DML statement (INSERT, UPDATE, DELETE). This changes the working copy. | ✔ |
| `diff_table` | The differences between the working copy and the file (number of changed rows, and some of the changed cells) | - |
| `save_table` | Save the working copy to the file | ✔ |
| `discard_changes` | Discard the working copy and read the file again | ✔ |
| `register_source` | Register a file (only in the `--allow-dir` directories) | ✔ |
| `unregister_source` | Unregister a source (the file is not deleted) | ✔ |
| `export_result` | Write a query result to CSV or XLSX (only in the `--allow-dir` directories) | ✔ |

### 3.1 Example input and output

```jsonc
// Input of query
{ "sql": "SELECT 部署, SUM(金額) AS 合計 FROM sales GROUP BY 部署", "max_rows": 100 }

// Output of query (structuredContent)
{
  "columns": [{ "name": "部署", "type": "text" }, { "name": "合計", "type": "integer" }],
  "rows": [["営業", 1200000], ["開発", 800000]],
  "row_count": 2,
  "truncated": false
}
```

- The server returns the output as `structuredContent` (JSON) and also as a Markdown table with the same contents (for compatibility with clients).
- When `truncated` is `true`, the text tells the agent to add `LIMIT` or to use `export_result`.

### 3.2 Tool annotations

- Read tools: `readOnlyHint: true`
- `execute`, `discard_changes`: `destructiveHint: false` (the file does not change), `idempotentHint: false`
- `save_table`: `destructiveHint: true` (the file is overwritten)

## 4. Resources and prompts

| Type | URI or name | Contents |
|---|---|---|
| Resource | `armarium://tables/{sql_name}/schema` | The schema of the table (JSON) |
| Resource | `armarium://tables/{sql_name}/sample` | The first 20 rows (CSV text) |
| Prompt | `analyze_table` | A template that gets statistics for each column of a table (count, number of NULLs, minimum, maximum, number of distinct values) and asks the agent to explain the patterns |

## 5. Safety

- The SQL sandbox is the same as in the GUI ([ADR-0009](../adrs/0009-sql-sandbox.md)).
- Only two tools write files: `save_table` (overwrites a registered file) and `export_result` (in the `--allow-dir` directories).
- If `save_table` detects an external change, it fails. (There is no overwrite confirmation as in the GUI.) The agent must call `discard_changes` and try again.
- If the GUI has the same file open and dirty: the GUI detects the save as an external change and shows a warning when the user saves.
  (In standalone mode, the MCP server cannot know the dirty state of the GUI. The GUI-attached mode will solve this.)
- `--max-rows` limits the number of rows in a response. This prevents waste of the agent's context.

## 6. Sharing the workspace

- `armarium-mcp` reads `workspace.json` again at start and before each tool call, but only if the date of the last change is different. This makes registrations in the GUI available.
- `register_source` and `unregister_source` change `workspace.json`. The GUI detects this through file monitoring and updates the tree.
