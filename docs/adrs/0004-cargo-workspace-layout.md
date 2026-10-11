# ADR-0004: Put the domain logic in a core crate that does not depend on Tauri

- Status: Accepted
- Date: 2026-10-11

## Context

The GUI (Tauri) and the MCP server (headless) both use the same functions: read, edit, SQL, and save.
Also, we want to test the logic without a UI.

## Decision

Use this Cargo workspace structure:

```
crates/armarium-core   # Library. Does not depend on Tauri or rmcp.
crates/armarium-mcp    # Binary. armarium-core + rmcp
src-tauri              # Binary. armarium-core + tauri
```

- The public API of `armarium-core` has synchronous `Workspace` and `Engine` handles, typed errors, and typed events.
- The Tauri commands and the MCP tools are thin adapters. They only call the core API.
- The core sends progress and table change notifications through a callback (a trait). Each adapter converts them to Tauri events or MCP notifications.
- The core defines the types that IPC uses, with `serde` and `ts-rs` derives. `ts-rs` generates the TypeScript type definitions from them.
  - We write the TypeScript functions that call the Tauri commands ourselves. There are approximately 20 commands, so this is a small amount of work.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| Put everything in `src-tauri` | Faster at the start, but we must separate the code later for MCP. |
| One GUI binary with a `--mcp` flag | Only one file to distribute. When an MCP client starts it, we must prevent the initialization of Tauri and WebView2, and the dependencies are large. Also, log output must not go to stdout. |
| `tauri-specta` to generate the types and the command functions | It also generates the functions that call the commands. But version 2 is still a release candidate (2.0.0-rc.25) after more than two years, and a release candidate can have breaking changes. |

## Consequences

- We can run `cargo test` on the core alone, and the tests are fast.
- The MCP binary does not depend on Tauri, so it stays small.
- From the start, we must design the boundaries between the crates (the public API) carefully.
- When we add or change a Tauri command, we must also update the TypeScript function for it by hand.
