//! Reading files into the session database ([ADR-0008]).

pub mod csv;
pub mod encoding;
pub mod excel;

use crate::error::{CoreError, Result};
use crate::fsutil::Fingerprint;
use crate::model::{
    CellNoteKind, ColumnSchema, Dialect, LogicalType, ReadOptions, TableId, TypeSource,
};
use crate::naming::{self, quote_ident};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// The result of a read.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub columns: Vec<ColumnSchema>,
    pub dialect: Option<Dialect>,
    pub row_count: u64,
    pub warnings: Vec<String>,
    /// Formula and error cells of Excel sheets: (rid, column SQL name) → note.
    pub notes: HashMap<(i64, String), (CellNoteKind, String)>,
    pub fingerprint: Fingerprint,
}

/// Cancel and progress for a long read.
pub struct LoadControl<'a> {
    pub cancel: &'a AtomicBool,
    pub progress: &'a dyn Fn(u64, Option<u64>),
}

impl LoadControl<'_> {
    pub fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(CoreError::Cancelled)
        } else {
            Ok(())
        }
    }
}

pub fn ord_table(id: TableId) -> String {
    format!("__arm_ord_{id}")
}

pub fn base_table(id: TableId) -> String {
    format!("__arm_base_{id}")
}

const STAGE: &str = "__arm_stage";

/// Reads a delimited text file and detects its dialect.
pub fn read_text(path: &Path, opts: &ReadOptions) -> Result<(String, Dialect, bool)> {
    let bytes = std::fs::read(path).map_err(|e| CoreError::from_io(e, path))?;
    let decoded = encoding::decode(
        &bytes,
        opts.encoding.as_deref(),
        &path.display().to_string(),
    )?;
    drop(bytes);
    let ext = crate::workspace::extension(path);
    let fixed_delim = opts.delimiter.or(if matches!(ext.as_str(), "tsv" | "tab") {
        Some('\t')
    } else {
        None
    });
    let fixed_quote = opts.quote.map(|q| if q == '\0' { None } else { Some(q) });
    let det = csv::detect(
        &decoded.text,
        fixed_delim,
        fixed_quote,
        opts.has_header,
        opts.skip_rows,
    );
    let text = decoded.text;
    let final_line_ending = text.ends_with(['\n', '\r']);
    let dialect = Dialect {
        encoding: decoded.encoding.name().to_string(),
        bom: decoded.bom,
        delimiter: det.delimiter,
        quote: det.quote,
        escape: det.escape,
        line_ending: det.line_ending,
        has_header: det.has_header,
        final_line_ending,
        skipped_lines: Vec::new(),
        header_quoted: false,
    };
    Ok((text, dialect, det.uncertain))
}

/// Reads a CSV or TSV file into a working table (05. File formats, section 1.2).
pub fn load_csv(
    conn: &Connection,
    id: TableId,
    sql_name: &str,
    path: &Path,
    opts: &ReadOptions,
    ctl: &LoadControl<'_>,
) -> Result<Loaded> {
    let fingerprint = Fingerprint::of(path)?;
    let (text, mut dialect, uncertain) = read_text(path, opts)?;
    let mut warnings = Vec::new();
    if uncertain && crate::workspace::extension(path) == "txt" {
        warnings
            .push("Armarium could not detect the delimiter. Check the read settings.".to_string());
    }
    let total = text.len() as u64;

    crate::db::in_transaction(conn, || {
        drop_internal(conn, id, sql_name)?;
        conn.execute_batch(&format!("DROP TABLE IF EXISTS {STAGE}"))?;

        let mut parser = csv::Parser::new(&text, dialect.delimiter, dialect.quote, dialect.escape);
        dialect.skipped_lines = parser.skip_lines(opts.skip_rows);
        let mut rec = csv::Record::default();

        // The header row (or the first record, to know the number of columns).
        let mut headers: Vec<String> = Vec::new();
        let mut first: Option<(Vec<Option<String>>, Vec<bool>)> = None;
        if parser.next_record(&mut rec).is_some() {
            if dialect.has_header {
                headers = rec.fields().map(str::to_string).collect();
                dialect.header_quoted = (0..rec.len()).all(|i| rec.quoted(i));
            } else {
                first = Some(record_values(&rec));
            }
        }
        let mut ncols = headers
            .len()
            .max(first.as_ref().map(|f| f.0.len()).unwrap_or(0))
            .max(1);
        create_stage(conn, ncols)?;
        let mut all_quoted = vec![true; ncols];
        let mut seen_value = vec![false; ncols];
        let mut ragged = 0u64;
        let mut rows = 0u64;
        let mut stmt_cols = 0usize;
        let mut stmt: Option<rusqlite::Statement<'_>> = None;

        let mut insert = |values: &[Option<String>],
                          quoted: &[bool],
                          ncols: &mut usize,
                          all_quoted: &mut Vec<bool>,
                          seen_value: &mut Vec<bool>|
         -> Result<()> {
            if values.len() > *ncols {
                for c in *ncols..values.len() {
                    conn.execute_batch(&format!("ALTER TABLE {STAGE} ADD COLUMN c{}", c + 1))?;
                }
                *ncols = values.len();
                all_quoted.resize(*ncols, false);
                seen_value.resize(*ncols, true);
            }
            if stmt.is_none() || stmt_cols != *ncols {
                stmt = None;
                let params = vec!["?"; *ncols].join(",");
                stmt = Some(conn.prepare(&format!("INSERT INTO {STAGE} VALUES ({params})"))?);
                stmt_cols = *ncols;
            }
            let st = stmt.as_mut().unwrap();
            for i in 0..*ncols {
                let v = values.get(i).and_then(|v| v.as_deref());
                st.raw_bind_parameter(i + 1, v)?;
                if i < values.len() {
                    seen_value[i] = true;
                    if !quoted[i] {
                        all_quoted[i] = false;
                    }
                }
            }
            st.raw_execute()?;
            Ok(())
        };

        if let Some((vals, q)) = first.take() {
            insert(&vals, &q, &mut ncols, &mut all_quoted, &mut seen_value)?;
            rows += 1;
        }
        while parser.next_record(&mut rec).is_some() {
            let (vals, q) = record_values(&rec);
            if vals.len() != ncols {
                ragged += 1;
            }
            insert(&vals, &q, &mut ncols, &mut all_quoted, &mut seen_value)?;
            rows += 1;
            if rows.is_multiple_of(16384) {
                ctl.check()?;
                (ctl.progress)(parser.position() as u64, Some(total));
            }
        }
        if ragged > 0 {
            warnings.push(format!("{ragged} row(s) had a different number of fields."));
        }

        // Column names.
        let mut display: Vec<String> = Vec::with_capacity(ncols);
        for i in 0..ncols {
            display.push(if dialect.has_header {
                headers.get(i).cloned().unwrap_or_default()
            } else {
                format!("column{}", i + 1)
            });
        }
        let sql_names = naming::column_names(&display);
        let quoted: Vec<bool> = (0..ncols)
            .map(|i| all_quoted[i] && seen_value[i] && rows > 0)
            .collect();
        let (columns, mut w) =
            infer_and_create(conn, id, sql_name, &display, &sql_names, &quoted, opts)?;
        warnings.append(&mut w);
        conn.execute_batch(&format!("DROP TABLE {STAGE}"))?;
        create_internal(conn, id, sql_name)?;
        (ctl.progress)(total, Some(total));
        Ok(Loaded {
            columns,
            dialect: Some(dialect),
            row_count: rows,
            warnings,
            notes: HashMap::new(),
            fingerprint,
        })
    })
}

fn record_values(rec: &csv::Record) -> (Vec<Option<String>>, Vec<bool>) {
    let mut v = Vec::with_capacity(rec.len());
    let mut q = Vec::with_capacity(rec.len());
    for i in 0..rec.len() {
        let f = rec.field(i);
        v.push(if f.is_empty() {
            None
        } else {
            Some(f.to_string())
        });
        q.push(rec.quoted(i));
    }
    (v, q)
}

fn create_stage(conn: &Connection, ncols: usize) -> Result<()> {
    let cols: Vec<String> = (1..=ncols).map(|i| format!("c{i}")).collect();
    conn.execute_batch(&format!("CREATE TABLE {STAGE} ({})", cols.join(", ")))?;
    Ok(())
}

/// Infers the column types and creates the STRICT working table from the stage table ([ADR-0007]).
fn infer_and_create(
    conn: &Connection,
    id: TableId,
    sql_name: &str,
    display: &[String],
    sql_names: &[String],
    quoted: &[bool],
    opts: &ReadOptions,
) -> Result<(Vec<ColumnSchema>, Vec<String>)> {
    let n = display.len();
    let mut warnings = Vec::new();
    let mut types = vec![LogicalType::Text; n];
    if opts.type_inference && n > 0 {
        let mut parts = Vec::new();
        for i in 1..=n {
            let c = format!("c{i}");
            parts.push(format!("count({c})"));
            parts.push(format!(
                "sum({c} IS NOT NULL AND CAST(CAST({c} AS INTEGER) AS TEXT) IS NOT {c})"
            ));
            parts.push(format!(
                "sum({c} IS NOT NULL AND CAST(CAST({c} AS REAL) AS TEXT) IS NOT {c})"
            ));
            parts.push(format!(
                "sum({c} IS NOT NULL AND date(julianday({c})) IS NOT {c})"
            ));
            parts.push(format!(
                "sum({c} IS NOT NULL AND {c} IS NOT datetime(julianday({c})) \
                 AND {c} IS NOT strftime('%Y-%m-%d %H:%M:%f', julianday({c})) \
                 AND {c} IS NOT strftime('%Y-%m-%dT%H:%M:%S', julianday({c})) \
                 AND {c} IS NOT strftime('%Y-%m-%dT%H:%M:%f', julianday({c})))"
            ));
        }
        let sql = format!("SELECT {} FROM {STAGE}", parts.join(", "));
        conn.query_row(&sql, [], |r| {
            for (i, ty) in types.iter_mut().enumerate() {
                let count: i64 = r.get(i * 5)?;
                if count == 0 {
                    continue;
                }
                let bad = |k: usize| -> rusqlite::Result<i64> {
                    Ok(r.get::<_, Option<i64>>(i * 5 + k)?.unwrap_or(0))
                };
                *ty = if bad(1)? == 0 {
                    LogicalType::Integer
                } else if bad(2)? == 0 {
                    LogicalType::Real
                } else if bad(3)? == 0 {
                    LogicalType::Date
                } else if bad(4)? == 0 {
                    LogicalType::DateTime
                } else {
                    LogicalType::Text
                };
            }
            Ok(())
        })?;
    }

    // User overrides (by display name).
    let mut sources = vec![TypeSource::Inferred; n];
    for (i, name) in display.iter().enumerate() {
        let Some(&want) = opts
            .column_types
            .get(name)
            .or_else(|| opts.column_types.get(&sql_names[i]))
        else {
            continue;
        };
        if want == types[i] {
            sources[i] = TypeSource::UserOverride;
            continue;
        }
        match convert_stage_column(conn, i + 1, want) {
            Ok(()) => {
                types[i] = want;
                sources[i] = TypeSource::UserOverride;
            }
            Err(e) => warnings.push(format!(
                "The type of column '{name}' cannot be changed to {}: {e}",
                want.as_str()
            )),
        }
    }

    let defs: Vec<String> = (0..n)
        .map(|i| format!("{} {}", quote_ident(&sql_names[i]), types[i].sqlite_type()))
        .collect();
    let defs = if defs.is_empty() {
        vec![format!("{} TEXT", quote_ident("column1"))]
    } else {
        defs
    };
    conn.execute_batch(&format!(
        "CREATE TABLE {} ({}) STRICT",
        quote_ident(sql_name),
        defs.join(", ")
    ))?;
    let targets: Vec<String> = sql_names.iter().map(|s| quote_ident(s)).collect();
    let exprs: Vec<String> = (0..n)
        .map(|i| {
            let c = format!("c{}", i + 1);
            match types[i] {
                LogicalType::Integer | LogicalType::Boolean => format!("CAST({c} AS INTEGER)"),
                LogicalType::Real => format!("CAST({c} AS REAL)"),
                _ => c,
            }
        })
        .collect();
    if n > 0 {
        conn.execute_batch(&format!(
            "INSERT INTO {}(rowid, {}) SELECT rowid, {} FROM {STAGE}",
            quote_ident(sql_name),
            targets.join(", "),
            exprs.join(", ")
        ))?;
    }
    let _ = id;
    let columns = (0..n.max(1))
        .map(|i| ColumnSchema {
            display_name: display.get(i).cloned().unwrap_or_else(|| "column1".into()),
            sql_name: sql_names
                .get(i)
                .cloned()
                .unwrap_or_else(|| "column1".into()),
            logical_type: types.get(i).copied().unwrap_or(LogicalType::Text),
            type_source: sources.get(i).copied().unwrap_or(TypeSource::Inferred),
            always_quoted: quoted.get(i).copied().unwrap_or(false),
        })
        .collect();
    Ok((columns, warnings))
}

/// Converts the values of a stage column to the standard text of a type.
fn convert_stage_column(conn: &Connection, col: usize, ty: LogicalType) -> Result<()> {
    let c = format!("c{col}");
    let mut updates: Vec<(i64, String)> = Vec::new();
    {
        let mut st = conn.prepare(&format!(
            "SELECT rowid, {c} FROM {STAGE} WHERE {c} IS NOT NULL"
        ))?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let rid: i64 = r.get(0)?;
            let v: String = r.get(1)?;
            let conv = crate::format::parse_input(Some(&v), ty)?;
            let text = match conv {
                rusqlite::types::Value::Integer(i) => i.to_string(),
                rusqlite::types::Value::Real(f) => format!("{f:?}"),
                rusqlite::types::Value::Text(s) => s,
                _ => continue,
            };
            if text != v {
                updates.push((rid, text));
            }
        }
    }
    let mut st = conn.prepare(&format!("UPDATE {STAGE} SET {c} = ?1 WHERE rowid = ?2"))?;
    for (rid, t) in updates {
        st.execute(rusqlite::params![t, rid])?;
    }
    Ok(())
}

/// Creates the order table, the base table, and the triggers ([ADR-0006]).
///
/// The base table maps each line of the file (as it was read or last saved) to a rowid.
/// `diff_table` uses it.
pub fn create_internal(conn: &Connection, id: TableId, sql_name: &str) -> Result<()> {
    let ord = ord_table(id);
    let base = base_table(id);
    let t = quote_ident(sql_name);
    conn.execute_batch(&format!(
        r#"
        CREATE TABLE "{ord}" (rid INTEGER PRIMARY KEY, ord REAL NOT NULL);
        INSERT INTO "{ord}"(rid, ord) SELECT rowid, rowid FROM {t};
        CREATE INDEX "{ord}_ix" ON "{ord}"(ord);
        CREATE TABLE "{base}" (line INTEGER PRIMARY KEY, rid INTEGER NOT NULL);
        INSERT INTO "{base}"(line, rid) SELECT rowid, rowid FROM {t};
        "#
    ))?;
    create_triggers(conn, id, sql_name)
}

/// Creates the triggers that keep the order table up to date.
pub fn create_triggers(conn: &Connection, id: TableId, sql_name: &str) -> Result<()> {
    let ord = ord_table(id);
    let t = quote_ident(sql_name);
    conn.execute_batch(&format!(
        r#"
        CREATE TRIGGER "{ord}_ins" AFTER INSERT ON {t}
          WHEN NOT EXISTS (SELECT 1 FROM "{ord}" WHERE rid = NEW.rowid)
          BEGIN
            INSERT INTO "{ord}"(rid, ord)
              VALUES (NEW.rowid, coalesce((SELECT max(ord) FROM "{ord}"), 0) + 1);
          END;
        CREATE TRIGGER "{ord}_del" AFTER DELETE ON {t}
          BEGIN
            DELETE FROM "{ord}" WHERE rid = OLD.rowid;
          END;
        "#
    ))?;
    Ok(())
}

/// Drops the working table and its internal tables.
pub fn drop_internal(conn: &Connection, id: TableId, sql_name: &str) -> Result<()> {
    conn.execute_batch(&format!(
        r#"DROP TABLE IF EXISTS {};
           DROP TABLE IF EXISTS "{}";
           DROP TABLE IF EXISTS "{}";"#,
        quote_ident(sql_name),
        ord_table(id),
        base_table(id)
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests;
