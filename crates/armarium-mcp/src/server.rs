//! MCP tools, resources, and prompts (08. MCP server, sections 3 and 4).

use crate::text::{csv, markdown_table};
use armarium_core::model::{CellValue, ExportFormat, StatementResult, TableId};
use armarium_core::sql::SqlOptions;
use armarium_core::{CoreError, Engine};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Options {
    pub read_only: bool,
    pub allow_dirs: Vec<PathBuf>,
    pub timeout: Duration,
    pub max_rows: usize,
}

/// The tools that write. `--read-only` removes them.
pub const WRITE_TOOLS: [&str; 6] = [
    "execute",
    "save_table",
    "discard_changes",
    "register_source",
    "unregister_source",
    "export_result",
];

#[derive(Clone)]
pub struct Armarium {
    engine: Arc<Engine>,
    opts: Arc<Options>,
    tool_router: ToolRouter<Armarium>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TableArgs {
    /// The SQL name of the table (see list_sources).
    pub table: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadRowsArgs {
    /// The SQL name of the table.
    pub table: String,
    /// The first row (0-based, in file order).
    #[serde(default)]
    pub offset: Option<u64>,
    /// The number of rows (limited by the server setting --max-rows).
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryArgs {
    /// One SELECT statement (SQLite dialect). Tables are referred to by their SQL names.
    pub sql: String,
    /// The maximum number of rows to return (limited by --max-rows).
    #[serde(default)]
    pub max_rows: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExecuteArgs {
    /// One or more INSERT, UPDATE, or DELETE statements, separated by ';'.
    pub sql: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DiffArgs {
    /// The SQL name of the table.
    pub table: String,
    /// The maximum number of changed cells in the sample (default 20).
    #[serde(default)]
    pub sample: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RegisterArgs {
    /// The absolute path of a CSV, TSV, TXT, XLSX, XLSM, XLS, XLSB, or ODS file.
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UnregisterArgs {
    /// The SQL name of a table of the source, or the source ID.
    pub source: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExportArgs {
    /// One SELECT statement.
    pub sql: String,
    /// The absolute path of the output file (.csv, .tsv, or .xlsx).
    pub path: String,
}

fn core_error(e: CoreError) -> CallToolResult {
    let mut message = e.to_string();
    if matches!(e, CoreError::ExternalModification(_)) {
        message.push_str(
            ". Call discard_changes to read the file again, and then make the changes again.",
        );
    }
    let v = json!({ "error": { "code": e.code(), "message": message } });
    let mut r = CallToolResult::structured_error(v);
    r.content = vec![ContentBlock::text(format!("{}: {message}", e.code()))];
    r
}

fn ok(structured: Value, text: String) -> CallToolResult {
    let mut r = CallToolResult::structured(structured);
    r.content = vec![ContentBlock::text(text)];
    r
}

fn cell_json(v: &CellValue) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// Columns, rows, total number of rows, and the result ID of a query.
type Selected = (
    Vec<armarium_core::model::ResultColumn>,
    Vec<Vec<CellValue>>,
    u64,
    u32,
);

fn rows_json(rows: &[Vec<CellValue>]) -> Value {
    Value::Array(
        rows.iter()
            .map(|r| Value::Array(r.iter().map(cell_json).collect()))
            .collect(),
    )
}

impl Armarium {
    pub fn new(engine: Arc<Engine>, opts: Options) -> Armarium {
        let mut tool_router = Self::tool_router();
        if opts.read_only {
            for t in WRITE_TOOLS {
                tool_router.remove_route(t);
            }
        }
        Armarium {
            engine,
            opts: Arc::new(opts),
            tool_router,
        }
    }

    /// Runs blocking engine work on a worker thread. Reads `workspace.json` again
    /// first if another process changed it (08. MCP server, section 6).
    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Engine, &Options) -> Result<T, CoreError> + Send + 'static,
    ) -> Result<T, CoreError> {
        let engine = self.engine.clone();
        let opts = self.opts.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = engine.check_external_changes() {
                tracing::warn!("cannot check external changes: {e}");
            }
            f(&engine, &opts)
        })
        .await
        .map_err(|e| CoreError::Internal(e.to_string()))?
    }

    fn check_path(opts: &Options, path: &str, must_exist: bool) -> Result<PathBuf, CoreError> {
        let p = Path::new(path);
        if !p.is_absolute() {
            return Err(CoreError::InvalidArgument(
                "the path must be absolute".into(),
            ));
        }
        let resolved = if must_exist {
            std::fs::canonicalize(p).map_err(|e| CoreError::from_io(e, p))?
        } else {
            let parent = p
                .parent()
                .ok_or_else(|| CoreError::InvalidArgument("the path has no directory".into()))?;
            let dir = std::fs::canonicalize(parent).map_err(|e| CoreError::from_io(e, parent))?;
            dir.join(
                p.file_name().ok_or_else(|| {
                    CoreError::InvalidArgument("the path has no file name".into())
                })?,
            )
        };
        if opts.allow_dirs.iter().any(|d| resolved.starts_with(d)) {
            Ok(resolved)
        } else if opts.allow_dirs.is_empty() {
            Err(CoreError::ReadOnly(
                "file access is not allowed. Start the server with --allow-dir".into(),
            ))
        } else {
            Err(CoreError::ReadOnly(format!(
                "the path is not in an allowed directory ({})",
                opts.allow_dirs
                    .iter()
                    .map(|d| d.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
    }

    /// Runs one SELECT statement and returns the columns, rows, total, and result ID.
    fn select(
        engine: &Engine,
        opts: &Options,
        sql: &str,
        limit: usize,
    ) -> Result<Selected, CoreError> {
        let run = engine.with_timeout(Some(opts.timeout), |e| {
            let run = e.run_sql(
                sql,
                &SqlOptions {
                    read_only: true,
                    ..Default::default()
                },
            );
            match run.error {
                Some(err) => {
                    for r in &run.results {
                        if let StatementResult::Rows { result_id, .. } = r {
                            let _ = e.close_view(*result_id);
                        }
                    }
                    Err(match err.code.as_str() {
                        "SQL_FORBIDDEN" => CoreError::SqlForbidden(err.message),
                        "CANCELLED" => CoreError::Sql("the query timed out".into()),
                        _ => CoreError::Sql(err.message),
                    })
                }
                None => Ok(run),
            }
        })?;
        let mut ids: Vec<u32> = run
            .results
            .iter()
            .filter_map(|r| match r {
                StatementResult::Rows { result_id, .. } => Some(*result_id),
                _ => None,
            })
            .collect();
        let last = ids
            .pop()
            .ok_or_else(|| CoreError::InvalidArgument("the SQL has no SELECT statement".into()))?;
        for id in ids {
            let _ = engine.close_view(id);
        }
        let (cols, rows, total) = engine.result_values(last, 0, limit as u32)?;
        Ok((cols, rows, total, last))
    }

    fn table_id(engine: &Engine, name: &str) -> Result<TableId, CoreError> {
        engine.find_table(name)
    }
}

#[tool_router]
impl Armarium {
    /// List the registered sources and their tables: SQL name, file path, type, and load state.
    #[tool(annotations(read_only_hint = true))]
    async fn list_sources(&self) -> Result<CallToolResult, McpError> {
        let r = self.blocking(|e, _| e.list_sources()).await;
        Ok(match r {
            Err(e) => core_error(e),
            Ok(sources) => {
                let mut md =
                    String::from("| SQL name | file | sheet | state |\n|---|---|---|---|\n");
                for s in &sources {
                    for t in &s.tables {
                        let state = match &t.load {
                            armarium_core::model::LoadState::Unloaded => "not loaded".to_string(),
                            armarium_core::model::LoadState::Loaded { dirty: true } => {
                                "loaded, changed".to_string()
                            }
                            armarium_core::model::LoadState::Loaded { dirty: false } => {
                                "loaded".to_string()
                            }
                            armarium_core::model::LoadState::Error { code, .. } => {
                                format!("error {code}")
                            }
                        };
                        md.push_str(&format!(
                            "| {} | {} | {} | {} |\n",
                            t.sql_name,
                            s.path,
                            t.sheet.clone().unwrap_or_default(),
                            state
                        ));
                    }
                }
                if sources.is_empty() {
                    md = "No sources are registered.".into();
                }
                ok(json!({ "sources": sources }), md)
            }
        })
    }

    /// Show the columns (SQL names and logical types), the number of rows, and the first 5 rows of a table.
    #[tool(annotations(read_only_hint = true))]
    async fn describe_table(
        &self,
        Parameters(args): Parameters<TableArgs>,
    ) -> Result<CallToolResult, McpError> {
        let r = self
            .blocking(move |e, _| {
                let id = Self::table_id(e, &args.table)?;
                let meta = e.open_table(id)?;
                let (_, rows, _) = e.table_values(id, 0, 5)?;
                Ok((meta, rows))
            })
            .await;
        Ok(match r {
            Err(e) => core_error(e),
            Ok((meta, rows)) => {
                let cols: Vec<Value> = meta
                    .columns
                    .iter()
                    .map(|c| json!({ "name": c.sql_name, "header": c.display_name, "type": c.logical_type }))
                    .collect();
                let headers: Vec<String> =
                    meta.columns.iter().map(|c| c.sql_name.clone()).collect();
                let mut md = format!(
                    "Table `{}`: {} rows\n\n| column | type |\n|---|---|\n",
                    meta.sql_name, meta.row_count
                );
                for c in &meta.columns {
                    md.push_str(&format!(
                        "| {} | {} |\n",
                        c.sql_name,
                        c.logical_type.as_str()
                    ));
                }
                md.push_str("\nFirst rows:\n\n");
                md.push_str(&markdown_table(&headers, &rows));
                ok(
                    json!({
                        "table": meta.sql_name,
                        "columns": cols,
                        "row_count": meta.row_count,
                        "dirty": meta.dirty,
                        "sample": rows_json(&rows),
                    }),
                    md,
                )
            }
        })
    }

    /// Get rows of a table in file order, with offset and limit.
    #[tool(annotations(read_only_hint = true))]
    async fn read_rows(
        &self,
        Parameters(args): Parameters<ReadRowsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let r = self
            .blocking(move |e, o| {
                let id = Self::table_id(e, &args.table)?;
                let limit = args
                    .limit
                    .map(|l| l as usize)
                    .unwrap_or(o.max_rows)
                    .min(o.max_rows);
                let offset = args.offset.unwrap_or(0);
                let (cols, rows, total) = e.table_values(id, offset, limit as u32)?;
                Ok((cols, rows, total, offset))
            })
            .await;
        Ok(match r {
            Err(e) => core_error(e),
            Ok((cols, rows, total, offset)) => {
                let headers: Vec<String> = cols.iter().map(|c| c.sql_name.clone()).collect();
                let truncated = offset + (rows.len() as u64) < total;
                let mut md = markdown_table(&headers, &rows);
                md.push_str(&format!(
                    "\nRows {}–{} of {total}.",
                    offset + 1,
                    offset + rows.len() as u64
                ));
                ok(
                    json!({
                        "columns": cols.iter().map(|c| json!({"name": c.sql_name, "type": c.logical_type})).collect::<Vec<_>>(),
                        "rows": rows_json(&rows),
                        "offset": offset,
                        "row_count": total,
                        "truncated": truncated,
                    }),
                    md,
                )
            }
        })
    }

    /// Run a query (SELECT, WITH, or VALUES) on the registered tables and return the result.
    /// Use SQLite SQL. Tables are referred to by their SQL names (see list_sources).
    #[tool(annotations(read_only_hint = true))]
    async fn query(
        &self,
        Parameters(args): Parameters<QueryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let r = self
            .blocking(move |e, o| {
                let limit = args
                    .max_rows
                    .map(|m| m as usize)
                    .unwrap_or(o.max_rows)
                    .min(o.max_rows);
                let (cols, rows, total, id) = Self::select(e, o, &args.sql, limit)?;
                let _ = e.close_view(id);
                Ok((cols, rows, total))
            })
            .await;
        Ok(match r {
            Err(e) => core_error(e),
            Ok((cols, rows, total)) => {
                let headers: Vec<String> = cols.iter().map(|c| c.name.clone()).collect();
                let truncated = (rows.len() as u64) < total;
                let mut md = markdown_table(&headers, &rows);
                if truncated {
                    md.push_str(&format!(
                        "\nThe result has {total} rows; only {} are shown. Add LIMIT or aggregate the data, or use export_result to write all rows to a file.",
                        rows.len()
                    ));
                } else {
                    md.push_str(&format!("\n{total} rows."));
                }
                ok(
                    json!({
                        "columns": cols.iter().map(|c| json!({"name": c.name, "type": c.logical_type})).collect::<Vec<_>>(),
                        "rows": rows_json(&rows),
                        "row_count": total,
                        "truncated": truncated,
                    }),
                    md,
                )
            }
        })
    }

    /// Run INSERT, UPDATE, or DELETE statements. This changes the working copy only;
    /// call save_table to write the file. Each statement can change one table.
    #[tool(annotations(
        read_only_hint = false,
        destructive_hint = false,
        idempotent_hint = false
    ))]
    async fn execute(
        &self,
        Parameters(args): Parameters<ExecuteArgs>,
    ) -> Result<CallToolResult, McpError> {
        let r = self
            .blocking(move |e, o| {
                let run = e.with_timeout(Some(o.timeout), |e| {
                    Ok(e.run_sql(&args.sql, &SqlOptions { read_only: false, allow_no_undo: true }))
                })?;
                let mut out = Vec::new();
                for res in &run.results {
                    match res {
                        StatementResult::Changed { sql, sql_name, changed_rows, result_id, .. } => {
                            let mut item = json!({ "sql": sql, "table": sql_name, "changed_rows": changed_rows });
                            if let Some(id) = result_id {
                                let (cols, rows, total) = e.result_values(*id, 0, o.max_rows as u32)?;
                                item["returning"] = json!({
                                    "columns": cols.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
                                    "rows": rows_json(&rows),
                                    "row_count": total,
                                });
                                let _ = e.close_view(*id);
                            }
                            out.push(item);
                        }
                        StatementResult::Rows { sql, result_id, row_count, .. } => {
                            let _ = e.close_view(*result_id);
                            out.push(json!({ "sql": sql, "note": "a query does not change data; use the query tool", "row_count": row_count }));
                        }
                    }
                }
                Ok((out, run.error))
            })
            .await;
        Ok(match r {
            Err(e) => core_error(e),
            Ok((out, err)) => {
                let mut md = String::new();
                for item in &out {
                    if let Some(n) = item.get("changed_rows") {
                        md.push_str(&format!(
                            "{} row(s) changed in `{}`.\n",
                            n,
                            item["table"].as_str().unwrap_or("")
                        ));
                    }
                }
                if !out.is_empty() {
                    md.push_str("The working copy changed. Call save_table to write the file.\n");
                }
                match err {
                    Some(err) => {
                        md.push_str(&format!(
                            "Statement {} failed: {}: {}",
                            err.statement_index + 1,
                            err.code,
                            err.message
                        ));
                        let v = json!({ "results": out, "error": err });
                        let mut r = CallToolResult::structured_error(v);
                        r.content = vec![ContentBlock::text(md)];
                        r
                    }
                    None => ok(json!({ "results": out }), md),
                }
            }
        })
    }

    /// Show the differences between the working copy of a table and its file:
    /// the number of inserted, deleted, and changed rows, and some changed cells.
    #[tool(annotations(read_only_hint = true))]
    async fn diff_table(
        &self,
        Parameters(args): Parameters<DiffArgs>,
    ) -> Result<CallToolResult, McpError> {
        let r = self
            .blocking(move |e, _| {
                let id = Self::table_id(e, &args.table)?;
                e.diff_table(id, args.sample.unwrap_or(20) as usize)
            })
            .await;
        Ok(match r {
            Err(e) => core_error(e),
            Ok(d) => {
                let mut md = format!(
                    "Inserted rows: {}, deleted rows: {}, changed rows: {}{}\n",
                    d.inserted_rows,
                    d.deleted_rows,
                    d.changed_rows,
                    if d.columns_changed {
                        ", columns changed"
                    } else {
                        ""
                    }
                );
                if !d.sample.is_empty() {
                    md.push_str("\n| row | column | before | after |\n|---|---|---|---|\n");
                    for c in &d.sample {
                        md.push_str(&format!(
                            "| {} | {} | {} | {} |\n",
                            c.row_number,
                            c.column,
                            c.before.clone().unwrap_or_default(),
                            c.after.clone().unwrap_or_default()
                        ));
                    }
                }
                ok(serde_json::to_value(&d).unwrap_or(Value::Null), md)
            }
        })
    }

    /// Save the working copy of a table to its file (the file is overwritten).
    /// Fails if another program changed the file.
    #[tool(annotations(
        read_only_hint = false,
        destructive_hint = true,
        idempotent_hint = true
    ))]
    async fn save_table(
        &self,
        Parameters(args): Parameters<TableArgs>,
    ) -> Result<CallToolResult, McpError> {
        let r = self
            .blocking(move |e, _| {
                let id = Self::table_id(e, &args.table)?;
                let info = e.table_info(id)?;
                let meta = e.save_table(id, false)?;
                let path = e
                    .list_sources()?
                    .into_iter()
                    .find(|s| s.id == info.source_id)
                    .map(|s| s.path)
                    .unwrap_or_default();
                Ok((meta, path))
            })
            .await;
        Ok(match r {
            Err(e) => core_error(e),
            Ok((meta, path)) => ok(
                json!({ "table": meta.sql_name, "path": path, "row_count": meta.row_count }),
                format!(
                    "Saved `{}` to {path} ({} rows).",
                    meta.sql_name, meta.row_count
                ),
            ),
        })
    }

    /// Discard the changes of a table and read its file again.
    #[tool(annotations(
        read_only_hint = false,
        destructive_hint = false,
        idempotent_hint = false
    ))]
    async fn discard_changes(
        &self,
        Parameters(args): Parameters<TableArgs>,
    ) -> Result<CallToolResult, McpError> {
        let r = self
            .blocking(move |e, _| {
                let id = Self::table_id(e, &args.table)?;
                e.reload_table(id)
            })
            .await;
        Ok(match r {
            Err(e) => core_error(e),
            Ok(meta) => ok(
                json!({ "table": meta.sql_name, "row_count": meta.row_count }),
                format!(
                    "Read `{}` again from its file ({} rows).",
                    meta.sql_name, meta.row_count
                ),
            ),
        })
    }

    /// Register a file as a source (only in the directories that the server allows).
    #[tool(annotations(
        read_only_hint = false,
        destructive_hint = false,
        idempotent_hint = true
    ))]
    async fn register_source(
        &self,
        Parameters(args): Parameters<RegisterArgs>,
    ) -> Result<CallToolResult, McpError> {
        let r = self
            .blocking(move |e, o| {
                let path = Self::check_path(o, &args.path, true)?;
                let r = e.add_sources(std::slice::from_ref(&path))?;
                let item = r
                    .into_iter()
                    .next()
                    .ok_or_else(|| CoreError::Internal("no result".into()))?;
                if let Some(err) = item.error {
                    return Err(CoreError::InvalidArgument(format!(
                        "{}: {}",
                        err.code, err.message
                    )));
                }
                let id = item.source_id.unwrap_or_default();
                let src = e.list_sources()?.into_iter().find(|s| s.id == id);
                Ok((item.added, src))
            })
            .await;
        Ok(match r {
            Err(e) => core_error(e),
            Ok((added, src)) => {
                let names: Vec<String> = src
                    .iter()
                    .flat_map(|s| s.tables.iter().map(|t| t.sql_name.clone()))
                    .collect();
                ok(
                    json!({ "added": added, "source": src }),
                    format!(
                        "{} Tables: {}",
                        if added {
                            "Registered the file."
                        } else {
                            "The file was already registered."
                        },
                        names.join(", ")
                    ),
                )
            }
        })
    }

    /// Unregister a source. The file is not deleted.
    #[tool(annotations(
        read_only_hint = false,
        destructive_hint = false,
        idempotent_hint = true
    ))]
    async fn unregister_source(
        &self,
        Parameters(args): Parameters<UnregisterArgs>,
    ) -> Result<CallToolResult, McpError> {
        let r = self
            .blocking(move |e, _| {
                let sid = match e.find_table(&args.source) {
                    Ok(id) => e.table_info(id)?.source_id,
                    Err(_) => args.source.clone(),
                };
                e.remove_source(&sid)?;
                Ok(sid)
            })
            .await;
        Ok(match r {
            Err(e) => core_error(e),
            Ok(sid) => ok(
                json!({ "source_id": sid }),
                "Unregistered the source. The file was not deleted.".into(),
            ),
        })
    }

    /// Run a SELECT statement and write all rows of the result to a CSV, TSV, or XLSX file
    /// (only in the directories that the server allows).
    #[tool(annotations(
        read_only_hint = false,
        destructive_hint = true,
        idempotent_hint = true
    ))]
    async fn export_result(
        &self,
        Parameters(args): Parameters<ExportArgs>,
    ) -> Result<CallToolResult, McpError> {
        let r = self
            .blocking(move |e, o| {
                let path = Self::check_path(o, &args.path, false)?;
                let (_, _, total, id) = Self::select(e, o, &args.sql, 0)?;
                let r = e.export_result(id, &path, ExportFormat::for_path(&path));
                let _ = e.close_view(id);
                r?;
                Ok((path, total))
            })
            .await;
        Ok(match r {
            Err(e) => core_error(e),
            Ok((path, total)) => ok(
                json!({ "path": path, "row_count": total }),
                format!("Wrote {total} rows to {}.", path.display()),
            ),
        })
    }
}

const SCHEMA_PREFIX: &str = "armarium://tables/";

fn parse_uri(uri: &str) -> Option<(String, &str)> {
    let rest = uri.strip_prefix(SCHEMA_PREFIX)?;
    let (name, kind) = rest.rsplit_once('/')?;
    let name = percent_decode(name);
    Some((name, kind))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl Armarium {
    fn column_stats(e: &Engine, table: &str) -> Result<String, CoreError> {
        let id = e.find_table(table)?;
        let meta = e.open_table(id)?;
        let q = |s: &str| armarium_core::naming::quote_ident(s);
        let mut parts = Vec::new();
        for c in &meta.columns {
            let n = q(&c.sql_name);
            let name = format!("'{}'", c.sql_name.replace('\'', "''"));
            parts.push(format!(
                "SELECT {name} AS column_name, '{ty}' AS type, count({n}) AS count, count(*) - count({n}) AS nulls, \
                 min({n}) AS min, max({n}) AS max, count(DISTINCT {n}) AS distinct_values FROM {t}",
                ty = c.logical_type.as_str(),
                t = q(&meta.sql_name),
            ));
        }
        let sql = parts.join(" UNION ALL ");
        let opts = Options {
            read_only: true,
            allow_dirs: vec![],
            timeout: Duration::from_secs(120),
            max_rows: 10_000,
        };
        let (cols, rows, _, rid) = Self::select(e, &opts, &sql, 10_000)?;
        let _ = e.close_view(rid);
        let headers: Vec<String> = cols.iter().map(|c| c.name.clone()).collect();
        Ok(format!(
            "Table `{}` has {} rows and {} columns.\n\n{}",
            meta.sql_name,
            meta.row_count,
            meta.columns.len(),
            markdown_table(&headers, &rows)
        ))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Armarium {
    fn get_info(&self) -> ServerConfig {
        let caps = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .enable_prompts()
            .build();
        let mut info = Implementation::new("armarium", env!("CARGO_PKG_VERSION"));
        info.title = Some("Armarium".into());
        ServerConfig::new(caps).with_server_info(info).with_instructions(
            "Armarium gives access to tabular files (CSV, TSV, Excel) that the user registered. \
             Call list_sources to see the tables and their SQL names, describe_table to see columns, \
             and query to run SQLite SELECT statements (joins and GROUP BY across files work). \
             execute changes only the working copy; save_table writes the file. \
             Results are limited in size: aggregate in SQL or add LIMIT.",
        )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let sources = self
            .blocking(|e, _| e.list_sources())
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let mut items = Vec::new();
        for s in &sources {
            for t in &s.tables {
                let enc = percent_encode(&t.sql_name);
                items.push(
                    Resource::new(
                        format!("{SCHEMA_PREFIX}{enc}/schema"),
                        format!("{} schema", t.sql_name),
                    )
                    .with_description(format!("Columns and types of {} ({})", t.sql_name, s.path))
                    .with_mime_type("application/json"),
                );
            }
        }
        Ok(ListResourcesResult::with_all_items(items))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new(
                format!("{SCHEMA_PREFIX}{{sql_name}}/schema"),
                "table schema",
            )
            .with_description("The schema of a table (JSON)")
            .with_mime_type("application/json"),
            ResourceTemplate::new(
                format!("{SCHEMA_PREFIX}{{sql_name}}/sample"),
                "table sample",
            )
            .with_description("The first 20 rows of a table (CSV)")
            .with_mime_type("text/csv"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri.clone();
        let Some((name, kind)) = parse_uri(&uri) else {
            return Err(McpError::resource_not_found(
                format!("unknown resource: {uri}"),
                None,
            ));
        };
        let kind = kind.to_string();
        let r = self
            .blocking(move |e, _| {
                let id = e.find_table(&name)?;
                let meta = e.open_table(id)?;
                if kind == "schema" {
                    let v = json!({
                        "table": meta.sql_name,
                        "row_count": meta.row_count,
                        "columns": meta.columns.iter().map(|c| json!({"name": c.sql_name, "header": c.display_name, "type": c.logical_type})).collect::<Vec<_>>(),
                    });
                    Ok(Some(("application/json", serde_json::to_string_pretty(&v).unwrap_or_default())))
                } else if kind == "sample" {
                    let (cols, rows, _) = e.table_values(id, 0, 20)?;
                    let headers: Vec<String> = cols.iter().map(|c| c.sql_name.clone()).collect();
                    Ok(Some(("text/csv", csv(&headers, &rows))))
                } else {
                    Ok(None)
                }
            })
            .await;
        match r {
            Ok(Some((mime, text))) => {
                let contents = ResourceContents::text(text, uri).with_mime_type(mime);
                Ok(ReadResourceResult::new(vec![contents]).into())
            }
            Ok(None) => Err(McpError::resource_not_found(
                format!("unknown resource: {uri}"),
                None,
            )),
            Err(CoreError::NotFound(m)) => Err(McpError::resource_not_found(m, None)),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        Ok(ListPromptsResult::with_all_items(vec![Prompt::new(
            "analyze_table",
            Some("Get statistics for each column of a table and explain the patterns in the data"),
            Some(vec![PromptArgument::new("table")
                .with_description("The SQL name of the table")
                .with_required(true)]),
        )]))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        if request.name != "analyze_table" {
            return Err(McpError::invalid_params(
                format!("unknown prompt: {}", request.name),
                None,
            ));
        }
        let table = request
            .arguments
            .as_ref()
            .and_then(|a| a.get("table"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| McpError::invalid_params("the argument 'table' is necessary", None))?;
        let stats = self
            .blocking(move |e, _| Self::column_stats(e, &table))
            .await
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let text = format!(
            "{stats}\n\nThe table above shows statistics for each column (count of values, number of NULLs, \
             minimum, maximum, and number of distinct values). Explain the patterns in this data: \
             what each column probably means, data quality problems (for example, many NULLs or \
             unexpected values), and useful next queries. Use the query tool to look at the data in more detail."
        );
        Ok(GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)]).into())
    }
}

#[cfg(test)]
mod tests;
