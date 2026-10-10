//! The engine: the public API of the core library ([ADR-0004]).
//!
//! One `Engine` owns the session database of a process. All write operations run
//! one at a time on the write connection (a mutex replaces the worker thread of
//! 02. Architecture, section 6, with the same effect). Page fetches use a separate
//! read connection, so the grid can scroll while a long operation runs.

mod edit;
mod persist;
mod views;

pub use persist::write_csv_rows;

/// The name of the internal table of a query result.
pub fn result_table_name(id: ViewId) -> String {
    views::result_table(id)
}

use crate::db::auth::Sandbox;
use crate::db::{self, DbSettings, SessionFile};
use crate::error::{CoreError, ErrorPayload, Result};
use crate::fsutil::{self, Fingerprint};
use crate::ingest::{self, LoadControl, Loaded};
use crate::model::*;
use crate::naming::quote_ident;
use crate::workspace::Workspace;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use ts_rs::TS;

/// Receives notifications from the engine.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: CoreEvent);
}

/// An event sink that ignores all events.
pub struct NoEvents;

impl EventSink for NoEvents {
    fn emit(&self, _event: CoreEvent) {}
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub workspace_path: PathBuf,
    /// The directory for the session database (`%LOCALAPPDATA%\Armarium\tmp`).
    pub temp_dir: PathBuf,
    pub db: DbSettings,
    /// The maximum number of undo entries for each table.
    pub undo_max_ops: usize,
    /// The maximum total size of the undo entries for each table, in bytes.
    pub undo_max_bytes: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            workspace_path: Workspace::default_path(),
            temp_dir: fsutil::local_data_dir().join("tmp"),
            db: DbSettings::default(),
            undo_max_ops: 1000,
            undo_max_bytes: 256 * 1024 * 1024,
        }
    }
}

/// The result of one file in `add_sources`.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AddSourceResult {
    pub path: String,
    pub source_id: Option<SourceId>,
    /// False if the file was already registered.
    pub added: bool,
    pub error: Option<ErrorPayload>,
}

/// Names for completion in the SQL editor.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CompletionTable {
    pub sql_name: String,
    pub description: String,
    /// The columns of a loaded table. `null` if the table is not loaded.
    pub columns: Option<Vec<String>>,
}

pub(crate) struct Table {
    pub id: TableId,
    pub source_id: SourceId,
    pub sheet: Option<String>,
    pub sql_name: String,
    pub hidden: bool,
    pub data: Option<TableData>,
    pub error: Option<ErrorPayload>,
    pub externally_modified: bool,
}

pub(crate) struct TableData {
    pub columns: Vec<ColumnSchema>,
    pub dialect: Option<Dialect>,
    pub fingerprint: Fingerprint,
    pub warnings: Vec<String>,
    pub notes: HashMap<(i64, String), (CellNoteKind, String)>,
    pub undo: Vec<edit::UndoEntry>,
    pub redo: Vec<edit::UndoEntry>,
    pub state: u64,
    pub saved_state: u64,
    pub version: u64,
    pub row_count: u64,
    pub save_as_only: bool,
}

impl TableData {
    pub fn dirty(&self) -> bool {
        self.state != self.saved_state
    }
}

/// What the read connection needs to fetch the pages of a table.
#[derive(Clone)]
pub(crate) struct FetchMeta {
    pub sql_name: String,
    pub id: TableId,
    pub columns: Vec<ColumnSchema>,
    pub row_count: u64,
    pub notes: Arc<HashMap<(i64, String), (CellNoteKind, String)>>,
}

#[derive(Clone)]
pub(crate) enum ViewMeta {
    Grid {
        table: TableId,
        spec: ViewSpec,
        total: u64,
    },
    Result {
        columns: Vec<ResultColumn>,
        total: u64,
    },
}

pub(crate) struct Inner {
    pub conn: Connection,
    pub sandbox: Sandbox,
    pub ws: Workspace,
    pub tables: BTreeMap<TableId, Table>,
    pub next_table: TableId,
    pub next_view: ViewId,
    pub next_state: u64,
    pub cfg: EngineConfig,
    pub sink: Arc<dyn EventSink>,
    pub cancel: Arc<AtomicBool>,
    pub fetch: Arc<Mutex<HashMap<TableId, FetchMeta>>>,
    pub views: Arc<Mutex<HashMap<ViewId, ViewMeta>>>,
}

pub struct Engine {
    inner: Mutex<Inner>,
    reader: Mutex<Connection>,
    interrupt: rusqlite::InterruptHandle,
    cancel: Arc<AtomicBool>,
    deadline: Arc<Mutex<Option<Instant>>>,
    fetch: Arc<Mutex<HashMap<TableId, FetchMeta>>>,
    views: Arc<Mutex<HashMap<ViewId, ViewMeta>>>,
    // Dropped last: deletes the session database file.
    _session: SessionFile,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Engine {
    pub fn new(cfg: EngineConfig, sink: Arc<dyn EventSink>) -> Result<Engine> {
        let session = SessionFile::create(&cfg.temp_dir)?;
        let conn = db::open_write(session.path(), &cfg.db)?;
        let sandbox = Sandbox::install(&conn)?;
        let reader = db::open_read(session.path(), &cfg.db)?;
        let interrupt = conn.get_interrupt_handle();
        let cancel = Arc::new(AtomicBool::new(false));
        let deadline: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let d2 = deadline.clone();
        conn.progress_handler(
            10_000,
            Some(move || match *lock(&d2) {
                Some(t) => Instant::now() >= t,
                None => false,
            }),
        )?;
        let ws = Workspace::open(&cfg.workspace_path)?;
        let fetch = Arc::new(Mutex::new(HashMap::new()));
        let views = Arc::new(Mutex::new(HashMap::new()));
        let mut inner = Inner {
            conn,
            sandbox,
            ws,
            tables: BTreeMap::new(),
            next_table: 1,
            next_view: 1,
            next_state: 1,
            cfg,
            sink,
            cancel: cancel.clone(),
            fetch: fetch.clone(),
            views: views.clone(),
        };
        inner.sync_tables();
        Ok(Engine {
            inner: Mutex::new(inner),
            reader: Mutex::new(reader),
            interrupt,
            cancel,
            deadline,
            fetch,
            views,
            _session: session,
        })
    }

    fn run<T>(&self, f: impl FnOnce(&mut Inner) -> Result<T>) -> Result<T> {
        let mut inner = lock(&self.inner);
        self.cancel.store(false, Ordering::SeqCst);
        f(&mut inner)
    }

    /// Cancels the running operation (a read or a query).
    pub fn interrupt(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.interrupt.interrupt();
    }

    /// Runs `f` with a time limit for SQL (the MCP server uses it).
    pub fn with_timeout<T>(
        &self,
        timeout: Option<Duration>,
        f: impl FnOnce(&Engine) -> Result<T>,
    ) -> Result<T> {
        *lock(&self.deadline) = timeout.map(|t| Instant::now() + t);
        let r = f(self);
        *lock(&self.deadline) = None;
        match r {
            Err(CoreError::Cancelled)
                if timeout.is_some() && !self.cancel.load(Ordering::SeqCst) =>
            {
                Err(CoreError::Sql("the query timed out".into()))
            }
            other => other,
        }
    }

    pub fn workspace_path(&self) -> PathBuf {
        lock(&self.inner).ws.path().to_path_buf()
    }

    // ----- Sources and tables -----

    pub fn list_sources(&self) -> Result<Vec<SourceInfo>> {
        self.run(|i| Ok(i.source_infos()))
    }

    pub fn add_sources(&self, paths: &[PathBuf]) -> Result<Vec<AddSourceResult>> {
        self.run(|i| i.add_sources(paths))
    }

    pub fn remove_source(&self, source_id: &str) -> Result<()> {
        self.run(|i| i.remove_source(source_id))
    }

    pub fn rename_table(&self, table: TableId, name: &str) -> Result<TableInfo> {
        self.run(|i| i.rename_table(table, name))
    }

    pub fn relocate_source(&self, source_id: &str, path: &Path) -> Result<()> {
        self.run(|i| i.relocate_source(source_id, path))
    }

    pub fn read_options(&self, table: TableId) -> Result<ReadOptions> {
        self.run(|i| {
            let t = i.table(table)?;
            Ok(i.ws.source(&t.source_id)?.read_options(t.sheet.as_deref()))
        })
    }

    /// Changes the read settings and reads the table again. Changes that are not saved are lost.
    pub fn set_read_options(&self, table: TableId, options: ReadOptions) -> Result<TableMeta> {
        self.run(|i| {
            let (sid, sheet) = {
                let t = i.table(table)?;
                (t.source_id.clone(), t.sheet.clone())
            };
            i.ws.set_options(&sid, sheet.as_deref(), &options)?;
            i.ws.save()?;
            i.load(table)?;
            i.meta(table)
        })
    }

    pub fn find_table(&self, sql_name: &str) -> Result<TableId> {
        self.run(|i| i.find_table(sql_name))
    }

    pub fn table_info(&self, table: TableId) -> Result<TableInfo> {
        self.run(|i| i.table_info(table))
    }

    /// Loads the table if necessary and returns its schema.
    pub fn open_table(&self, table: TableId) -> Result<TableMeta> {
        self.run(|i| {
            i.ensure_loaded(table)?;
            i.meta(table)
        })
    }

    pub fn table_meta(&self, table: TableId) -> Result<TableMeta> {
        self.run(|i| i.meta(table))
    }

    /// Reads the file again and discards the changes (also "Discard changes").
    pub fn reload_table(&self, table: TableId) -> Result<TableMeta> {
        self.run(|i| {
            i.load(table)?;
            i.meta(table)
        })
    }

    /// Tables with changes that are not saved.
    pub fn dirty_tables(&self) -> Result<Vec<TableInfo>> {
        self.run(|i| {
            let ids: Vec<TableId> = i
                .tables
                .values()
                .filter(|t| t.data.as_ref().map(|d| d.dirty()).unwrap_or(false))
                .map(|t| t.id)
                .collect();
            ids.into_iter().map(|id| i.table_info(id)).collect()
        })
    }

    pub fn completion(&self) -> Result<Vec<CompletionTable>> {
        self.run(|i| {
            let mut out = Vec::new();
            for t in i.tables.values() {
                let src = i.ws.source(&t.source_id)?;
                let description = match &t.sheet {
                    Some(s) => format!("{} / {s}", src.file_name()),
                    None => src.file_name(),
                };
                out.push(CompletionTable {
                    sql_name: t.sql_name.clone(),
                    description,
                    columns: t
                        .data
                        .as_ref()
                        .map(|d| d.columns.iter().map(|c| c.sql_name.clone()).collect()),
                });
            }
            Ok(out)
        })
    }

    /// Reads `workspace.json` again if another process changed it, and checks the
    /// files of loaded tables. A loaded table without changes is read again
    /// automatically. Returns the tables that another program changed.
    pub fn check_external_changes(&self) -> Result<Vec<TableId>> {
        self.run(|i| i.check_external_changes())
    }

    // ----- Grid -----

    /// Fetches a page of rows. `view: None` is the order in the file.
    pub fn fetch_rows(
        &self,
        table: TableId,
        view: Option<ViewId>,
        offset: u64,
        limit: u32,
    ) -> Result<RowPage> {
        let meta = lock(&self.fetch)
            .get(&table)
            .cloned()
            .ok_or_else(|| CoreError::NotFound(format!("loaded table {table}")))?;
        let vmeta = match view {
            Some(v) => Some(
                lock(&self.views)
                    .get(&v)
                    .cloned()
                    .ok_or_else(|| CoreError::NotFound(format!("view {v}")))?,
            ),
            None => None,
        };
        let reader = lock(&self.reader);
        views::fetch_table_page(&reader, &meta, view.zip(vmeta), offset, limit)
    }

    /// Fetches a page of a query result.
    pub fn fetch_result(&self, result: ViewId, offset: u64, limit: u32) -> Result<RowPage> {
        let vmeta = lock(&self.views)
            .get(&result)
            .cloned()
            .ok_or_else(|| CoreError::NotFound(format!("result {result}")))?;
        let reader = lock(&self.reader);
        views::fetch_result_page(&reader, result, &vmeta, offset, limit)
    }

    /// Typed values of a query result (for MCP).
    pub fn result_values(
        &self,
        result: ViewId,
        offset: u64,
        limit: u32,
    ) -> Result<(Vec<ResultColumn>, Vec<Vec<CellValue>>, u64)> {
        let vmeta = lock(&self.views)
            .get(&result)
            .cloned()
            .ok_or_else(|| CoreError::NotFound(format!("result {result}")))?;
        let reader = lock(&self.reader);
        views::result_values(&reader, result, &vmeta, offset, limit)
    }

    /// Typed values of a table in file order (for MCP).
    pub fn table_values(
        &self,
        table: TableId,
        offset: u64,
        limit: u32,
    ) -> Result<(Vec<ColumnSchema>, Vec<Vec<CellValue>>, u64)> {
        self.run(|i| i.ensure_loaded(table))?;
        let meta = lock(&self.fetch)
            .get(&table)
            .cloned()
            .ok_or_else(|| CoreError::NotFound(format!("loaded table {table}")))?;
        let reader = lock(&self.reader);
        views::table_values(&reader, &meta, offset, limit)
    }

    /// Creates a sorted or filtered view. Returns the view ID and the number of rows.
    pub fn create_view(&self, table: TableId, spec: ViewSpec) -> Result<(ViewId, u64)> {
        self.run(|i| i.create_view(table, spec))
    }

    /// Closes a view or a query result.
    pub fn close_view(&self, view: ViewId) -> Result<()> {
        self.run(|i| i.close_view(view))
    }

    // ----- Edit -----

    pub fn apply_edit(&self, table: TableId, op: EditOp) -> Result<EditResult> {
        self.run(|i| i.apply_edit(table, op))
    }

    pub fn undo(&self, table: TableId) -> Result<TableMeta> {
        self.run(|i| i.undo(table))
    }

    pub fn redo(&self, table: TableId) -> Result<TableMeta> {
        self.run(|i| i.redo(table))
    }

    // ----- Save and export -----

    /// Writes the table to its file. With `force`, Armarium overwrites a file that
    /// another program changed.
    pub fn save_table(&self, table: TableId, force: bool) -> Result<TableMeta> {
        self.run(|i| i.save_table(table, force))
    }

    /// Writes the table to a different file and connects the source to it (CSV and TSV).
    /// For an Excel sheet, Armarium writes the file and registers it as a new source.
    pub fn save_table_as(
        &self,
        table: TableId,
        path: &Path,
        format: Option<ExportFormat>,
    ) -> Result<TableMeta> {
        self.run(|i| i.save_table_as(table, path, format))
    }

    pub fn export_table(&self, table: TableId, path: &Path, format: ExportFormat) -> Result<()> {
        self.run(|i| i.export_table(table, path, &format))
    }

    pub fn export_result(&self, result: ViewId, path: &Path, format: ExportFormat) -> Result<()> {
        self.run(|i| i.export_result(result, path, &format))
    }

    /// Writes a query result to a file and registers the file.
    pub fn save_result_as_source(
        &self,
        result: ViewId,
        path: &Path,
        format: ExportFormat,
    ) -> Result<SourceId> {
        self.run(|i| {
            i.export_result(result, path, &format)?;
            let r = i.add_sources(&[path.to_path_buf()])?;
            match r.into_iter().next() {
                Some(AddSourceResult {
                    source_id: Some(id),
                    ..
                }) => Ok(id),
                Some(AddSourceResult { error: Some(e), .. }) => Err(CoreError::Internal(e.message)),
                _ => Err(CoreError::Internal("cannot register the file".into())),
            }
        })
    }

    pub fn diff_table(&self, table: TableId, sample: usize) -> Result<TableDiff> {
        self.run(|i| i.diff_table(table, sample))
    }

    // ----- SQL -----

    pub fn run_sql(&self, text: &str, opts: &crate::sql::SqlOptions) -> crate::sql::SqlRun {
        let mut inner = lock(&self.inner);
        self.cancel.store(false, Ordering::SeqCst);
        crate::sql::run(&mut inner, text, opts)
    }
}

impl Inner {
    pub fn emit(&self, e: CoreEvent) {
        self.sink.emit(e);
    }

    pub fn table(&self, id: TableId) -> Result<&Table> {
        self.tables
            .get(&id)
            .ok_or_else(|| CoreError::NotFound(format!("table {id}")))
    }

    pub fn table_mut(&mut self, id: TableId) -> Result<&mut Table> {
        self.tables
            .get_mut(&id)
            .ok_or_else(|| CoreError::NotFound(format!("table {id}")))
    }

    pub fn data(&self, id: TableId) -> Result<&TableData> {
        self.table(id)?
            .data
            .as_ref()
            .ok_or_else(|| CoreError::NotFound(format!("loaded table {id}")))
    }

    pub fn data_mut(&mut self, id: TableId) -> Result<&mut TableData> {
        self.table_mut(id)?
            .data
            .as_mut()
            .ok_or_else(|| CoreError::NotFound(format!("loaded table {id}")))
    }

    pub fn new_state(&mut self) -> u64 {
        self.next_state += 1;
        self.next_state
    }

    pub fn find_table(&self, sql_name: &str) -> Result<TableId> {
        self.tables
            .values()
            .find(|t| t.sql_name.eq_ignore_ascii_case(sql_name))
            .map(|t| t.id)
            .ok_or_else(|| CoreError::NotFound(format!("table {sql_name}")))
    }

    /// Makes the table list agree with the workspace. Keeps the IDs of existing tables.
    pub fn sync_tables(&mut self) {
        let mut wanted: Vec<(SourceId, Option<String>, String, bool)> = Vec::new();
        for s in self.ws.sources() {
            match s.kind {
                SourceKind::Csv => {
                    if let Some(n) = &s.sql_name {
                        wanted.push((s.id.clone(), None, n.clone(), false));
                    }
                }
                SourceKind::Excel => {
                    for sh in &s.sheets {
                        wanted.push((
                            s.id.clone(),
                            Some(sh.name.clone()),
                            sh.sql_name.clone(),
                            sh.hidden,
                        ));
                    }
                }
            }
        }
        // Remove tables that are not in the workspace.
        let stale: Vec<TableId> = self
            .tables
            .values()
            .filter(|t| !wanted.iter().any(|w| w.0 == t.source_id && w.1 == t.sheet))
            .map(|t| t.id)
            .collect();
        for id in stale {
            self.drop_table_data(id);
            self.tables.remove(&id);
        }
        for (sid, sheet, name, hidden) in wanted {
            let existing = self
                .tables
                .values()
                .find(|t| t.source_id == sid && t.sheet == sheet)
                .map(|t| t.id);
            match existing {
                Some(id) => {
                    let old_name = self.tables[&id].sql_name.clone();
                    if old_name != name {
                        if self.tables[&id].data.is_some() {
                            if let Err(e) = self.rename_working_table(id, &old_name, &name) {
                                tracing::warn!("cannot rename table {old_name}: {e}");
                            }
                        }
                        self.tables.get_mut(&id).unwrap().sql_name = name;
                        self.publish_fetch(id);
                    }
                    self.tables.get_mut(&id).unwrap().hidden = hidden;
                }
                None => {
                    let id = self.next_table;
                    self.next_table += 1;
                    self.tables.insert(
                        id,
                        Table {
                            id,
                            source_id: sid,
                            sheet,
                            sql_name: name,
                            hidden,
                            data: None,
                            error: None,
                            externally_modified: false,
                        },
                    );
                }
            }
        }
    }

    fn rename_working_table(&mut self, id: TableId, old: &str, new: &str) -> Result<()> {
        self.conn.execute_batch(&format!(
            "ALTER TABLE {} RENAME TO {}",
            quote_ident(old),
            quote_ident(new)
        ))?;
        // Changesets keep the table name, so the history of the table is cleared.
        if let Some(d) = self.tables.get_mut(&id).and_then(|t| t.data.as_mut()) {
            d.undo.clear();
            d.redo.clear();
        }
        Ok(())
    }

    fn drop_table_data(&mut self, id: TableId) {
        let Some(t) = self.tables.get_mut(&id) else {
            return;
        };
        let had = t.data.take().is_some();
        let name = t.sql_name.clone();
        if had {
            if let Err(e) = ingest::drop_internal(&self.conn, id, &name) {
                tracing::warn!("cannot drop table {name}: {e}");
            }
        }
        lock(&self.fetch).remove(&id);
        let views: Vec<ViewId> = lock(&self.views)
            .iter()
            .filter(|(_, v)| matches!(v, ViewMeta::Grid { table, .. } if *table == id))
            .map(|(k, _)| *k)
            .collect();
        for v in views {
            let _ = self.close_view(v);
        }
    }

    pub fn source_infos(&self) -> Vec<SourceInfo> {
        self.ws
            .sources()
            .iter()
            .map(|s| {
                let meta = std::fs::metadata(&s.path).ok();
                let tables = self
                    .tables
                    .values()
                    .filter(|t| t.source_id == s.id)
                    .filter_map(|t| self.table_info(t.id).ok())
                    .collect::<Vec<_>>();
                let mut tables = tables;
                if s.kind == SourceKind::Excel {
                    tables.sort_by_key(|t| {
                        s.sheets
                            .iter()
                            .position(|sh| Some(&sh.name) == t.sheet.as_ref())
                    });
                }
                SourceInfo {
                    id: s.id.clone(),
                    path: s.path.clone(),
                    file_name: s.file_name(),
                    kind: s.kind,
                    exists: meta.is_some(),
                    size: meta.as_ref().map(|m| m.len()),
                    modified_ms: meta
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64),
                    tables,
                }
            })
            .collect()
    }

    pub fn table_info(&self, id: TableId) -> Result<TableInfo> {
        let t = self.table(id)?;
        let src = self.ws.source(&t.source_id)?;
        let load = match (&t.data, &t.error) {
            (Some(d), _) => LoadState::Loaded { dirty: d.dirty() },
            (None, Some(e)) => LoadState::Error {
                code: e.code.clone(),
                message: e.message.clone(),
            },
            (None, None) => LoadState::Unloaded,
        };
        Ok(TableInfo {
            id,
            source_id: t.source_id.clone(),
            sheet: t.sheet.clone(),
            sql_name: t.sql_name.clone(),
            hidden: t.hidden,
            load,
            externally_modified: t.externally_modified,
            read_only: src.kind == SourceKind::Excel || src.is_read_only_format(),
        })
    }

    pub fn meta(&self, id: TableId) -> Result<TableMeta> {
        let t = self.table(id)?;
        let d = self.data(id)?;
        Ok(TableMeta {
            id,
            sql_name: t.sql_name.clone(),
            columns: d.columns.clone(),
            row_count: d.row_count,
            dirty: d.dirty(),
            read_only: false,
            save_as_only: d.save_as_only,
            dialect: d.dialect.clone(),
            warnings: d.warnings.clone(),
            version: d.version,
            can_undo: !d.undo.is_empty(),
            can_redo: !d.redo.is_empty(),
        })
    }

    pub fn add_sources(&mut self, paths: &[PathBuf]) -> Result<Vec<AddSourceResult>> {
        let mut out = Vec::new();
        let mut changed = false;
        for p in paths {
            let r = (|| -> Result<(SourceId, bool)> {
                let kind = crate::workspace::kind_for_path(p)?;
                let abs = fsutil::absolute(p)?;
                if let Some(e) = self.ws.find_by_path(&abs.to_string_lossy()) {
                    return Ok((e.id.clone(), false));
                }
                let sheets = match kind {
                    SourceKind::Excel => Some(ingest::excel::list_sheets(&abs)?),
                    SourceKind::Csv => None,
                };
                self.ws.add_source(&abs, sheets)
            })();
            match r {
                Ok((id, added)) => {
                    changed |= added;
                    out.push(AddSourceResult {
                        path: p.display().to_string(),
                        source_id: Some(id),
                        added,
                        error: None,
                    })
                }
                Err(e) => out.push(AddSourceResult {
                    path: p.display().to_string(),
                    source_id: None,
                    added: false,
                    error: Some(e.to_payload()),
                }),
            }
        }
        if changed {
            self.ws.save()?;
            self.sync_tables();
            self.emit(CoreEvent::WorkspaceChanged);
        }
        Ok(out)
    }

    pub fn remove_source(&mut self, id: &str) -> Result<()> {
        self.ws.remove_source(id)?;
        self.ws.save()?;
        self.sync_tables();
        self.emit(CoreEvent::WorkspaceChanged);
        Ok(())
    }

    pub fn rename_table(&mut self, id: TableId, name: &str) -> Result<TableInfo> {
        let (sid, sheet) = {
            let t = self.table(id)?;
            (t.source_id.clone(), t.sheet.clone())
        };
        self.ws.rename_table(&sid, sheet.as_deref(), name)?;
        self.ws.save()?;
        self.sync_tables();
        self.emit(CoreEvent::WorkspaceChanged);
        self.table_info(id)
    }

    pub fn relocate_source(&mut self, id: &str, path: &Path) -> Result<()> {
        if !path.is_file() {
            return Err(CoreError::FileNotFound(path.display().to_string()));
        }
        self.ws.relocate(id, path)?;
        self.ws.save()?;
        let ids: Vec<TableId> = self
            .tables
            .values()
            .filter(|t| t.source_id == id)
            .map(|t| t.id)
            .collect();
        for t in ids {
            if let Some(tab) = self.tables.get_mut(&t) {
                tab.error = None;
            }
            if self.tables[&t].data.is_some() {
                self.load(t)?;
            }
        }
        self.emit(CoreEvent::WorkspaceChanged);
        Ok(())
    }

    pub fn ensure_loaded(&mut self, id: TableId) -> Result<()> {
        if self.table(id)?.data.is_some() {
            return Ok(());
        }
        self.load(id)
    }

    /// Reads the file of a table into its working table. Clears the history.
    pub fn load(&mut self, id: TableId) -> Result<()> {
        let (sid, sheet, name) = {
            let t = self.table(id)?;
            (t.source_id.clone(), t.sheet.clone(), t.sql_name.clone())
        };
        let src = self.ws.source(&sid)?.clone();
        let opts = src.read_options(sheet.as_deref());
        let path = src.path_buf();
        let sink = self.sink.clone();
        let task = format!("load:{id}");
        let last = std::cell::Cell::new(Instant::now());
        let progress = |done: u64, total: Option<u64>| {
            if last.get().elapsed() > Duration::from_millis(100) {
                last.set(Instant::now());
                sink.emit(CoreEvent::Progress {
                    task: task.clone(),
                    done,
                    total,
                });
            }
        };
        let ctl = LoadControl {
            cancel: &self.cancel,
            progress: &progress,
        };
        let result: Result<Loaded> = match (src.kind, sheet.as_deref()) {
            (SourceKind::Csv, _) => ingest::load_csv(&self.conn, id, &name, &path, &opts, &ctl),
            (SourceKind::Excel, Some(sh)) => {
                ingest::excel::load_sheet(&self.conn, id, &name, &path, sh, &opts, &ctl)
            }
            (SourceKind::Excel, None) => {
                Err(CoreError::Internal("an Excel table needs a sheet".into()))
            }
        };
        // Forget the old data and views of the table.
        let old_views: Vec<ViewId> = lock(&self.views)
            .iter()
            .filter(|(_, v)| matches!(v, ViewMeta::Grid { table, .. } if *table == id))
            .map(|(k, _)| *k)
            .collect();
        match result {
            Ok(l) => {
                let state = self.new_state();
                let save_as_only = src.kind == SourceKind::Excel || src.is_read_only_format();
                let t = self.table_mut(id)?;
                let version = t.data.as_ref().map(|d| d.version + 1).unwrap_or(1);
                t.data = Some(TableData {
                    columns: l.columns,
                    dialect: l.dialect,
                    fingerprint: l.fingerprint,
                    warnings: l.warnings,
                    notes: l.notes,
                    undo: Vec::new(),
                    redo: Vec::new(),
                    state,
                    saved_state: state,
                    version,
                    row_count: l.row_count,
                    save_as_only,
                });
                t.error = None;
                t.externally_modified = false;
                self.publish_fetch(id);
                for v in old_views {
                    let _ = self.rebuild_view(v);
                }
                self.emit_changed(id);
                Ok(())
            }
            Err(e) => {
                let t = self.table_mut(id)?;
                if t.data.is_none() || !matches!(e, CoreError::Cancelled) {
                    t.error = Some(e.to_payload());
                }
                if !matches!(e, CoreError::Cancelled) {
                    // The transaction was rolled back, so an older working table can remain.
                    if t.data.is_some() {
                        t.data = None;
                        let _ = ingest::drop_internal(&self.conn, id, &name);
                        lock(&self.fetch).remove(&id);
                    }
                }
                self.emit(CoreEvent::WorkspaceChanged);
                Err(e)
            }
        }
    }

    /// Updates the snapshot that the read connection uses.
    pub fn publish_fetch(&self, id: TableId) {
        let Some(t) = self.tables.get(&id) else {
            return;
        };
        let Some(d) = &t.data else {
            lock(&self.fetch).remove(&id);
            return;
        };
        lock(&self.fetch).insert(
            id,
            FetchMeta {
                sql_name: t.sql_name.clone(),
                id,
                columns: d.columns.clone(),
                row_count: d.row_count,
                notes: Arc::new(d.notes.clone()),
            },
        );
    }

    pub fn emit_changed(&self, id: TableId) {
        if let Ok(d) = self.data(id) {
            self.emit(CoreEvent::TableChanged {
                table_id: id,
                version: d.version,
                dirty: d.dirty(),
                row_count: d.row_count,
            });
        }
    }

    pub fn count_rows(&self, id: TableId) -> Result<u64> {
        let name = &self.table(id)?.sql_name;
        let n: i64 = self.conn.query_row(
            &format!("SELECT count(*) FROM {}", quote_ident(name)),
            [],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    fn check_external_changes(&mut self) -> Result<Vec<TableId>> {
        if self.ws.reload_if_changed()? {
            self.sync_tables();
            self.emit(CoreEvent::WorkspaceChanged);
        }
        let mut changed = Vec::new();
        let ids: Vec<TableId> = self.tables.keys().copied().collect();
        let mut sheets_checked: Vec<SourceId> = Vec::new();
        for id in ids {
            let t = &self.tables[&id];
            let Some(d) = &t.data else { continue };
            let src = self.ws.source(&t.source_id)?;
            let path = src.path_buf();
            if !d.fingerprint.metadata_differs(&path) {
                continue;
            }
            let Ok(fp) = Fingerprint::of(&path) else {
                continue;
            };
            if fp == d.fingerprint {
                continue;
            }
            let dirty = d.dirty();
            let kind = src.kind;
            let sid = t.source_id.clone();
            changed.push(id);
            if kind == SourceKind::Excel && !sheets_checked.contains(&sid) {
                sheets_checked.push(sid.clone());
                if let Ok(list) = ingest::excel::list_sheets(&path) {
                    if self.ws.sync_sheets(&sid, &list)? {
                        self.ws.save()?;
                        self.sync_tables();
                        self.emit(CoreEvent::WorkspaceChanged);
                    }
                }
            }
            if !self.tables.contains_key(&id) {
                continue;
            }
            if dirty {
                self.table_mut(id)?.externally_modified = true;
                self.emit(CoreEvent::WorkspaceChanged);
            } else if let Err(e) = self.load(id) {
                tracing::warn!("cannot reload table {id}: {e}");
            }
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests;
