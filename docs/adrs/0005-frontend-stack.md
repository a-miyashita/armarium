# ADR-0005: Use React and CodeMirror 6 for the frontend, and select the grid with a spike

- Status: Accepted
- Date: 2026-10-11

## Context

The UI has three main parts:

- A table grid that scrolls and edits smoothly, also with 1 million rows
- An SQL editor with completion
- A source tree

The grid must have these functions:

- Virtual scrolling, with an API that gives the data only for the visible range (for page fetches, [ADR-0014](0014-paged-grid-ipc.md))
- Cell editing
- Range selection
- Copy and paste of ranges (TSV, for use with Excel)
- Correct Japanese input (IME)

## Decision

| Purpose | Decision | Reason |
|---|---|---|
| Framework | React, TypeScript, and Vite | The largest ecosystem of components. Tauri has an official template. |
| Table grid | **Not decided yet.** Spike M0-6 compares Glide Data Grid and RevoGrid. Then we update this ADR. | See "Grid candidates" below. |
| SQL editor | CodeMirror 6 (`@codemirror/lang-sql`) | Small and modular. It can complete names from a schema. Smaller than Monaco. |
| Tree | Our own implementation (ARIA tree pattern) or `react-arborist` | The number of nodes is small (hundreds at most), so a small solution is sufficient. |
| State management | Zustand | Small and simple. Easy to update from IPC events. |
| i18n | i18next | Japanese and English |
| IPC types | Generate TypeScript types from Rust with `ts-rs` ([ADR-0004](0004-cargo-workspace-layout.md)) | Prevents type mismatches. |

### Grid candidates

| Candidate | Good | Risk |
|---|---|---|
| Glide Data Grid (`@glideapps/glide-data-grid`) | Canvas rendering for millions of rows. Cell editing, range selection, copy and paste, and column resize are built in. The `getCellContent` API agrees well with page fetches. MIT license. | The last stable version is 6.0.3 (February 2024). It supports React only up to 18. React 19 support is only in alpha versions (6.0.4-alpha24, October 2025). There were no releases in the last year. |
| RevoGrid (`@revolist/revogrid`) | Actively maintained (latest release October 2026). A web component that does not depend on a framework, with a React wrapper. Virtual scrolling, editing, range selection, and clipboard. | We must check the performance with page fetches for 1 million rows, and the IME behavior. We must check which functions are in the free version. |

Spike M0-6 compares the two candidates with these criteria: scrolling with page fetches for 1 million rows, range selection, copy and paste, and Japanese input.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| AG Grid Community | Many projects use it. The Infinite Row Model (range-based loading) is available in Community. But range selection and the clipboard are only in the Enterprise version. An editor cannot work without them. The bundle is also large. |
| TanStack Table and TanStack Virtual | Flexible, but we must make editing, selection, and copy and paste ourselves. |
| Svelte or SolidJS | Small and fast, but there are fewer components such as grids. |
| Monaco Editor | Many functions, but heavy (several MB). CodeMirror 6 has all the functions that we need. |

## Consequences

- We decide the grid from measured data, so the risk of a wrong selection is lower.
- Until M0-6 is complete, the grid code in M1 cannot start (M1-9 depends on M0-6).
- If we select Glide Data Grid, we depend on an alpha version for React 19. In this case, we must watch its maintenance and be ready to change to the other candidate.
