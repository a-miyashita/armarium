# ADR-0012: Store the workspace in a JSON file

- Status: Accepted
- Date: 2026-10-11

## Context

Armarium must keep the list of registered sources and their settings (SQL names, read settings, column type overrides) after a restart.
Both the GUI and the MCP server (a different process) read and write this information.

## Decision

- Store the workspace as JSON in `%APPDATA%\Armarium\workspace.json` (for the schema, see section 5.1 of [04. Workspace](../specs/04-workspace.md)).
- The file has a `version` field for migrations. Armarium keeps unknown fields.
- Write with a temporary file and atomic replacement. Before each write, read the latest file again and merge for each source (`id`).
  - If two processes change the same source at the same time, the last write wins. (This occurs rarely, so we accept it.)
- Use a lock file (`workspace.json.lock`, an exclusive lock with the `fs4` crate) for exclusive writes.
- Keep information that only the GUI uses (window state, query history) in a separate file, `ui-state.json`.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| Store in an SQLite file | No new dependency, and good for concurrent access. But the amount of data is small, so JSON is sufficient. People cannot read or edit it directly. |
| `tauri-plugin-store` | Easy, but it depends on Tauri, so the MCP binary cannot easily use it ([ADR-0004](0004-cargo-workspace-layout.md)). |
| Windows registry | Not good for structured data. Difficult to back up and move. |

## Consequences

- People can read and edit the file. Backup and migration are easy.
- The GUI and the MCP server can share the file.
- We must handle concurrent writes ourselves (lock and merge).

## Amendments

### 2026-10-11: File lock

- Armarium uses `std::fs::File::lock` (Rust 1.89 and later) for `workspace.json.lock` instead of the
  `fs4` crate. The behavior is the same, and there is one dependency less.

### 2026-11-02: Fields for scratch sources

- Scratch sources ([04. Workspace](../specs/04-workspace.md), section 6) add three optional fields:
  `workspaceId` (a random UUID), `nextScratchNumber` (an integer counter), and `scratch` (a boolean on
  a source).
- The schema `version` stays 1, because the fields are optional and Armarium keeps unknown fields.
- Merge rule: when two processes write at the same time, `nextScratchNumber` takes the larger value,
  so a scratch number is never used again. `workspaceId` is written once, and the first writer wins.
- Armarium stores the scratch files in `%LOCALAPPDATA%\Armarium\scratch\<workspaceId>\`, not in
  `%APPDATA%`. The files are large and are not worth copying with a roaming profile. A consequence:
  after the profile roams to another PC, scratch sources show `⚠` there.
