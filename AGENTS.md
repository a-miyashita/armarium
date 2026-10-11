# AGENTS.md

Instructions for AI coding agents (and human contributors) in this repository.

## Project

Armarium is a Windows desktop editor for tabular data (CSV, TSV, Excel) with SQL and an MCP server.
The stack is Rust, Tauri 2, and SQLite.

- Specifications: [docs/specs/](docs/specs/README.md)
- Design decisions: [docs/adrs/](docs/adrs/README.md)

## Documentation language

- Write all documentation in this repository in **English**. This includes specifications, ADRs, README files, code comments, and commit messages.
- Use plain English: short sentences, active voice, one term for one concept. Write for readers who are not native English speakers.
- You can keep non-English text only where it is data. Examples: sample values, column names in SQL examples, and file names such as `売上 2026.csv`.
- When you talk with a user in another language, you can reply in that language. The files that you write in the repository must still be in English.

## ADRs

- Follow the rules in [ADR-0001](docs/adrs/0001-record-architecture-decisions.md).
- Do not change the body of an Accepted ADR. Append an "Amendments" section with the date, or write a new ADR.
