//! Page fetches, grid views (sort and filter), and result tables ([ADR-0014]).

use super::{lock, FetchMeta, Inner, ViewMeta};
use crate::error::{CoreError, Result};
use crate::ingest::ord_table;
use crate::model::*;
use crate::naming::quote_ident;
use rusqlite::types::Value;
use rusqlite::Connection;

pub fn view_table(id: ViewId) -> String {
    format!("__arm_view_{id}")
}

pub fn result_table(id: ViewId) -> String {
    format!("__arm_res_{id}")
}

/// The display text of a column in a SELECT list.
pub fn display_expr(col: &str, ty: LogicalType) -> String {
    let c = quote_ident(col);
    match ty {
        LogicalType::Boolean => format!(
            "CASE t.{c} WHEN 1 THEN 'TRUE' WHEN 0 THEN 'FALSE' ELSE CAST(t.{c} AS TEXT) END"
        ),
        _ => format!("CAST(t.{c} AS TEXT)"),
    }
}

pub fn fetch_table_page(
    conn: &Connection,
    meta: &FetchMeta,
    view: Option<(ViewId, ViewMeta)>,
    offset: u64,
    limit: u32,
) -> Result<RowPage> {
    let t = quote_ident(&meta.sql_name);
    let cols: Vec<String> = meta
        .columns
        .iter()
        .map(|c| display_expr(&c.sql_name, c.logical_type))
        .collect();
    let ncols = cols.len();
    let select_cols = if cols.is_empty() {
        "NULL".to_string()
    } else {
        cols.join(", ")
    };
    let (sql, total, numbered) = match &view {
        None => (
            format!(
                "SELECT o.rid, 0, {select_cols} FROM \"{}\" o JOIN {t} t ON t.rowid = o.rid ORDER BY o.ord LIMIT ?1 OFFSET ?2",
                ord_table(meta.id)
            ),
            meta.row_count,
            false,
        ),
        Some((vid, ViewMeta::Grid { total, .. })) => (
            format!(
                "SELECT v.rid, v.rn, {select_cols} FROM \"{}\" v JOIN {t} t ON t.rowid = v.rid \
                 WHERE v.pos > ?2 AND v.pos <= ?2 + ?1 ORDER BY v.pos",
                view_table(*vid)
            ),
            *total,
            true,
        ),
        Some((_, ViewMeta::Result { .. })) => return Err(CoreError::InvalidArgument("not a table view".into())),
    };
    let mut st = conn.prepare_cached(&sql)?;
    let mut rows = st.query(rusqlite::params![limit as i64, offset as i64])?;
    let mut rids = Vec::new();
    let mut row_numbers = Vec::new();
    let mut data: Vec<Vec<Option<String>>> = vec![Vec::new(); ncols];
    while let Some(r) = rows.next()? {
        let rid: i64 = r.get(0)?;
        rids.push(rid);
        row_numbers.push(if numbered {
            r.get::<_, i64>(1)? as u64
        } else {
            offset + rids.len() as u64
        });
        for (c, col) in data.iter_mut().enumerate() {
            col.push(r.get(c + 2)?);
        }
    }
    let mut notes = Vec::new();
    if !meta.notes.is_empty() {
        for (ri, rid) in rids.iter().enumerate() {
            for (ci, c) in meta.columns.iter().enumerate() {
                if let Some((kind, text)) = meta.notes.get(&(*rid, c.sql_name.clone())) {
                    notes.push(CellNote {
                        row: ri as u32,
                        col: ci as u32,
                        kind: *kind,
                        text: text.clone(),
                    });
                }
            }
        }
    }
    Ok(RowPage {
        offset,
        total,
        rids,
        row_numbers,
        data,
        notes,
    })
}

pub fn fetch_result_page(
    conn: &Connection,
    id: ViewId,
    meta: &ViewMeta,
    offset: u64,
    limit: u32,
) -> Result<RowPage> {
    let ViewMeta::Result { columns, total } = meta else {
        return Err(CoreError::InvalidArgument("not a query result".into()));
    };
    let cols: Vec<String> = (0..columns.len())
        .map(|i| display_expr(&format!("c{i}"), columns[i].logical_type))
        .collect();
    let select_cols = if cols.is_empty() {
        "NULL".to_string()
    } else {
        cols.join(", ")
    };
    let sql = format!(
        "SELECT {select_cols} FROM \"{}\" t WHERE t.rowid > ?2 AND t.rowid <= ?2 + ?1 ORDER BY t.rowid",
        result_table(id)
    );
    let mut st = conn.prepare_cached(&sql)?;
    let mut rows = st.query(rusqlite::params![limit as i64, offset as i64])?;
    let mut data: Vec<Vec<Option<String>>> = vec![Vec::new(); columns.len()];
    let mut row_numbers = Vec::new();
    while let Some(r) = rows.next()? {
        row_numbers.push(offset + row_numbers.len() as u64 + 1);
        for (c, col) in data.iter_mut().enumerate() {
            col.push(r.get(c)?);
        }
    }
    Ok(RowPage {
        offset,
        total: *total,
        rids: Vec::new(),
        row_numbers,
        data,
        notes: Vec::new(),
    })
}

fn typed_cell(v: rusqlite::types::ValueRef<'_>, ty: LogicalType) -> CellValue {
    match (ty, v) {
        (LogicalType::Boolean, rusqlite::types::ValueRef::Integer(i)) => {
            CellValue::Text(if i != 0 {
                "true".into()
            } else {
                "false".into()
            })
        }
        _ => CellValue::from_ref(v),
    }
}

pub fn result_values(
    conn: &Connection,
    id: ViewId,
    meta: &ViewMeta,
    offset: u64,
    limit: u32,
) -> Result<(Vec<ResultColumn>, Vec<Vec<CellValue>>, u64)> {
    let ViewMeta::Result { columns, total } = meta else {
        return Err(CoreError::InvalidArgument("not a query result".into()));
    };
    let mut st = conn.prepare(&format!(
        "SELECT * FROM \"{}\" WHERE rowid > ?2 AND rowid <= ?2 + ?1 ORDER BY rowid",
        result_table(id)
    ))?;
    let mut rows = st.query(rusqlite::params![limit as i64, offset as i64])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let mut row = Vec::with_capacity(columns.len());
        for (i, c) in columns.iter().enumerate() {
            row.push(typed_cell(r.get_ref(i)?, c.logical_type));
        }
        out.push(row);
    }
    Ok((columns.clone(), out, *total))
}

pub fn table_values(
    conn: &Connection,
    meta: &FetchMeta,
    offset: u64,
    limit: u32,
) -> Result<(Vec<ColumnSchema>, Vec<Vec<CellValue>>, u64)> {
    let cols: Vec<String> = meta
        .columns
        .iter()
        .map(|c| format!("t.{}", quote_ident(&c.sql_name)))
        .collect();
    let sql = format!(
        "SELECT {} FROM \"{}\" o JOIN {} t ON t.rowid = o.rid ORDER BY o.ord LIMIT ?1 OFFSET ?2",
        cols.join(", "),
        ord_table(meta.id),
        quote_ident(&meta.sql_name)
    );
    let mut st = conn.prepare(&sql)?;
    let mut rows = st.query(rusqlite::params![limit as i64, offset as i64])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let mut row = Vec::with_capacity(cols.len());
        for (i, c) in meta.columns.iter().enumerate() {
            row.push(typed_cell(r.get_ref(i)?, c.logical_type));
        }
        out.push(row);
    }
    Ok((meta.columns.clone(), out, meta.row_count))
}

impl Inner {
    pub fn create_view(&mut self, table: TableId, spec: ViewSpec) -> Result<(ViewId, u64)> {
        self.ensure_loaded(table)?;
        let id = self.next_view;
        self.next_view += 1;
        self.conn.execute_batch(&format!(
            "CREATE TABLE \"{}\" (pos INTEGER PRIMARY KEY, rid INTEGER NOT NULL, rn INTEGER NOT NULL)",
            view_table(id)
        ))?;
        lock(&self.views).insert(
            id,
            ViewMeta::Grid {
                table,
                spec,
                total: 0,
            },
        );
        let total = self.rebuild_view(id)?;
        Ok((id, total))
    }

    /// Fills a view again from the current data of its table.
    pub fn rebuild_view(&mut self, id: ViewId) -> Result<u64> {
        let Some(ViewMeta::Grid { table, spec, .. }) = lock(&self.views).get(&id).cloned() else {
            return Err(CoreError::NotFound(format!("view {id}")));
        };
        let t = self.table(table)?;
        let d = self.data(table)?;
        let tname = quote_ident(&t.sql_name);
        let find = |name: &str| {
            d.columns
                .iter()
                .find(|c| c.sql_name.eq_ignore_ascii_case(name))
        };
        let mut conds: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        for f in &spec.filters {
            let value = f.value.clone().unwrap_or_default();
            let cols: Vec<&ColumnSchema> = match &f.column {
                Some(c) => match find(c) {
                    Some(col) => vec![col],
                    None => continue,
                },
                None => d.columns.iter().collect(),
            };
            let mut parts = Vec::new();
            for col in cols {
                let c = format!("t.{}", quote_ident(&col.sql_name));
                let p = params.len() + 1;
                let part = match f.op {
                    FilterOp::Contains => {
                        format!("instr(lower(CAST({c} AS TEXT)), lower(?{p})) > 0")
                    }
                    FilterOp::Equals => format!("{c} = ?{p}"),
                    FilterOp::NotEquals => format!("({c} IS NULL OR {c} <> ?{p})"),
                    FilterOp::Greater => format!("{c} > ?{p}"),
                    FilterOp::Less => format!("{c} < ?{p}"),
                    FilterOp::IsEmpty => format!("({c} IS NULL OR {c} = '')"),
                    FilterOp::IsNotEmpty => format!("({c} IS NOT NULL AND {c} <> '')"),
                };
                if !matches!(f.op, FilterOp::IsEmpty | FilterOp::IsNotEmpty) {
                    let v = if col.logical_type.is_numeric() && f.op != FilterOp::Contains {
                        crate::format::parse_input(Some(&value), col.logical_type)
                            .unwrap_or(Value::Text(value.clone()))
                    } else {
                        Value::Text(value.clone())
                    };
                    params.push(v);
                }
                parts.push(part);
            }
            if !parts.is_empty() {
                conds.push(format!("({})", parts.join(" OR ")));
            }
        }
        let mut order: Vec<String> = Vec::new();
        for s in &spec.sort {
            if let Some(col) = find(&s.column) {
                let dir = match s.direction {
                    SortDirection::Asc => "ASC",
                    SortDirection::Desc => "DESC",
                };
                order.push(format!("t.{} {dir}", quote_ident(&col.sql_name)));
            }
        }
        order.push("r.rn".into());
        let where_clause = if conds.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conds.join(" AND "))
        };
        let v = view_table(id);
        let sql = format!(
            "INSERT INTO \"{v}\"(rid, rn) SELECT t.rowid, r.rn FROM {tname} t \
             JOIN (SELECT rid, row_number() OVER (ORDER BY ord) AS rn FROM \"{}\") r ON r.rid = t.rowid \
             {where_clause} ORDER BY {}",
            ord_table(table),
            order.join(", ")
        );
        crate::db::in_transaction(&self.conn, || {
            self.conn.execute_batch(&format!("DELETE FROM \"{v}\""))?;
            self.conn
                .execute(&sql, rusqlite::params_from_iter(params.iter()))?;
            Ok(())
        })?;
        let total: i64 =
            self.conn
                .query_row(&format!("SELECT count(*) FROM \"{v}\""), [], |r| r.get(0))?;
        if let Some(ViewMeta::Grid { total: tt, .. }) = lock(&self.views).get_mut(&id) {
            *tt = total as u64;
        }
        Ok(total as u64)
    }

    /// Fills all views of a table again (after rows are inserted or deleted).
    pub fn rebuild_views_of(&mut self, table: TableId) {
        let ids: Vec<ViewId> = lock(&self.views)
            .iter()
            .filter(|(_, v)| matches!(v, ViewMeta::Grid { table: t, .. } if *t == table))
            .map(|(k, _)| *k)
            .collect();
        for v in ids {
            if let Err(e) = self.rebuild_view(v) {
                tracing::warn!("cannot rebuild view {v}: {e}");
            }
        }
    }

    pub fn close_view(&mut self, id: ViewId) -> Result<()> {
        let removed = lock(&self.views).remove(&id);
        let name = match removed {
            Some(ViewMeta::Grid { .. }) => view_table(id),
            Some(ViewMeta::Result { .. }) => result_table(id),
            None => return Ok(()),
        };
        self.conn
            .execute_batch(&format!("DROP TABLE IF EXISTS \"{name}\""))?;
        Ok(())
    }

    /// Registers a result table that the SQL module created.
    pub fn register_result(&mut self, id: ViewId, columns: Vec<ResultColumn>, total: u64) {
        lock(&self.views).insert(id, ViewMeta::Result { columns, total });
    }

    pub fn alloc_view_id(&mut self) -> ViewId {
        let id = self.next_view;
        self.next_view += 1;
        id
    }

    pub fn result_meta(&self, id: ViewId) -> Result<(Vec<ResultColumn>, u64)> {
        match lock(&self.views).get(&id) {
            Some(ViewMeta::Result { columns, total }) => Ok((columns.clone(), *total)),
            _ => Err(CoreError::NotFound(format!("result {id}"))),
        }
    }
}
