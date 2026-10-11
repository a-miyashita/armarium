//! Reading Excel and OpenDocument workbooks with calamine ([ADR-0011]).

use super::{create_internal, drop_internal, LoadControl, Loaded};
use crate::error::{CoreError, Result};
use crate::fsutil::Fingerprint;
use crate::model::{CellNoteKind, ColumnSchema, LogicalType, ReadOptions, TableId, TypeSource};
use crate::naming::{self, quote_ident};
use crate::workspace::SheetListing;
use calamine::{open_workbook_auto, Data, Range, Reader, SheetType, SheetVisible};
use rusqlite::types::Value;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;

fn open(path: &Path) -> Result<calamine::Sheets<std::io::BufReader<std::fs::File>>> {
    if !path.exists() {
        return Err(CoreError::FileNotFound(path.display().to_string()));
    }
    open_workbook_auto(path).map_err(|e| match e {
        calamine::Error::Io(io) => CoreError::from_io(io, path),
        other => CoreError::Parse(format!("{}: {other}", path.display())),
    })
}

/// Lists the worksheets of a workbook (charts and macro sheets are not tables).
pub fn list_sheets(path: &Path) -> Result<Vec<SheetListing>> {
    let wb = open(path)?;
    Ok(wb
        .sheets_metadata()
        .iter()
        .filter(|s| s.typ == SheetType::WorkSheet)
        .map(|s| SheetListing {
            name: s.name.clone(),
            hidden: s.visible != SheetVisible::Visible,
        })
        .collect())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Int,
    Float,
    Bool,
    Date,
    DateTime,
    Text,
}

fn kind_of(d: &Data) -> Option<Kind> {
    Some(match d {
        Data::Empty | Data::Error(_) => return None,
        Data::Int(_) => Kind::Int,
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 9.0e15 {
                Kind::Int
            } else {
                Kind::Float
            }
        }
        Data::Bool(_) => Kind::Bool,
        Data::DateTime(dt) if dt.is_datetime() => match dt.as_datetime() {
            Some(n) if n.time() == chrono::NaiveTime::MIN => Kind::Date,
            Some(_) => Kind::DateTime,
            None => Kind::Text,
        },
        Data::String(s) if s.is_empty() => return None,
        _ => Kind::Text,
    })
}

/// The text of a cell, similar to the display in Excel.
pub fn display_text(d: &Data) -> Option<String> {
    Some(match d {
        Data::Empty | Data::Error(_) => return None,
        Data::Int(i) => i.to_string(),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 9.0e15 {
                format!("{}", *f as i64)
            } else {
                format!("{f}")
            }
        }
        Data::String(s) => {
            if s.is_empty() {
                return None;
            }
            s.clone()
        }
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(dt) => match dt.as_datetime() {
            Some(n) if dt.is_datetime() => format_datetime(n),
            _ => format!("{}", dt.as_f64()),
        },
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
    })
}

fn format_datetime(n: chrono::NaiveDateTime) -> String {
    if n.time() == chrono::NaiveTime::MIN {
        n.format("%Y-%m-%d").to_string()
    } else if n.and_utc().timestamp_subsec_millis() != 0 {
        n.format("%Y-%m-%d %H:%M:%S%.3f").to_string()
    } else {
        n.format("%Y-%m-%d %H:%M:%S").to_string()
    }
}

fn typed_value(d: &Data, ty: LogicalType) -> Value {
    match (ty, d) {
        (_, Data::Empty | Data::Error(_)) => Value::Null,
        (LogicalType::Integer, Data::Int(i)) => Value::Integer(*i),
        (LogicalType::Integer, Data::Float(f)) => Value::Integer(*f as i64),
        (LogicalType::Real, Data::Int(i)) => Value::Real(*i as f64),
        (LogicalType::Real, Data::Float(f)) => Value::Real(*f),
        (LogicalType::Boolean, Data::Bool(b)) => Value::Integer(*b as i64),
        _ => match display_text(d) {
            Some(s) => Value::Text(s),
            None => Value::Null,
        },
    }
}

/// Reads one sheet into a working table (05. File formats, section 2.1).
pub fn load_sheet(
    conn: &Connection,
    id: TableId,
    sql_name: &str,
    path: &Path,
    sheet: &str,
    opts: &ReadOptions,
    ctl: &LoadControl<'_>,
) -> Result<Loaded> {
    let fingerprint = Fingerprint::of(path)?;
    let mut wb = open(path)?;
    let range: Range<Data> = wb
        .worksheet_range(sheet)
        .map_err(|e| CoreError::Parse(format!("{sheet}: {e}")))?;
    let formulas: Option<Range<String>> = wb.worksheet_formula(sheet).ok();
    let start = range.start().unwrap_or((0, 0));
    let width = range.width();
    let mut rows = range.rows().skip(opts.skip_rows as usize);
    let has_header = opts.has_header.unwrap_or(true);

    let mut display: Vec<String> = (0..width).map(|i| format!("column{}", i + 1)).collect();
    let header_offset = opts.skip_rows as usize + usize::from(has_header);
    if has_header {
        if let Some(h) = rows.next() {
            for (i, c) in h.iter().enumerate() {
                display[i] = display_text(c).unwrap_or_default();
            }
        }
    }
    let data: Vec<&[Data]> = rows.collect();
    let width = width.max(1);
    if display.is_empty() {
        display.push("column1".into());
    }
    let sql_names = naming::column_names(&display);

    // Column types: a column gets a type only if all values have the same type.
    let mut types = Vec::with_capacity(width);
    for j in 0..width {
        let mut k: Option<Kind> = None;
        let mut mixed = false;
        for r in &data {
            if let Some(kc) = r.get(j).and_then(kind_of) {
                k = match (k, kc) {
                    (None, x) => Some(x),
                    (Some(a), b) if a == b => Some(a),
                    (Some(Kind::Int), Kind::Float) | (Some(Kind::Float), Kind::Int) => {
                        Some(Kind::Float)
                    }
                    (Some(Kind::Date), Kind::DateTime) | (Some(Kind::DateTime), Kind::Date) => {
                        Some(Kind::DateTime)
                    }
                    _ => {
                        mixed = true;
                        break;
                    }
                };
            }
        }
        let mut ty = match (mixed, k) {
            (true, _) | (_, None) | (_, Some(Kind::Text)) => LogicalType::Text,
            (_, Some(Kind::Int)) => LogicalType::Integer,
            (_, Some(Kind::Float)) => LogicalType::Real,
            (_, Some(Kind::Bool)) => LogicalType::Boolean,
            (_, Some(Kind::Date)) => LogicalType::Date,
            (_, Some(Kind::DateTime)) => LogicalType::DateTime,
        };
        if !opts.type_inference {
            ty = LogicalType::Text;
        }
        types.push(ty);
    }
    let mut sources = vec![TypeSource::Inferred; width];
    let mut warnings = Vec::new();
    for j in 0..width {
        if let Some(&want) = opts
            .column_types
            .get(&display[j])
            .or_else(|| opts.column_types.get(&sql_names[j]))
        {
            let ok = data.iter().all(|r| {
                let v = r
                    .get(j)
                    .map(|d| typed_value(d, types[j]))
                    .unwrap_or(Value::Null);
                crate::format::convert_value(&v, want).is_ok()
            });
            if ok {
                types[j] = want;
                sources[j] = TypeSource::UserOverride;
            } else {
                warnings.push(format!(
                    "The type of column '{}' cannot be changed to {}.",
                    display[j],
                    want.as_str()
                ));
            }
        }
    }

    crate::db::in_transaction(conn, || {
        drop_internal(conn, id, sql_name)?;
        let defs: Vec<String> = (0..width)
            .map(|j| format!("{} {}", quote_ident(&sql_names[j]), types[j].sqlite_type()))
            .collect();
        conn.execute_batch(&format!(
            "CREATE TABLE {} ({}) STRICT",
            quote_ident(sql_name),
            defs.join(", ")
        ))?;
        let cols: Vec<String> = sql_names.iter().map(|s| quote_ident(s)).collect();
        let params: Vec<String> = (0..=width).map(|i| format!("?{}", i + 1)).collect();
        let mut st = conn.prepare(&format!(
            "INSERT INTO {}(rowid, {}) VALUES ({})",
            quote_ident(sql_name),
            cols.join(", "),
            params.join(", ")
        ))?;
        let mut notes = HashMap::new();
        for (i, r) in data.iter().enumerate() {
            let rid = i as i64 + 1;
            st.raw_bind_parameter(1, rid)?;
            for j in 0..width {
                let d = r.get(j).unwrap_or(&Data::Empty);
                let v = typed_value(d, types[j]);
                let v = if sources[j] == TypeSource::UserOverride {
                    crate::format::convert_value(&v, types[j])?
                } else {
                    v
                };
                st.raw_bind_parameter(j + 2, v)?;
                if let Data::Error(e) = d {
                    notes.insert(
                        (rid, sql_names[j].clone()),
                        (CellNoteKind::Error, e.to_string()),
                    );
                }
                if let Some(f) = &formulas {
                    let abs = (start.0 + (header_offset + i) as u32, start.1 + j as u32);
                    if let Some(text) = f.get_value(abs) {
                        if !text.is_empty() {
                            notes.insert(
                                (rid, sql_names[j].clone()),
                                (CellNoteKind::Formula, format!("={text}")),
                            );
                        }
                    }
                }
            }
            st.raw_execute()?;
            if i % 16384 == 0 {
                ctl.check()?;
                (ctl.progress)(i as u64, Some(data.len() as u64));
            }
        }
        drop(st);
        create_internal(conn, id, sql_name)?;
        let columns = (0..width)
            .map(|j| ColumnSchema {
                display_name: display[j].clone(),
                sql_name: sql_names[j].clone(),
                logical_type: types[j],
                type_source: sources[j],
                always_quoted: false,
            })
            .collect();
        Ok(Loaded {
            columns,
            dialect: None,
            row_count: data.len() as u64,
            warnings: warnings.clone(),
            notes,
            fingerprint: fingerprint.clone(),
        })
    })
}
