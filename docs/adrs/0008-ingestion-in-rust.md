# ADR-0008: Read and write files in Rust

- Status: Accepted
- Date: 2026-10-11

## Context

- Many users are in Japan. Armarium must read CSV files in Shift_JIS (CP932) correctly and **write them back in the same encoding**.
- When Armarium saves a file, it must reproduce the delimiter, the quote style, the line ending, the BOM, and the final line ending. Armarium must get this information when it reads the file.
- If a row has the wrong number of fields, Armarium must read it without data loss.
- SQL must not access files ([ADR-0009](0009-sql-sandbox.md)).

## Decision

- Rust code does all reading:
  - Encoding: check the BOM, detect the encoding with `chardetng`, and convert as a stream with `encoding_rs`.
  - CSV parsing: the `csv` crate. Our own code detects the dialect ([05. File formats](../specs/05-file-formats.md)).
  - Excel: `calamine` ([ADR-0011](0011-excel-read-write.md)).
- To insert data into SQLite, Armarium runs a prepared multi-row `INSERT` many times in one transaction, into a temporary table where all columns are `TEXT`. Then type inference creates the working table ([ADR-0007](0007-round-trip-safe-typing.md)).
- Rust code also does all writing (`persist`).
- Armarium does not include the SQLite CSV virtual table extension or any function that reads files.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| The SQLite CSV virtual table (`ext/misc/csv.c`) | UTF-8 only. It does not give the dialect information. If Armarium allows virtual tables, the sandbox becomes weaker. |
| Infer types in Rust during parsing, without the temporary table | Faster, because Armarium inserts the data only one time. But Rust must reproduce exactly the same format as `CAST` in SQLite, so the round-trip guarantee is easier to break. We can consider this as an optimization if performance is not sufficient. |

## Consequences

### Good

- Armarium controls the encoding and the dialect completely, and can make sure that a round trip gives a byte-for-byte identical file.
- SQL has no method to access files.
- The GUI and the MCP server use the same read code.

### Bad

- We must make and test the read code (especially dialect detection).
- Armarium inserts the data two times (temporary table, then working table). Spike M0-8 measures the time against the target (100 MB in 5 seconds).
