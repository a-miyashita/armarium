# ADR-0007: Infer column types that are round-trip safe

- Status: Accepted
- Date: 2026-10-11

## Context

A CSV file is text without types. Two requirements conflict:

- **Easy aggregation and sorting**: To compare and sort numbers, or to select a range of dates, a column needs a type.
- **Data integrity**: If the user only opens and saves a file, the values must not change. For example, `00123` must not become `123`, `1.50` must not become `1.5`, and `2026/01/02` must not become `2026-01-02`. (This is very important for postal codes, product codes, and telephone numbers.)

If all columns are text, the data is safe, but it is inconvenient. For example, numbers sort in string order.

## Decision

We use this rule: **give a type to a column only if each value can change to the type and back to exactly the original text** (round-trip safe).

### 1. Type system

Each column has a SQLite storage type (the column type of the STRICT table) and an application logical type.

| Logical type | SQLite column type | Value representation |
|---|---|---|
| `Integer` | `INTEGER` | 64-bit integer |
| `Real` | `REAL` | Double-precision floating point |
| `Text` | `TEXT` | String |
| `Date` | `TEXT` | `YYYY-MM-DD` |
| `DateTime` | `TEXT` | `YYYY-MM-DD HH:MM:SS[.fff]` |
| `Boolean` | `INTEGER` | 0 or 1 (only for Excel booleans; Armarium does not infer it for CSV) |

- Armarium stores the logical type in the workspace and in the table metadata. The grid display (right alignment, date editor), input validation, and the MCP tool `describe_table` use it.
- The SQLite date functions (`date()`, `strftime()`) work directly on `Date` and `DateTime` values.

### 2. Inference steps (CSV and TSV)

1. Read all columns into a temporary table as `TEXT`.
2. For each column, try the candidates in this order: `Integer` → `Real` → `Date` → `DateTime`.
3. Select a candidate only if, for all rows except NULL and empty strings, the value changes to the type and back to exactly the original text.

   ```sql
   -- Integer
   SELECT count(*) = 0 FROM tmp
   WHERE v <> '' AND CAST(CAST(v AS INTEGER) AS TEXT) IS NOT v;
   -- Real
   ... WHERE v <> '' AND CAST(CAST(v AS REAL) AS TEXT) IS NOT v;
   -- Date (date(v) alone returns 2026-02-30 unchanged, so use julianday)
   ... WHERE v <> '' AND date(julianday(v)) IS NOT v;
   -- DateTime: use the same method, for example datetime(julianday(v))
   -- (only if all values in the column have the same number of fractional-second digits)
   ```

   - `00123`, `+5`, and `1,234` do not become `Integer`. `1.50` and `1e3` do not become `Real`. `2026-02-30` does not become `Date` (checked with SQLite 3.40).
   - Armarium checks all columns in one aggregate query. Inference uses all rows, not a sample.
4. If no candidate is valid, the type is `Text`.
5. Create a `STRICT` table with the inferred types. Move the data from the temporary table with `INSERT ... SELECT CAST(...)`.

### 3. User overrides

- The user can override the type of each column. For example, the user can make `2026/01/02` a `Date`, although it is not round-trip safe. Armarium converts the values with the date format in the read settings.
- For an overridden column, the UI tells the user that Armarium will write the values in the standard format of the type when it saves.
- A source setting can disable type inference. Then all columns are `Text`.

### 4. Types during editing

- The tables are `STRICT`, so SQLite returns an error if a value does not agree with the column type (for example, `abc` in an `INTEGER` column). This applies to the grid and to DML.
- In the grid, the user selects "Change the column to `Text` and enter the value" or "Cancel the input". For MCP and SQL, Armarium returns the error as it is.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| All columns `TEXT` | Completely safe. SQLite can `SUM` strings, but comparison and sorting use string order, which causes mistakes such as `'10' < '9'`. |
| Use type affinity of a table that is not STRICT | With a declared type, `00123` becomes `123`, so the data is not safe. Without a declared type, it is the same as all text. |
| Keep the original text and the typed value (two columns) | Gives integrity and convenience, but needs two times the space. After DML, it is complicated to know which value is correct. |
| Also infer `Boolean` from `true` and `false` in CSV | Values stored as 0 and 1 are not round-trip safe, so Armarium does not infer it. |

## Consequences

### Good

- A file that the user only opens and saves is byte-for-byte the same (the round-trip test M2-7 makes sure of this).
- Many integer, decimal, and ISO date columns get a type automatically. The user can compare and aggregate them directly.
- The rule is simple and easy to explain to users.

### Bad

- Decimals with a fixed number of digits (for example, `1.50`), `2026/01/02`, and `1,234` (with thousands separators) become `Text`. SQLite can use strings as numbers in `SUM` and `AVG`, but sorting uses string order. Later, we can possibly add read settings for date formats and thousands separators.
- SQLite has no DECIMAL type. Thus, the sum of a `Real` column can have floating-point errors.
- The rule depends on the text format of `CAST(... AS TEXT)` in SQLite. When we update SQLite, the round-trip tests must check it.
- Armarium writes edited values in the standard format of the type.
