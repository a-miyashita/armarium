# ADR-0001: Record design decisions in ADRs

- Status: Accepted
- Date: 2026-10-10

## Context

Armarium combines Rust, Tauri, SQLite, and a web frontend. Many design decisions affect more than one layer.
Later readers must be able to find the reasons for the decisions.

## Decision

We record ADRs in the Michael Nygard format in `docs/adrs/NNNN-kebab-case-title.md`.

- Sections: Context, Decision, Alternatives considered, Consequences (good and bad), and Amendments (if necessary).
- Language: English, in plain language. Keep identifiers and library names as they are.
- Do not change the body of an Accepted ADR. To update information, append an "Amendments" section at the end, with the date.
- If the content changes a lot, write a new ADR and change the status of the old ADR to `Superseded by ADR-XXXX`.
- Keep the list of ADRs in [README.md](README.md).

## Consequences

- The history of each decision stays available. New contributors and AI agents can understand the context more easily.
- Writing ADRs takes more effort.
