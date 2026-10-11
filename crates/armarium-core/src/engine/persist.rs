//! Writing tables and results to files (05. File formats, sections 1.3, 2.2, and 3).

use super::views::{display_expr, result_table};
use super::Inner;
use crate::error::{CoreError, Result};
use crate::fsutil::{atomic_write, Fingerprint};
use crate::ingest::csv::write_field;
use crate::ingest::encoding::{lookup, TextEncoder};
use crate::ingest::{base_table, ord_table, LoadControl};
use crate::model::*;
use crate::naming::quote_ident;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// A column to write.
pub struct OutColumn {
    pub header: String,
    pub ty: LogicalType,
    pub always_quoted: bool,
}

const MAX_REPORTED: usize = 5;

/// Writes rows as delimited text in a dialect. `rows` gives the text of each field.
pub fn write_csv_rows<W: Write>(
    w: &mut W,
    dialect: &Dialect,
    columns: &[OutColumn],
    header: bool,
    quote_all: bool,
    mut rows: impl FnMut(&mut dyn FnMut(&[Option<String>]) -> Result<()>) -> Result<()>,
) -> Result<()> {
    let enc = lookup(&dialect.encoding)?;
    let mut encoder = TextEncoder::new(enc);
    if dialect.bom {
        w.write_all(encoder.bom())?;
    }
    let le = dialect.line_ending.as_str();
    let mut first = true;
    let mut line = String::new();
    let mut bytes = Vec::with_capacity(1 << 16);
    let mut bad: Vec<(u64, String)> = Vec::new();
    let mut bad_count = 0usize;
    let mut row_no: u64 = 0;

    let mut emit =
        |line: &str, row_no: u64, fields: Option<&[Option<String>]>, w: &mut W| -> Result<()> {
            bytes.clear();
            if let Err(_c) = encoder.encode(line, &mut bytes) {
                bad_count += 1;
                if bad.len() < MAX_REPORTED {
                    let col = fields
                        .and_then(|f| {
                            f.iter().position(|v| {
                                v.as_deref()
                                    .map(|s| !encoder.can_encode(s))
                                    .unwrap_or(false)
                            })
                        })
                        .and_then(|i| columns.get(i))
                        .map(|c| c.header.clone())
                        .unwrap_or_default();
                    bad.push((row_no, col));
                }
                return Ok(());
            }
            w.write_all(&bytes)?;
            Ok(())
        };

    for skipped in &dialect.skipped_lines {
        line.clear();
        if !first {
            line.push_str(le);
        }
        first = false;
        line.push_str(skipped);
        emit(&line, 0, None, w)?;
    }
    if header {
        line.clear();
        if !first {
            line.push_str(le);
        }
        first = false;
        for (i, c) in columns.iter().enumerate() {
            if i > 0 {
                line.push(dialect.delimiter);
            }
            write_field(
                &mut line,
                &c.header,
                dialect,
                quote_all || dialect.header_quoted,
            );
        }
        emit(&line, 0, None, w)?;
    }
    rows(&mut |fields: &[Option<String>]| {
        row_no += 1;
        line.clear();
        if !first {
            line.push_str(le);
        }
        first = false;
        for (i, v) in fields.iter().enumerate() {
            if i > 0 {
                line.push(dialect.delimiter);
            }
            let force = quote_all || columns.get(i).map(|c| c.always_quoted).unwrap_or(false);
            match v {
                Some(s) => write_field(&mut line, s, dialect, force),
                None if force => write_field(&mut line, "", dialect, true),
                None => {}
            }
        }
        emit(&line, row_no, Some(fields), w)
    })?;
    if !first && dialect.final_line_ending {
        emit(le, 0, None, w)?;
    }
    if let Some((row, col)) = bad.first() {
        return Err(CoreError::Unencodable {
            encoding: dialect.encoding.clone(),
            cells: bad_count,
            first_row: *row,
            first_column: col.clone(),
        });
    }
    Ok(())
}

/// Runs a query and gives the text of each row to `f`.
fn for_each_text_row(
    conn: &Connection,
    sql: &str,
    n: usize,
    f: &mut dyn FnMut(&[Option<String>]) -> Result<()>,
) -> Result<()> {
    let mut st = conn.prepare(sql)?;
    let mut rows = st.query([])?;
    let mut buf: Vec<Option<String>> = vec![None; n];
    while let Some(r) = rows.next()? {
        for (i, slot) in buf.iter_mut().enumerate() {
            *slot = r.get(i)?;
        }
        f(&buf)?;
    }
    Ok(())
}

/// Writes a new XLSX file from a query. `sql` returns the raw values.
fn write_xlsx(
    conn: &Connection,
    path: &Path,
    sql: &str,
    columns: &[OutColumn],
    sheet_name: Option<&str>,
) -> Result<()> {
    use rust_xlsxwriter::{Format, Workbook};
    let xerr = |e: rust_xlsxwriter::XlsxError| CoreError::Internal(format!("XLSX: {e}"));
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet_with_constant_memory();
    if let Some(name) = sheet_name {
        ws.set_name(name).map_err(xerr)?;
    }
    let bold = Format::new().set_bold();
    let date_fmt = Format::new().set_num_format("yyyy-mm-dd");
    let dt_fmt = Format::new().set_num_format("yyyy-mm-dd hh:mm:ss");
    for (j, c) in columns.iter().enumerate() {
        ws.write_string_with_format(0, j as u16, &c.header, &bold)
            .map_err(xerr)?;
    }
    let mut st = conn.prepare(sql)?;
    let mut rows = st.query([])?;
    let mut r: u32 = 1;
    while let Some(row) = rows.next()? {
        if r >= 1_048_576 {
            return Err(CoreError::InvalidArgument(
                "an Excel sheet can have at most 1,048,575 data rows".into(),
            ));
        }
        for (j, c) in columns.iter().enumerate() {
            let col = j as u16;
            match row.get_ref(j)? {
                ValueRef::Null => {}
                ValueRef::Integer(i) => {
                    if c.ty == LogicalType::Boolean {
                        ws.write_boolean(r, col, i != 0).map_err(xerr)?;
                    } else if i.unsigned_abs() < (1u64 << 53) {
                        ws.write_number(r, col, i as f64).map_err(xerr)?;
                    } else {
                        ws.write_string(r, col, i.to_string()).map_err(xerr)?;
                    }
                }
                ValueRef::Real(f) => {
                    ws.write_number(r, col, f).map_err(xerr)?;
                }
                ValueRef::Text(t) => {
                    let s = String::from_utf8_lossy(t);
                    let written = match c.ty {
                        LogicalType::Date => crate::format::parse_date(&s).and_then(|(y, m, d)| {
                            rust_xlsxwriter::ExcelDateTime::from_ymd(y as u16, m as u8, d as u8)
                                .ok()
                        }),
                        LogicalType::DateTime => {
                            rust_xlsxwriter::ExcelDateTime::parse_from_str(&s.replace(' ', "T"))
                                .ok()
                        }
                        _ => None,
                    };
                    match (written, c.ty) {
                        (Some(dt), LogicalType::Date) => {
                            ws.write_datetime_with_format(r, col, &dt, &date_fmt)
                                .map_err(xerr)?;
                        }
                        (Some(dt), _) => {
                            ws.write_datetime_with_format(r, col, &dt, &dt_fmt)
                                .map_err(xerr)?;
                        }
                        (None, _) => {
                            ws.write_string(r, col, s.as_ref()).map_err(xerr)?;
                        }
                    }
                }
                ValueRef::Blob(_) => {}
            }
        }
        r += 1;
    }
    drop(rows);
    drop(st);
    atomic_write(path, |w| {
        wb.save_to_writer(w).map_err(xerr)?;
        Ok(())
    })
}

fn csv_dialect_from(format: &ExportFormat) -> Option<(Dialect, bool, bool)> {
    match format {
        ExportFormat::Csv {
            encoding,
            bom,
            delimiter,
            line_ending,
            quote_all,
            header,
        } => Some((
            Dialect {
                encoding: encoding.clone(),
                bom: *bom,
                delimiter: *delimiter,
                quote: Some('"'),
                escape: EscapeStyle::Doubled,
                line_ending: *line_ending,
                has_header: *header,
                final_line_ending: true,
                skipped_lines: Vec::new(),
                header_quoted: false,
            },
            *header,
            *quote_all,
        )),
        ExportFormat::Xlsx { .. } => None,
    }
}

impl Inner {
    fn table_select(&self, table: TableId, display: bool) -> Result<String> {
        let t = self.table(table)?;
        let d = self.data(table)?;
        let cols: Vec<String> = d
            .columns
            .iter()
            .map(|c| {
                if display {
                    display_expr(&c.sql_name, c.logical_type)
                } else {
                    format!("t.{}", quote_ident(&c.sql_name))
                }
            })
            .collect();
        Ok(format!(
            "SELECT {} FROM \"{}\" o JOIN {} t ON t.rowid = o.rid ORDER BY o.ord",
            cols.join(", "),
            ord_table(table),
            quote_ident(&t.sql_name)
        ))
    }

    fn table_out_columns(&self, table: TableId) -> Result<Vec<OutColumn>> {
        Ok(self
            .data(table)?
            .columns
            .iter()
            .map(|c| OutColumn {
                header: c.display_name.clone(),
                ty: c.logical_type,
                always_quoted: c.always_quoted,
            })
            .collect())
    }

    /// Writes the working table to a file in a dialect.
    fn write_table_csv(
        &self,
        table: TableId,
        path: &Path,
        dialect: &Dialect,
        header: bool,
        quote_all: bool,
    ) -> Result<()> {
        let sql = self.table_select(table, true)?;
        let cols = self.table_out_columns(table)?;
        let n = cols.len();
        atomic_write(path, |w| {
            write_csv_rows(w, dialect, &cols, header, quote_all, |f| {
                for_each_text_row(&self.conn, &sql, n, f)
            })
        })
    }

    /// After a save: the file is the new base for `diff_table`.
    fn mark_saved(&mut self, table: TableId, path: &Path) -> Result<()> {
        let ord = ord_table(table);
        let base = base_table(table);
        self.conn.execute_batch(&format!(
            "DELETE FROM \"{base}\";
             INSERT INTO \"{base}\"(line, rid) SELECT row_number() OVER (ORDER BY ord), rid FROM \"{ord}\";"
        ))?;
        let fp = Fingerprint::of(path)?;
        let t = self.table_mut(table)?;
        t.externally_modified = false;
        let d = t.data.as_mut().unwrap();
        d.fingerprint = fp;
        d.saved_state = d.state;
        d.version += 1;
        self.emit_changed(table);
        self.emit(CoreEvent::WorkspaceChanged);
        Ok(())
    }

    pub fn save_table(&mut self, table: TableId, force: bool) -> Result<TableMeta> {
        self.ensure_loaded(table)?;
        let src = self.ws.source(&self.table(table)?.source_id)?.clone();
        let d = self.data(table)?;
        if d.save_as_only {
            return Err(CoreError::ReadOnly(
                "Armarium cannot overwrite this file. Use \"Save as\" or \"Export\".".into(),
            ));
        }
        let path = src.path_buf();
        if !force && path.exists() {
            let now = Fingerprint::of(&path)?;
            if now != d.fingerprint {
                self.table_mut(table)?.externally_modified = true;
                return Err(CoreError::ExternalModification(path.display().to_string()));
            }
        }
        let dialect = d.dialect.clone().unwrap_or_default();
        self.write_table_csv(table, &path, &dialect, dialect.has_header, false)?;
        self.mark_saved(table, &path)?;
        self.meta(table)
    }

    pub fn save_table_as(
        &mut self,
        table: TableId,
        path: &Path,
        format: Option<ExportFormat>,
    ) -> Result<TableMeta> {
        self.ensure_loaded(table)?;
        let (sid, kind) = {
            let t = self.table(table)?;
            let s = self.ws.source(&t.source_id)?;
            (s.id.clone(), s.kind)
        };
        let abs = crate::fsutil::absolute(path)?;
        if kind == SourceKind::Excel {
            let format = format.unwrap_or_else(|| ExportFormat::for_path(&abs));
            self.export_table(table, &abs, &format)?;
            self.add_sources(&[abs])?;
            return self.meta(table);
        }
        if crate::workspace::kind_for_path(&abs)? != SourceKind::Csv {
            return Err(CoreError::InvalidArgument(
                "a CSV or TSV table can be saved as a text file only. Use \"Export\" for XLSX."
                    .into(),
            ));
        }
        if let Some(other) = self.ws.find_by_path(&abs.to_string_lossy()) {
            if other.id != sid {
                return Err(CoreError::InvalidArgument(format!(
                    "the file is registered as another source: {}",
                    abs.display()
                )));
            }
        }
        let mut dialect = self.data(table)?.dialect.clone().unwrap_or_default();
        let mut quote_all = false;
        if let Some((d, header, qa)) = format.as_ref().and_then(csv_dialect_from) {
            dialect = Dialect {
                skipped_lines: Vec::new(),
                has_header: header,
                ..d
            };
            quote_all = qa;
        }
        self.write_table_csv(table, &abs, &dialect, dialect.has_header, quote_all)?;
        self.ws.relocate(&sid, &abs)?;
        self.ws.save()?;
        self.data_mut(table)?.dialect = Some(dialect);
        self.mark_saved(table, &abs)?;
        self.meta(table)
    }

    pub fn export_table(
        &mut self,
        table: TableId,
        path: &Path,
        format: &ExportFormat,
    ) -> Result<()> {
        self.ensure_loaded(table)?;
        let cols = self.table_out_columns(table)?;
        match format {
            ExportFormat::Xlsx { sheet_name } => {
                let sql = self.table_select(table, false)?;
                let default_name = self.table(table)?.sheet.clone();
                let name = sheet_name.clone().or(default_name);
                write_xlsx(&self.conn, path, &sql, &cols, name.as_deref())
            }
            other => {
                let (dialect, header, quote_all) = csv_dialect_from(other).unwrap();
                let cols: Vec<OutColumn> = cols
                    .into_iter()
                    .map(|c| OutColumn {
                        always_quoted: false,
                        ..c
                    })
                    .collect();
                let sql = self.table_select(table, true)?;
                let n = cols.len();
                atomic_write(path, |w| {
                    write_csv_rows(w, &dialect, &cols, header, quote_all, |f| {
                        for_each_text_row(&self.conn, &sql, n, f)
                    })
                })
            }
        }
    }

    pub fn export_result(
        &mut self,
        result: ViewId,
        path: &Path,
        format: &ExportFormat,
    ) -> Result<()> {
        let (columns, _) = self.result_meta(result)?;
        let cols: Vec<OutColumn> = columns
            .iter()
            .map(|c| OutColumn {
                header: c.name.clone(),
                ty: c.logical_type,
                always_quoted: false,
            })
            .collect();
        let rt = result_table(result);
        match format {
            ExportFormat::Xlsx { sheet_name } => {
                let list: Vec<String> = (0..cols.len()).map(|i| format!("t.c{i}")).collect();
                let sql = format!(
                    "SELECT {} FROM \"{rt}\" t ORDER BY t.rowid",
                    list.join(", ")
                );
                write_xlsx(&self.conn, path, &sql, &cols, sheet_name.as_deref())
            }
            other => {
                let (dialect, header, quote_all) = csv_dialect_from(other).unwrap();
                let list: Vec<String> = (0..cols.len())
                    .map(|i| display_expr(&format!("c{i}"), cols[i].ty))
                    .collect();
                let sql = format!(
                    "SELECT {} FROM \"{rt}\" t ORDER BY t.rowid",
                    list.join(", ")
                );
                let n = cols.len();
                atomic_write(path, |w| {
                    write_csv_rows(w, &dialect, &cols, header, quote_all, |f| {
                        for_each_text_row(&self.conn, &sql, n, f)
                    })
                })
            }
        }
    }

    /// Compares the working copy with the file (`diff_table`).
    pub fn diff_table(&mut self, table: TableId, sample: usize) -> Result<TableDiff> {
        self.ensure_loaded(table)?;
        let (sid, sheet, name) = {
            let t = self.table(table)?;
            (t.source_id.clone(), t.sheet.clone(), t.sql_name.clone())
        };
        let src = self.ws.source(&sid)?.clone();
        let opts = src.read_options(sheet.as_deref());
        let path = src.path_buf();
        let cmp_id: TableId = u32::MAX;
        let cmp_name = "__arm_cmp";
        let cancel = AtomicBool::new(false);
        let progress = |_: u64, _: Option<u64>| {};
        let ctl = LoadControl {
            cancel: &cancel,
            progress: &progress,
        };
        let loaded = match (src.kind, sheet.as_deref()) {
            (SourceKind::Excel, Some(sh)) => crate::ingest::excel::load_sheet(
                &self.conn, cmp_id, cmp_name, &path, sh, &opts, &ctl,
            )?,
            _ => crate::ingest::load_csv(&self.conn, cmp_id, cmp_name, &path, &opts, &ctl)?,
        };
        let result = (|| -> Result<TableDiff> {
            let d = self.data(table)?;
            let t = quote_ident(&name);
            let base = base_table(table);
            let columns_changed = loaded.columns.len() != d.columns.len()
                || loaded
                    .columns
                    .iter()
                    .zip(&d.columns)
                    .any(|(a, b)| a.sql_name != b.sql_name || a.display_name != b.display_name);
            let q = |sql: &str| -> Result<u64> {
                Ok(self.conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64)
            };
            let deleted = q(&format!("SELECT count(*) FROM \"{base}\" b WHERE NOT EXISTS (SELECT 1 FROM {t} w WHERE w.rowid = b.rid)"))?;
            let inserted = q(&format!("SELECT count(*) FROM {t} w WHERE NOT EXISTS (SELECT 1 FROM \"{base}\" b WHERE b.rid = w.rowid)"))?;
            let common: Vec<&ColumnSchema> = d
                .columns
                .iter()
                .filter(|c| loaded.columns.iter().any(|l| l.sql_name == c.sql_name))
                .collect();
            let mut changed_rows = 0;
            let mut cells = Vec::new();
            if !common.is_empty() {
                let cond = common
                    .iter()
                    .map(|c| {
                        let n = quote_ident(&c.sql_name);
                        format!("CAST(w.{n} AS TEXT) IS NOT CAST(c.{n} AS TEXT)")
                    })
                    .collect::<Vec<_>>()
                    .join(" OR ");
                let from = format!(
                    "FROM \"{base}\" b JOIN {t} w ON w.rowid = b.rid JOIN {cmp_name} c ON c.rowid = b.line WHERE {cond}"
                );
                changed_rows = q(&format!("SELECT count(*) {from}"))?;
                let pairs = common
                    .iter()
                    .map(|c| {
                        let n = quote_ident(&c.sql_name);
                        format!("CAST(c.{n} AS TEXT), CAST(w.{n} AS TEXT)")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut st = self.conn.prepare(&format!(
                    "SELECT b.line, {pairs} {from} ORDER BY b.line LIMIT {}",
                    sample.max(1)
                ))?;
                let mut rows = st.query([])?;
                while let Some(r) = rows.next()? {
                    let line: i64 = r.get(0)?;
                    for (k, c) in common.iter().enumerate() {
                        let before: Option<String> = r.get(1 + 2 * k)?;
                        let after: Option<String> = r.get(2 + 2 * k)?;
                        if before != after && cells.len() < sample {
                            cells.push(CellChange {
                                row_number: line as u64,
                                column: c.sql_name.clone(),
                                before,
                                after,
                            });
                        }
                    }
                }
            }
            Ok(TableDiff {
                inserted_rows: inserted,
                deleted_rows: deleted,
                changed_rows,
                columns_changed,
                sample: cells,
            })
        })();
        crate::ingest::drop_internal(&self.conn, cmp_id, cmp_name)?;
        result
    }
}
