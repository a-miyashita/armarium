//! User SQL: sandbox, statement splitting, lazy loading, and results (07. SQL).

use crate::engine::Inner;
use crate::error::{CoreError, Result};
use crate::model::*;
use rusqlite::ffi;
use serde::Serialize;
use std::time::Instant;
use ts_rs::TS;

#[derive(Debug, Clone, Default)]
pub struct SqlOptions {
    /// Refuse DML (the MCP `query` tool and `--read-only`).
    pub read_only: bool,
    /// Commit a DML statement although its change is larger than the undo limit.
    pub allow_no_undo: bool,
}

/// The results of a run. Armarium stops at the first error.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SqlRun {
    pub results: Vec<StatementResult>,
    pub error: Option<SqlError>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SqlError {
    pub code: String,
    pub message: String,
    /// The index of the statement that failed.
    pub statement_index: u32,
    /// The start of the statement in the text, in UTF-16 code units (for the editor).
    pub offset: u32,
    /// The start of the statement in the text, in bytes.
    pub byte_offset: u32,
}

/// Runs one or more statements, separated by `;`.
pub(crate) fn run(inner: &mut Inner, text: &str, opts: &SqlOptions) -> SqlRun {
    let mut results = Vec::new();
    if let Err(e) = preload(inner, text) {
        return SqlRun {
            results,
            error: Some(SqlError {
                code: e.code().into(),
                message: e.to_string(),
                statement_index: 0,
                offset: 0,
                byte_offset: 0,
            }),
        };
    }
    let mut pos = 0usize;
    let mut index = 0u32;
    while pos < text.len() {
        let started = Instant::now();
        match run_next(inner, text, pos, opts) {
            Ok(None) => break,
            Ok(Some((len, mut res))) => {
                let ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
                if let Some(r) = res.as_mut() {
                    match r {
                        StatementResult::Rows { elapsed_ms, .. }
                        | StatementResult::Changed { elapsed_ms, .. } => *elapsed_ms = ms,
                    }
                }
                results.extend(res);
                pos += len;
                index += 1;
            }
            Err(e) => {
                inner.sandbox.end();
                let start = pos + leading_trivia(&text[pos..]);
                return SqlRun {
                    results,
                    error: Some(SqlError {
                        code: e.code().into(),
                        message: e.to_string(),
                        statement_index: index,
                        offset: text[..start].encode_utf16().count() as u32,
                        byte_offset: start as u32,
                    }),
                };
            }
        }
    }
    inner.sandbox.end();
    SqlRun {
        results,
        error: None,
    }
}

/// The length of whitespace and comments at the start of the text.
fn leading_trivia(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = 0;
    loop {
        while i < b.len() && (b[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if b[i..].starts_with(b"--") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            match s[i + 2..].find("*/") {
                Some(e) => i += e + 4,
                None => return b.len(),
            }
        } else {
            return i;
        }
    }
}

/// Loads the tables whose SQL names occur in the text ([07. SQL], section 3.2).
fn preload(inner: &mut Inner, text: &str) -> Result<()> {
    let tokens = identifiers(text);
    let ids: Vec<TableId> = inner
        .tables
        .values()
        .filter(|t| {
            t.data.is_none()
                && tokens
                    .iter()
                    .any(|tok| tok.eq_ignore_ascii_case(&t.sql_name))
        })
        .map(|t| t.id)
        .collect();
    for id in ids {
        inner.ensure_loaded(id)?;
    }
    Ok(())
}

/// Identifiers in SQL text (plain and quoted), without strings and comments.
pub fn identifiers(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if c == '\'' {
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            i += 1;
        } else if c == '"' || c == '`' || c == '[' {
            let close = if c == '[' { ']' } else { c };
            let mut s = String::new();
            i += 1;
            while i < chars.len() {
                if chars[i] == close {
                    if close != ']' && chars.get(i + 1) == Some(&close) {
                        s.push(close);
                        i += 2;
                        continue;
                    }
                    break;
                }
                s.push(chars[i]);
                i += 1;
            }
            i += 1;
            out.push(s);
        } else if c.is_alphabetic() || c == '_' {
            let mut s = String::new();
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
            {
                s.push(chars[i]);
                i += 1;
            }
            out.push(s);
        } else {
            i += 1;
        }
    }
    out
}

/// Finds the length of the next statement with the SQLite parser (the authorizer is enabled).
/// Returns `None` if only whitespace and comments remain.
fn split_next(conn: &rusqlite::Connection, rest: &str) -> Result<Option<(usize, bool)>> {
    let db = unsafe { conn.handle() };
    let mut stmt: *mut ffi::sqlite3_stmt = std::ptr::null_mut();
    let mut tail: *const std::ffi::c_char = std::ptr::null();
    let ptr = rest.as_ptr() as *const std::ffi::c_char;
    let rc = unsafe { ffi::sqlite3_prepare_v2(db, ptr, rest.len() as i32, &mut stmt, &mut tail) };
    let len = if tail.is_null() {
        rest.len()
    } else {
        (tail as usize) - (ptr as usize)
    };
    if rc != ffi::SQLITE_OK {
        let msg = unsafe { std::ffi::CStr::from_ptr(ffi::sqlite3_errmsg(db)) }
            .to_string_lossy()
            .into_owned();
        if !stmt.is_null() {
            unsafe { ffi::sqlite3_finalize(stmt) };
        }
        return Err(
            if rc == ffi::SQLITE_AUTH || msg.contains("not authorized") {
                CoreError::SqlForbidden(msg)
            } else if rc == ffi::SQLITE_INTERRUPT {
                CoreError::Cancelled
            } else {
                CoreError::Sql(msg)
            },
        );
    }
    if stmt.is_null() {
        return Ok(None);
    }
    unsafe { ffi::sqlite3_finalize(stmt) };
    Ok(Some((len, true)))
}

fn working_names(inner: &Inner) -> Vec<String> {
    inner
        .tables
        .values()
        .filter(|t| t.data.is_some())
        .map(|t| t.sql_name.clone())
        .collect()
}

fn missing_table(msg: &str) -> Option<String> {
    let rest = msg.split("no such table: ").nth(1)?;
    let name = rest.trim();
    let name = name.strip_prefix("main.").unwrap_or(name);
    Some(name.to_string())
}

/// Prepares and runs the next statement. Returns the consumed length and the result.
fn run_next(
    inner: &mut Inner,
    text: &str,
    pos: usize,
    opts: &SqlOptions,
) -> Result<Option<(usize, Option<StatementResult>)>> {
    let rest = &text[pos..];
    // Prepare with the authorizer; load tables that are missing and try again.
    let mut attempts = 0;
    let len = loop {
        inner.sandbox.begin(working_names(inner), opts.read_only);
        let r = split_next(&inner.conn, rest);
        let report = inner.sandbox.report();
        inner.sandbox.end();
        match r {
            Ok(None) => return Ok(None),
            Ok(Some((len, _))) => break len,
            Err(CoreError::Sql(msg)) if attempts < 16 => {
                let Some(name) = missing_table(&msg) else {
                    return Err(CoreError::Sql(msg));
                };
                match inner
                    .tables
                    .values()
                    .find(|t| t.sql_name.eq_ignore_ascii_case(&name) && t.data.is_none())
                {
                    Some(t) => {
                        let id = t.id;
                        inner.ensure_loaded(id)?;
                        attempts += 1;
                    }
                    None => return Err(CoreError::Sql(msg)),
                }
            }
            Err(CoreError::SqlForbidden(msg)) => {
                return Err(CoreError::SqlForbidden(match report.denied {
                    Some(d) => format!("{d} is not allowed"),
                    None => msg,
                }))
            }
            Err(e) => return Err(e),
        }
    };
    let raw = &rest[..len];
    let stmt_text = raw[leading_trivia(raw)..]
        .trim_end()
        .trim_end_matches(';')
        .trim_end()
        .to_string();
    if stmt_text.is_empty() {
        return Ok(Some((len, None)));
    }
    let result = execute(inner, &stmt_text, opts)?;
    Ok(Some((len, Some(result))))
}

/// Information about a prepared statement.
struct Prepared {
    readonly: bool,
    explain: bool,
    names: Vec<String>,
    origin_types: Vec<Option<LogicalType>>,
    writes: Vec<String>,
}

fn inspect(inner: &Inner, sql: &str, read_only: bool) -> Result<Prepared> {
    inner.sandbox.begin(working_names(inner), read_only);
    let r = (|| -> Result<Prepared> {
        let st = inner.conn.prepare(sql)?;
        let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
        let mut origin_types = Vec::with_capacity(names.len());
        for i in 0..names.len() {
            let ty = match st.column_metadata(i) {
                Ok(Some((_, table, column, ..))) => {
                    let (table, column) = (table.to_string_lossy(), column.to_string_lossy());
                    inner
                        .tables
                        .values()
                        .find(|t| t.sql_name.eq_ignore_ascii_case(&table))
                        .and_then(|t| t.data.as_ref())
                        .and_then(|d| {
                            d.columns
                                .iter()
                                .find(|c| c.sql_name.eq_ignore_ascii_case(&column))
                        })
                        .map(|c| c.logical_type)
                }
                _ => None,
            };
            origin_types.push(ty);
        }
        Ok(Prepared {
            readonly: st.readonly(),
            explain: st.is_explain() > 0,
            names,
            origin_types,
            writes: Vec::new(),
        })
    })();
    let report = inner.sandbox.report();
    inner.sandbox.end();
    let mut p = r.map_err(|e| match (e, &report.denied) {
        (CoreError::SqlForbidden(_), Some(d)) => {
            CoreError::SqlForbidden(format!("{d} is not allowed"))
        }
        (e, _) => e,
    })?;
    p.writes = report.writes;
    Ok(p)
}

fn result_columns(names: &[String], types: &[LogicalType]) -> Vec<ResultColumn> {
    names
        .iter()
        .zip(types)
        .map(|(n, t)| ResultColumn {
            name: n.clone(),
            logical_type: *t,
        })
        .collect()
}

fn create_result_table(inner: &Inner, id: ViewId, ncols: usize) -> Result<String> {
    let name = crate::engine::result_table_name(id);
    let cols: Vec<String> = (0..ncols.max(1)).map(|i| format!("c{i}")).collect();
    inner
        .conn
        .execute_batch(&format!("CREATE TABLE \"{name}\" ({})", cols.join(", ")))?;
    Ok(name)
}

/// Decides the logical type of each result column: the type of the original column,
/// or the storage types of the values.
fn result_types(
    inner: &Inner,
    table: &str,
    origin: &[Option<LogicalType>],
) -> Result<Vec<LogicalType>> {
    if origin.iter().all(|o| o.is_some()) {
        return Ok(origin.iter().map(|o| o.unwrap()).collect());
    }
    let mut parts = Vec::new();
    for i in 0..origin.len() {
        parts.push(format!(
            "max(typeof(c{i}) IN ('text','blob')), max(typeof(c{i}) = 'real'), max(typeof(c{i}) = 'integer')"
        ));
    }
    if parts.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!("SELECT {} FROM \"{table}\"", parts.join(", "));
    inner
        .conn
        .query_row(&sql, [], |r| {
            let mut out = Vec::new();
            for (i, o) in origin.iter().enumerate() {
                if let Some(t) = o {
                    out.push(*t);
                    continue;
                }
                let text: Option<i64> = r.get(i * 3)?;
                let real: Option<i64> = r.get(i * 3 + 1)?;
                let int: Option<i64> = r.get(i * 3 + 2)?;
                out.push(if text == Some(1) {
                    LogicalType::Text
                } else if real == Some(1) {
                    LogicalType::Real
                } else if int == Some(1) {
                    LogicalType::Integer
                } else {
                    LogicalType::Text
                });
            }
            Ok(out)
        })
        .map_err(Into::into)
}

/// Steps a statement in Rust and stores its rows in a new result table.
fn materialize(
    inner: &Inner,
    sql: &str,
    read_only: bool,
    id: ViewId,
    ncols: usize,
) -> Result<(u64, u64)> {
    let table = create_result_table(inner, id, ncols)?;
    inner.sandbox.begin(working_names(inner), read_only);
    let r = (|| -> Result<(u64, u64)> {
        let mut st = inner.conn.prepare(sql)?;
        inner.sandbox.end();
        let mut rows: Vec<Vec<rusqlite::types::Value>> = Vec::new();
        {
            let mut q = st.raw_query();
            while let Some(r) = q.next()? {
                let mut row = Vec::with_capacity(ncols);
                for i in 0..ncols {
                    row.push(r.get::<_, rusqlite::types::Value>(i)?);
                }
                rows.push(row);
            }
        }
        let changed = inner.conn.changes();
        let params: Vec<String> = (0..ncols.max(1)).map(|i| format!("?{}", i + 1)).collect();
        let mut ins = inner.conn.prepare(&format!(
            "INSERT INTO \"{table}\" VALUES ({})",
            params.join(",")
        ))?;
        for row in &rows {
            if ncols == 0 {
                ins.execute([rusqlite::types::Value::Null])?;
            } else {
                ins.execute(rusqlite::params_from_iter(row.iter()))?;
            }
        }
        Ok((rows.len() as u64, changed))
    })();
    inner.sandbox.end();
    if r.is_err() {
        let _ = inner
            .conn
            .execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\""));
    }
    r
}

fn execute(inner: &mut Inner, sql: &str, opts: &SqlOptions) -> Result<StatementResult> {
    let p = inspect(inner, sql, opts.read_only)?;
    if p.readonly {
        if p.names.is_empty() {
            return Err(CoreError::SqlForbidden(
                "this statement is not allowed".into(),
            ));
        }
        let id = inner.alloc_view_id();
        let total = if p.explain {
            materialize(inner, sql, true, id, p.names.len())?.0
        } else {
            let table = create_result_table(inner, id, p.names.len())?;
            // The authorizer checks the statement when it is prepared. The session
            // extension runs its own SQL while statements run, so the authorizer is
            // disabled before the statement runs.
            inner.sandbox.begin(working_names(inner), true);
            inner.sandbox.allow_result_target(Some(table.clone()));
            let st = inner
                .conn
                .prepare(&format!(
                    "INSERT INTO \"{table}\" SELECT * FROM (\n{sql}\n)"
                ))
                .map_err(CoreError::from);
            inner.sandbox.end();
            let r = st.and_then(|mut st| st.raw_execute().map_err(CoreError::from));
            match r {
                Ok(n) => n as u64,
                Err(e) => {
                    let _ = inner
                        .conn
                        .execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\""));
                    return Err(e);
                }
            }
        };
        let table = crate::engine::result_table_name(id);
        let types = result_types(inner, &table, &p.origin_types)?;
        let columns = result_columns(&p.names, &types);
        inner.register_result(id, columns.clone(), total);
        return Ok(StatementResult::Rows {
            sql: sql.to_string(),
            result_id: id,
            columns,
            row_count: total,
            elapsed_ms: 0,
        });
    }

    // DML: exactly one working table must change.
    if p.writes.len() != 1 {
        return Err(CoreError::SqlForbidden(if p.writes.is_empty() {
            "this statement is not allowed".into()
        } else {
            "a statement can change only one table".into()
        }));
    }
    let target = inner.find_table(&p.writes[0])?;
    let names = p.names.clone();
    let ncols = names.len();
    let read_only = opts.read_only;
    let (changed, returning) = inner.record_rows(target, opts.allow_no_undo, |i| {
        i.sandbox.begin(working_names(i), read_only);
        let r = (|| -> Result<(u64, Vec<Vec<rusqlite::types::Value>>)> {
            let st = i.conn.prepare(sql);
            i.sandbox.end();
            let mut st = st?;
            let mut rows = Vec::new();
            {
                let mut q = st.raw_query();
                while let Some(r) = q.next()? {
                    let mut row = Vec::with_capacity(ncols);
                    for k in 0..ncols {
                        row.push(r.get::<_, rusqlite::types::Value>(k)?);
                    }
                    rows.push(row);
                }
            }
            Ok((i.conn.changes(), rows))
        })();
        i.sandbox.end();
        r
    })?;
    inner.after_change(target, true)?;
    let mut result_id = None;
    let mut columns = Vec::new();
    let row_count = returning.len() as u64;
    if ncols > 0 {
        let id = inner.alloc_view_id();
        let table = create_result_table(inner, id, ncols)?;
        let params: Vec<String> = (0..ncols).map(|k| format!("?{}", k + 1)).collect();
        {
            let mut ins = inner.conn.prepare(&format!(
                "INSERT INTO \"{table}\" VALUES ({})",
                params.join(",")
            ))?;
            for row in &returning {
                ins.execute(rusqlite::params_from_iter(row.iter()))?;
            }
        }
        let types = result_types(inner, &table, &p.origin_types)?;
        columns = result_columns(&names, &types);
        inner.register_result(id, columns.clone(), row_count);
        result_id = Some(id);
    }
    let sql_name = inner.table(target)?.sql_name.clone();
    Ok(StatementResult::Changed {
        sql: sql.to_string(),
        table_id: target,
        sql_name,
        changed_rows: changed,
        result_id,
        columns,
        row_count,
        elapsed_ms: 0,
    })
}

/// SQLite keywords. A name that is a keyword needs double quotes in SQL.
const KEYWORDS: &[&str] = &[
    "abort",
    "action",
    "add",
    "after",
    "all",
    "alter",
    "always",
    "analyze",
    "and",
    "as",
    "asc",
    "attach",
    "autoincrement",
    "before",
    "begin",
    "between",
    "by",
    "cascade",
    "case",
    "cast",
    "check",
    "collate",
    "column",
    "commit",
    "conflict",
    "constraint",
    "create",
    "cross",
    "current",
    "current_date",
    "current_time",
    "current_timestamp",
    "database",
    "default",
    "deferrable",
    "deferred",
    "delete",
    "desc",
    "detach",
    "distinct",
    "do",
    "drop",
    "each",
    "else",
    "end",
    "escape",
    "except",
    "exclude",
    "exclusive",
    "exists",
    "explain",
    "fail",
    "filter",
    "first",
    "following",
    "for",
    "foreign",
    "from",
    "full",
    "generated",
    "glob",
    "group",
    "groups",
    "having",
    "if",
    "ignore",
    "immediate",
    "in",
    "index",
    "indexed",
    "initially",
    "inner",
    "insert",
    "instead",
    "intersect",
    "into",
    "is",
    "isnull",
    "join",
    "key",
    "last",
    "left",
    "like",
    "limit",
    "match",
    "materialized",
    "natural",
    "no",
    "not",
    "nothing",
    "notnull",
    "null",
    "nulls",
    "of",
    "offset",
    "on",
    "or",
    "order",
    "others",
    "outer",
    "over",
    "partition",
    "plan",
    "pragma",
    "preceding",
    "primary",
    "query",
    "raise",
    "range",
    "recursive",
    "references",
    "regexp",
    "reindex",
    "release",
    "rename",
    "replace",
    "restrict",
    "returning",
    "right",
    "rollback",
    "row",
    "rows",
    "savepoint",
    "select",
    "set",
    "table",
    "temp",
    "temporary",
    "then",
    "ties",
    "to",
    "transaction",
    "trigger",
    "unbounded",
    "union",
    "unique",
    "update",
    "using",
    "vacuum",
    "values",
    "view",
    "virtual",
    "when",
    "where",
    "window",
    "with",
    "without",
];

pub fn is_keyword(name: &str) -> bool {
    KEYWORDS
        .binary_search(&name.to_ascii_lowercase().as_str())
        .is_ok()
}

#[cfg(test)]
mod keyword_tests {
    #[test]
    fn keywords_are_sorted() {
        let mut v = super::KEYWORDS.to_vec();
        v.sort();
        assert_eq!(v, super::KEYWORDS);
    }
}
