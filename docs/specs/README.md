# Armarium specifications

Armarium is a viewer and editor for tabular data (CSV, TSV, and Excel) on Windows.
You can aggregate and transform the data with SQL.
The goal is that AI agents can also use Armarium as an MCP server.

## Documents

| # | Document | Contents |
|---|---|---|
| 01 | [Overview](01-overview.md) | Purpose, scope, terms, and non-functional requirements |
| 02 | [Architecture](02-architecture.md) | Components, processes, and data flow |
| 03 | [UI](03-ui.md) | Screen layout, the two panes, and user operations |
| 04 | [Workspace](04-workspace.md) | How to register and unregister sources, the tree, and persistence |
| 05 | [File formats](05-file-formats.md) | How Armarium reads and writes CSV, TSV, and Excel files |
| 06 | [Editing](06-editing.md) | The edit model, undo and redo, saving, and conflict detection |
| 07 | [SQL](07-sql.md) | Supported SQL, table names, and how Armarium runs queries and shows results |
| 08 | [MCP server](08-mcp-server.md) | MCP tools and operation modes |
| 09 | [Roadmap](09-roadmap.md) | Milestones, tasks, and dependencies |

For the design decisions, see the [ADR index](../adrs/README.md).

## Status

The specifications are **Draft**. The ADRs are **Accepted** (2026-10-11).
The first implementation (M0 to M5) is done. For the status and the known limits, see [09. Roadmap](09-roadmap.md), section 5.
ADR-0005, 0006, 0007, 0008, 0009, and 0012 have amendments with the results of the implementation.
