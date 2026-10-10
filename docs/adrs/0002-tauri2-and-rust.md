# ADR-0002: Use Tauri 2 and Rust as the application platform

- Status: Accepted
- Date: 2026-10-11

## Context

Armarium is a desktop application for Windows. It must do these things:

- Read large files (hundreds of MB) quickly, with little memory.
- Run an SQL engine in the same process.
- Give a rich UI (table grid, tree, SQL editor) with a reasonable amount of work.
- Have a small installer and start quickly.
- Run the GUI and the MCP server (headless) with the same logic.

## Decision

- Use **Tauri 2** as the application platform. Write the backend in **Rust**. Make the UI with web technology on WebView2.
- Do all heavy work (read, query, write) in Rust. Send only the data that the display needs to the WebView.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| Electron and Node.js | Many projects use it, and it is easy to make the UI. The installer is large (more than 100 MB), and it uses much memory. Encoding conversion of CSV files in Node is slow. |
| WinUI 3 or WPF (C#) | Native Windows look. It is difficult to also give an MCP server and to support other platforms. It does not agree with our technology stack (Rust). |
| A native Rust GUI (for example, egui or Slint) | One language for everything. We must make an advanced table grid, a code editor, and IME support (Japanese input) ourselves. This costs too much. |
| Tauri 1 | Version 2 is released and has a better permission model (capabilities) and better plugins. There is no reason to select version 1 for a new project. |

## Consequences

### Good

- The installer is small (WebView2 is part of the operating system). The application starts quickly.
- We can use the types and the performance of Rust for data processing. The MCP binary can share `armarium-core` ([ADR-0004](0004-cargo-workspace-layout.md)).
- We can use the many web UI components (grids, editors).
- Capabilities limit the commands that the WebView can call. This makes the application safer.

### Bad

- There are two languages, Rust and TypeScript, and the types at the IPC boundary must agree. To decrease this risk, we generate TypeScript types from Rust with `ts-rs` ([ADR-0004](0004-cargo-workspace-layout.md)).
- The application depends on the behavior of WebView2 (IME, font rendering).
- IPC has a cost (JSON serialization). The design must not send large amounts of data ([ADR-0014](0014-paged-grid-ipc.md)).
