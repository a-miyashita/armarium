//! Edit operations, undo, and redo (06. Editing).

use super::Inner;
use crate::db::changeset::{self, Recorder};
use crate::error::{CoreError, Result};
use crate::format::{convert_value, parse_input};
use crate::ingest::{create_triggers, ord_table};
use crate::model::*;
use crate::naming::{self, quote_ident};
use rusqlite::types::Value;
use std::collections::HashMap;

pub(crate) struct UndoEntry {
    /// The state before and after the operation.
    pub before: u64,
    pub after: u64,
    pub action: UndoAction,
    pub columns_before: Vec<ColumnSchema>,
    pub columns_after: Vec<ColumnSchema>,
    pub bytes: u64,
}

pub(crate) enum UndoAction {
    /// Row changes, recorded by the session extension.
    Changeset(Vec<u8>),
    /// A column was added at the end of the physical column list.
    AddColumn {
        sql_name: String,
        ty: LogicalType,
    },
    /// A column was deleted. The values are kept for undo.
    DropColumn {
        sql_name: String,
        ty: LogicalType,
        physical_index: usize,
        values: Vec<(i64, Value)>,
    },
    RenameColumn {
        from: String,
        to: String,
    },
    ChangeType {
        sql_name: String,
        from: LogicalType,
        to: LogicalType,
        values: Vec<(i64, Value)>,
    },
}

fn values_size(v: &[(i64, Value)]) -> u64 {
    v.iter()
        .map(|(_, v)| {
            16 + match v {
                Value::Text(s) => s.len() as u64,
                Value::Blob(b) => b.len() as u64,
                _ => 8,
            }
        })
        .sum()
}

/// The physical columns of a table: (name, declared type).
fn physical_columns(conn: &rusqlite::Connection, table: &str) -> Result<Vec<(String, String)>> {
    let mut st = conn.prepare(&format!(
        "SELECT name, type FROM pragma_table_info({})",
        sql_string(table)
    ))?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn sql_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Creates the table again with new physical columns and keeps the rowid values.
/// The values of the columns in `fill` come from Rust; other columns are copied.
fn rebuild_table(
    conn: &rusqlite::Connection,
    id: TableId,
    table: &str,
    columns: &[(String, String)],
    fill: &HashMap<String, Vec<(i64, Value)>>,
) -> Result<()> {
    let tmp = "__arm_rebuild";
    conn.execute_batch(&format!("DROP TABLE IF EXISTS {tmp}"))?;
    let defs: Vec<String> = columns
        .iter()
        .map(|(n, t)| format!("{} {t}", quote_ident(n)))
        .collect();
    conn.execute_batch(&format!("CREATE TABLE {tmp} ({}) STRICT", defs.join(", ")))?;
    let old: Vec<(String, String)> = physical_columns(conn, table)?;
    let copy: Vec<String> = columns
        .iter()
        .filter(|(n, _)| !fill.contains_key(n) && old.iter().any(|(o, _)| o == n))
        .map(|(n, _)| quote_ident(n))
        .collect();
    let list = std::iter::once("rowid".to_string())
        .chain(copy.iter().cloned())
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute_batch(&format!(
        "INSERT INTO {tmp}({list}) SELECT {list} FROM {}",
        quote_ident(table)
    ))?;
    for (name, values) in fill {
        let mut st = conn.prepare(&format!(
            "UPDATE {tmp} SET {} = ?1 WHERE rowid = ?2",
            quote_ident(name)
        ))?;
        for (rid, v) in values {
            st.execute(rusqlite::params![v, rid])?;
        }
    }
    conn.execute_batch(&format!(
        "DROP TABLE {t}; ALTER TABLE {tmp} RENAME TO {t};",
        t = quote_ident(table)
    ))?;
    create_triggers(conn, id, table)
}

fn read_column(conn: &rusqlite::Connection, table: &str, col: &str) -> Result<Vec<(i64, Value)>> {
    let mut st = conn.prepare(&format!(
        "SELECT rowid, {} FROM {} WHERE {} IS NOT NULL",
        quote_ident(col),
        quote_ident(table),
        quote_ident(col)
    ))?;
    let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Value>(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Splits TSV text (as Excel copies it) into rows of fields.
pub fn parse_tsv(text: &str) -> Vec<Vec<String>> {
    let text = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    let mut p = crate::ingest::csv::Parser::new(text, '\t', Some('"'), EscapeStyle::Doubled);
    let mut rec = crate::ingest::csv::Record::default();
    let mut out = Vec::new();
    while p.next_record(&mut rec).is_some() {
        out.push(rec.fields().map(str::to_string).collect());
    }
    out
}

impl Inner {
    fn column(&self, table: TableId, name: &str) -> Result<ColumnSchema> {
        self.data(table)?
            .columns
            .iter()
            .find(|c| c.sql_name.eq_ignore_ascii_case(name))
            .cloned()
            .ok_or_else(|| CoreError::NotFound(format!("column {name}")))
    }

    fn check_formula(&self, table: TableId, rid: i64, col: &str) -> Result<()> {
        if let Some((CellNoteKind::Formula, _)) =
            self.data(table)?.notes.get(&(rid, col.to_string()))
        {
            return Err(CoreError::ReadOnly(
                "a formula cell cannot be edited".into(),
            ));
        }
        Ok(())
    }

    /// Runs `f` in a transaction and records the changes of the table as one undo entry.
    pub(crate) fn record_rows<T>(
        &mut self,
        table: TableId,
        allow_no_undo: bool,
        f: impl FnOnce(&Inner) -> Result<T>,
    ) -> Result<T> {
        let name = self.table(table)?.sql_name.clone();
        let ord = ord_table(table);
        let limit = self.cfg.undo_max_bytes;
        let (value, cs) = {
            let rec = Recorder::start(&self.conn, &[&name, &ord])?;
            self.conn.execute_batch("BEGIN IMMEDIATE")?;
            let r = (|| -> Result<(T, Vec<u8>)> {
                let v = f(self)?;
                let cs = rec.changeset()?;
                if cs.len() as u64 > limit && !allow_no_undo {
                    return Err(CoreError::UndoLimitExceeded {
                        bytes: cs.len() as u64,
                    });
                }
                Ok((v, cs))
            })();
            match r {
                Ok(x) => {
                    self.conn.execute_batch("COMMIT")?;
                    x
                }
                Err(e) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(e);
                }
            }
        };
        let columns = self.data(table)?.columns.clone();
        if cs.len() as u64 > limit {
            // The user accepted that this operation cannot be undone.
            let state = self.new_state();
            let d = self.data_mut(table)?;
            d.undo.clear();
            d.redo.clear();
            d.state = state;
        } else if !cs.is_empty() {
            let bytes = cs.len() as u64;
            self.push_undo(
                table,
                UndoAction::Changeset(cs),
                columns.clone(),
                columns,
                bytes,
            )?;
        }
        Ok(value)
    }

    pub(crate) fn push_undo(
        &mut self,
        table: TableId,
        action: UndoAction,
        columns_before: Vec<ColumnSchema>,
        columns_after: Vec<ColumnSchema>,
        bytes: u64,
    ) -> Result<()> {
        let state = self.new_state();
        let (max_ops, max_bytes) = (self.cfg.undo_max_ops, self.cfg.undo_max_bytes);
        let d = self.data_mut(table)?;
        d.undo.push(UndoEntry {
            before: d.state,
            after: state,
            action,
            columns_before,
            columns_after,
            bytes,
        });
        d.redo.clear();
        d.state = state;
        // Discard the oldest entries when the stack is full.
        let mut total: u64 = d.undo.iter().map(|e| e.bytes).sum();
        while d.undo.len() > max_ops || (total > max_bytes && d.undo.len() > 1) {
            let e = d.undo.remove(0);
            total -= e.bytes;
        }
        Ok(())
    }

    /// Updates the counters after a change and notifies the adapters.
    pub(crate) fn after_change(&mut self, table: TableId, rows_changed: bool) -> Result<()> {
        let n = self.count_rows(table)?;
        let d = self.data_mut(table)?;
        d.row_count = n;
        d.version += 1;
        self.publish_fetch(table);
        if rows_changed {
            self.rebuild_views_of(table);
        }
        self.emit_changed(table);
        Ok(())
    }

    pub fn apply_edit(&mut self, table: TableId, op: EditOp) -> Result<EditResult> {
        self.ensure_loaded(table)?;
        let name = self.table(table)?.sql_name.clone();
        let t = quote_ident(&name);
        let mut inserted: Vec<i64> = Vec::new();
        let rows_changed;
        match op {
            EditOp::SetCells { cells } => {
                let mut prepared: Vec<(ColumnSchema, i64, Value)> = Vec::with_capacity(cells.len());
                for c in &cells {
                    let col = self.column(table, &c.column)?;
                    self.check_formula(table, c.rid, &col.sql_name)?;
                    let v = parse_input(c.value.as_deref(), col.logical_type)?;
                    prepared.push((col, c.rid, v));
                }
                self.record_rows(table, false, |i| {
                    for (col, rid, v) in &prepared {
                        let mut st = i.conn.prepare_cached(&format!(
                            "UPDATE {t} SET {} = ?1 WHERE rowid = ?2",
                            quote_ident(&col.sql_name)
                        ))?;
                        st.execute(rusqlite::params![v, rid])?;
                    }
                    Ok(())
                })?;
                rows_changed = false;
            }
            EditOp::InsertRows {
                after_rid,
                count,
                above,
            } => {
                if count == 0 {
                    return Ok(EditResult {
                        table: self.meta(table)?,
                        inserted_rids: vec![],
                    });
                }
                let above = above.unwrap_or(false);
                inserted = self.record_rows(table, false, |i| {
                    let after = if above {
                        row_before(i, table, after_rid)?
                    } else {
                        after_rid
                    };
                    insert_rows(i, table, &t, after, count)
                })?;
                rows_changed = true;
            }
            EditOp::DeleteRows { rids } => {
                self.record_rows(table, false, |i| {
                    let mut st = i
                        .conn
                        .prepare_cached(&format!("DELETE FROM {t} WHERE rowid = ?1"))?;
                    for rid in &rids {
                        st.execute([rid])?;
                    }
                    Ok(())
                })?;
                rows_changed = true;
            }
            EditOp::Paste {
                anchor_rid,
                anchor_column,
                tsv,
                rids,
            } => {
                let grid = parse_tsv(&tsv);
                let cols = self.data(table)?.columns.clone();
                let start = cols
                    .iter()
                    .position(|c| c.sql_name.eq_ignore_ascii_case(&anchor_column))
                    .ok_or_else(|| CoreError::NotFound(format!("column {anchor_column}")))?;
                let mut targets: Vec<i64> = rids.clone();
                if targets.first() != Some(&anchor_rid) {
                    targets.insert(0, anchor_rid);
                }
                let extra = grid.len().saturating_sub(targets.len()) as u32;
                // Convert all values first, so that a type error changes nothing.
                let mut values: Vec<Vec<(String, Value)>> = Vec::new();
                for row in &grid {
                    let mut r = Vec::new();
                    for (j, text) in row.iter().enumerate() {
                        let Some(col) = cols.get(start + j) else {
                            break;
                        };
                        r.push((
                            col.sql_name.clone(),
                            parse_input(Some(text), col.logical_type)?,
                        ));
                    }
                    values.push(r);
                }
                for (ri, row) in values.iter().enumerate() {
                    if let Some(rid) = targets.get(ri) {
                        for (c, _) in row {
                            self.check_formula(table, *rid, c)?;
                        }
                    }
                }
                inserted = self.record_rows(table, false, |i| {
                    let mut new_rids = Vec::new();
                    if extra > 0 {
                        new_rids = insert_rows(i, table, &t, last_rid(i, table)?, extra)?;
                        targets.extend(new_rids.iter().copied());
                    }
                    for (ri, row) in values.iter().enumerate() {
                        let rid = targets[ri];
                        for (c, v) in row {
                            let mut st = i.conn.prepare_cached(&format!(
                                "UPDATE {t} SET {} = ?1 WHERE rowid = ?2",
                                quote_ident(c)
                            ))?;
                            st.execute(rusqlite::params![v, rid])?;
                        }
                    }
                    Ok(new_rids)
                })?;
                rows_changed = !inserted.is_empty();
            }
            EditOp::InsertColumn {
                name: display,
                logical_type,
                position,
            } => {
                let before = self.data(table)?.columns.clone();
                let existing: Vec<&str> = before.iter().map(|c| c.sql_name.as_str()).collect();
                let pos = position
                    .map(|p| (p as usize).min(before.len()))
                    .unwrap_or(before.len());
                let sql_name = naming::column_name(&display, before.len(), &existing);
                self.conn.execute_batch(&format!(
                    "ALTER TABLE {t} ADD COLUMN {} {}",
                    quote_ident(&sql_name),
                    logical_type.sqlite_type()
                ))?;
                let mut after = before.clone();
                after.insert(
                    pos,
                    ColumnSchema {
                        display_name: if display.trim().is_empty() {
                            sql_name.clone()
                        } else {
                            display
                        },
                        sql_name: sql_name.clone(),
                        logical_type,
                        type_source: TypeSource::UserOverride,
                        always_quoted: false,
                    },
                );
                self.data_mut(table)?.columns = after.clone();
                self.push_undo(
                    table,
                    UndoAction::AddColumn {
                        sql_name,
                        ty: logical_type,
                    },
                    before,
                    after,
                    64,
                )?;
                rows_changed = false;
            }
            EditOp::DeleteColumn { column } => {
                let col = self.column(table, &column)?;
                let before = self.data(table)?.columns.clone();
                if before.len() <= 1 {
                    return Err(CoreError::InvalidArgument(
                        "a table must have at least one column".into(),
                    ));
                }
                let phys = physical_columns(&self.conn, &name)?;
                let physical_index = phys
                    .iter()
                    .position(|(n, _)| *n == col.sql_name)
                    .unwrap_or(phys.len());
                let values = read_column(&self.conn, &name, &col.sql_name)?;
                self.conn.execute_batch(&format!(
                    "ALTER TABLE {t} DROP COLUMN {}",
                    quote_ident(&col.sql_name)
                ))?;
                let after: Vec<ColumnSchema> = before
                    .iter()
                    .filter(|c| c.sql_name != col.sql_name)
                    .cloned()
                    .collect();
                self.data_mut(table)?.columns = after.clone();
                self.data_mut(table)?
                    .notes
                    .retain(|(_, c), _| *c != col.sql_name);
                let bytes = values_size(&values) + 64;
                self.push_undo(
                    table,
                    UndoAction::DropColumn {
                        sql_name: col.sql_name,
                        ty: col.logical_type,
                        physical_index,
                        values,
                    },
                    before,
                    after,
                    bytes,
                )?;
                rows_changed = true;
            }
            EditOp::RenameColumn { column, to } => {
                let col = self.column(table, &column)?;
                let before = self.data(table)?.columns.clone();
                if to.trim().is_empty() {
                    return Err(CoreError::InvalidArgument(
                        "the column name is empty".into(),
                    ));
                }
                let others: Vec<&str> = before
                    .iter()
                    .filter(|c| c.sql_name != col.sql_name)
                    .map(|c| c.sql_name.as_str())
                    .collect();
                let new_sql = naming::column_name(&to, 0, &others);
                if new_sql != col.sql_name {
                    self.conn.execute_batch(&format!(
                        "ALTER TABLE {t} RENAME COLUMN {} TO {}",
                        quote_ident(&col.sql_name),
                        quote_ident(&new_sql)
                    ))?;
                }
                let mut after = before.clone();
                for c in after.iter_mut().filter(|c| c.sql_name == col.sql_name) {
                    c.display_name = to.clone();
                    c.sql_name = new_sql.clone();
                }
                self.rename_notes(table, &col.sql_name, &new_sql)?;
                self.data_mut(table)?.columns = after.clone();
                self.push_undo(
                    table,
                    UndoAction::RenameColumn {
                        from: col.sql_name,
                        to: new_sql,
                    },
                    before,
                    after,
                    64,
                )?;
                rows_changed = false;
            }
            EditOp::ChangeColumnType {
                column,
                logical_type,
            } => {
                let col = self.column(table, &column)?;
                let before = self.data(table)?.columns.clone();
                let old_values = read_column(&self.conn, &name, &col.sql_name)?;
                let new_values = convert_all(&old_values, logical_type)?;
                self.change_type(table, &name, &col.sql_name, logical_type, new_values)?;
                let mut after = before.clone();
                for c in after.iter_mut().filter(|c| c.sql_name == col.sql_name) {
                    c.logical_type = logical_type;
                    c.type_source = TypeSource::UserOverride;
                }
                self.data_mut(table)?.columns = after.clone();
                let bytes = values_size(&old_values) + 64;
                self.push_undo(
                    table,
                    UndoAction::ChangeType {
                        sql_name: col.sql_name,
                        from: col.logical_type,
                        to: logical_type,
                        values: old_values,
                    },
                    before,
                    after,
                    bytes,
                )?;
                rows_changed = false;
            }
        }
        self.after_change(table, rows_changed)?;
        Ok(EditResult {
            table: self.meta(table)?,
            inserted_rids: inserted,
        })
    }

    fn rename_notes(&mut self, table: TableId, from: &str, to: &str) -> Result<()> {
        let d = self.data_mut(table)?;
        if d.notes.is_empty() {
            return Ok(());
        }
        let moved: Vec<((i64, String), (CellNoteKind, String))> = d
            .notes
            .iter()
            .filter(|((_, c), _)| c == from)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for ((rid, _), v) in moved {
            d.notes.remove(&(rid, from.to_string()));
            d.notes.insert((rid, to.to_string()), v);
        }
        Ok(())
    }

    fn change_type(
        &mut self,
        table: TableId,
        name: &str,
        col: &str,
        ty: LogicalType,
        values: Vec<(i64, Value)>,
    ) -> Result<()> {
        let phys: Vec<(String, String)> = physical_columns(&self.conn, name)?
            .into_iter()
            .map(|(n, t)| {
                if n == col {
                    (n, ty.sqlite_type().to_string())
                } else {
                    (n, t)
                }
            })
            .collect();
        let mut fill = HashMap::new();
        fill.insert(col.to_string(), values);
        crate::db::in_transaction(&self.conn, || {
            rebuild_table(&self.conn, table, name, &phys, &fill)
        })
    }

    fn apply_undo_action(&mut self, table: TableId, entry: &UndoEntry, undo: bool) -> Result<()> {
        let name = self.table(table)?.sql_name.clone();
        let t = quote_ident(&name);
        match &entry.action {
            UndoAction::Changeset(cs) => {
                let data = if undo {
                    changeset::invert(cs)?
                } else {
                    cs.clone()
                };
                crate::db::in_transaction(&self.conn, || changeset::apply(&self.conn, &data))?;
            }
            UndoAction::AddColumn { sql_name, ty } => {
                if undo {
                    self.conn.execute_batch(&format!(
                        "ALTER TABLE {t} DROP COLUMN {}",
                        quote_ident(sql_name)
                    ))?;
                } else {
                    self.conn.execute_batch(&format!(
                        "ALTER TABLE {t} ADD COLUMN {} {}",
                        quote_ident(sql_name),
                        ty.sqlite_type()
                    ))?;
                }
            }
            UndoAction::DropColumn {
                sql_name,
                ty,
                physical_index,
                values,
            } => {
                if undo {
                    let mut phys = physical_columns(&self.conn, &name)?;
                    let idx = (*physical_index).min(phys.len());
                    phys.insert(idx, (sql_name.clone(), ty.sqlite_type().to_string()));
                    let mut fill = HashMap::new();
                    fill.insert(sql_name.clone(), values.clone());
                    crate::db::in_transaction(&self.conn, || {
                        rebuild_table(&self.conn, table, &name, &phys, &fill)
                    })?;
                } else {
                    self.conn.execute_batch(&format!(
                        "ALTER TABLE {t} DROP COLUMN {}",
                        quote_ident(sql_name)
                    ))?;
                }
            }
            UndoAction::RenameColumn { from, to } => {
                let (a, b) = if undo { (to, from) } else { (from, to) };
                if a != b {
                    self.conn.execute_batch(&format!(
                        "ALTER TABLE {t} RENAME COLUMN {} TO {}",
                        quote_ident(a),
                        quote_ident(b)
                    ))?;
                }
                self.rename_notes(table, a, b)?;
            }
            UndoAction::ChangeType {
                sql_name,
                from,
                to,
                values,
            } => {
                if undo {
                    self.change_type(table, &name, sql_name, *from, values.clone())?;
                } else {
                    let conv = convert_all(values, *to)?;
                    self.change_type(table, &name, sql_name, *to, conv)?;
                }
            }
        }
        Ok(())
    }

    pub fn undo(&mut self, table: TableId) -> Result<TableMeta> {
        let entry = self
            .data_mut(table)?
            .undo
            .pop()
            .ok_or_else(|| CoreError::InvalidArgument("there is nothing to undo".into()))?;
        if let Err(e) = self.apply_undo_action(table, &entry, true) {
            self.data_mut(table)?.undo.push(entry);
            return Err(e);
        }
        let d = self.data_mut(table)?;
        d.state = entry.before;
        d.columns = entry.columns_before.clone();
        d.redo.push(entry);
        self.after_change(table, true)?;
        self.meta(table)
    }

    pub fn redo(&mut self, table: TableId) -> Result<TableMeta> {
        let entry = self
            .data_mut(table)?
            .redo
            .pop()
            .ok_or_else(|| CoreError::InvalidArgument("there is nothing to redo".into()))?;
        if let Err(e) = self.apply_undo_action(table, &entry, false) {
            self.data_mut(table)?.redo.push(entry);
            return Err(e);
        }
        let d = self.data_mut(table)?;
        d.state = entry.after;
        d.columns = entry.columns_after.clone();
        d.undo.push(entry);
        self.after_change(table, true)?;
        self.meta(table)
    }
}

fn convert_all(values: &[(i64, Value)], ty: LogicalType) -> Result<Vec<(i64, Value)>> {
    values
        .iter()
        .map(|(rid, v)| {
            convert_value(v, ty)
                .map(|nv| (*rid, nv))
                .map_err(|e| match e {
                    CoreError::TypeMismatch(m) => {
                        CoreError::TypeMismatch(format!("{m} (row id {rid})"))
                    }
                    other => other,
                })
        })
        .collect()
}

/// The row before a row in file order (`None` for the first row).
fn row_before(i: &Inner, table: TableId, rid: Option<i64>) -> Result<Option<i64>> {
    let Some(rid) = rid else { return Ok(None) };
    let ord = ord_table(table);
    Ok(i.conn
        .query_row(
            &format!(
                "SELECT rid FROM \"{ord}\" WHERE ord < (SELECT ord FROM \"{ord}\" WHERE rid = ?1) ORDER BY ord DESC LIMIT 1"
            ),
            [rid],
            |r| r.get(0),
        )
        .ok())
}

fn last_rid(i: &Inner, table: TableId) -> Result<Option<i64>> {
    Ok(i.conn
        .query_row(
            &format!(
                "SELECT rid FROM \"{}\" ORDER BY ord DESC LIMIT 1",
                ord_table(table)
            ),
            [],
            |r| r.get(0),
        )
        .ok())
}

/// Inserts empty rows after a row (or at the start) and sets their order.
fn insert_rows(
    i: &Inner,
    table: TableId,
    t: &str,
    after_rid: Option<i64>,
    count: u32,
) -> Result<Vec<i64>> {
    let ord = ord_table(table);
    let q = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> Result<Option<f64>> {
        Ok(i.conn.query_row(sql, p, |r| r.get::<_, Option<f64>>(0))?)
    };
    let (lo, hi) = match after_rid {
        Some(rid) => {
            let a = q(
                &format!("SELECT ord FROM \"{ord}\" WHERE rid = ?1"),
                &[&rid],
            )?
            .ok_or_else(|| CoreError::NotFound(format!("row {rid}")))?;
            let b = q(
                &format!("SELECT min(ord) FROM \"{ord}\" WHERE ord > ?1"),
                &[&a],
            )?;
            (a, b)
        }
        None => {
            let b = q(&format!("SELECT min(ord) FROM \"{ord}\""), &[])?;
            (b.map(|b| b - 1.0).unwrap_or(0.0), b)
        }
    };
    let mut rids = Vec::with_capacity(count as usize);
    {
        let mut st = i
            .conn
            .prepare_cached(&format!("INSERT INTO {t} DEFAULT VALUES"))?;
        for _ in 0..count {
            st.execute([])?;
            rids.push(i.conn.last_insert_rowid());
        }
    }
    if let Some(hi) = hi {
        let step = (hi - lo) / (count as f64 + 1.0);
        let renumber = step < 1e-6;
        if renumber {
            // Assign new order values to all rows first.
            i.conn.execute_batch(&format!(
                "DROP TABLE IF EXISTS __arm_renum;
                 CREATE TABLE __arm_renum AS
                   SELECT rid, row_number() OVER (ORDER BY ord) * 1024.0 AS n FROM \"{ord}\";
                 UPDATE \"{ord}\" SET ord = (SELECT n FROM __arm_renum r WHERE r.rid = \"{ord}\".rid);
                 DROP TABLE __arm_renum;"
            ))?;
            let new_ids = rids.clone();
            return reorder_after(i, table, after_rid, &new_ids);
        }
        let mut st = i
            .conn
            .prepare_cached(&format!("UPDATE \"{ord}\" SET ord = ?1 WHERE rid = ?2"))?;
        for (k, rid) in rids.iter().enumerate() {
            st.execute(rusqlite::params![lo + step * (k as f64 + 1.0), rid])?;
        }
    }
    Ok(rids)
}

/// After a renumbering, moves the new rows to their place again.
fn reorder_after(
    i: &Inner,
    table: TableId,
    after_rid: Option<i64>,
    rids: &[i64],
) -> Result<Vec<i64>> {
    let ord = ord_table(table);
    // The new rows were at the end; their order values are now the largest.
    let lo: f64 = match after_rid {
        Some(rid) => i.conn.query_row(
            &format!("SELECT ord FROM \"{ord}\" WHERE rid = ?1"),
            [rid],
            |r| r.get(0),
        )?,
        None => 1.0,
    };
    let placeholders = rids
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let hi: Option<f64> = i.conn.query_row(
        &format!("SELECT min(ord) FROM \"{ord}\" WHERE ord > ?1 AND rid NOT IN ({placeholders})"),
        [lo],
        |r| r.get(0),
    )?;
    let hi = hi.unwrap_or(lo + 1024.0);
    let step = (hi - lo) / (rids.len() as f64 + 1.0);
    let mut st = i
        .conn
        .prepare_cached(&format!("UPDATE \"{ord}\" SET ord = ?1 WHERE rid = ?2"))?;
    for (k, rid) in rids.iter().enumerate() {
        st.execute(rusqlite::params![lo + step * (k as f64 + 1.0), rid])?;
    }
    Ok(rids.to_vec())
}
