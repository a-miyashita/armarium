//! Tauri commands: thin adapters over the engine.
//!
//! Each command runs the engine call on a blocking thread, so the IPC thread
//! does not wait for long operations.

use crate::AppState;
use armarium_core::engine::{AddSourceResult, CompletionTable};
use armarium_core::error::ErrorPayload;
use armarium_core::model::*;
use armarium_core::sql::{SqlOptions, SqlRun};
use armarium_core::{CoreError, Engine};
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

type CmdResult<T> = Result<T, ErrorPayload>;

async fn blocking<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&Engine) -> Result<T, CoreError> + Send + 'static,
) -> CmdResult<T> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || f(&engine))
        .await
        .map_err(|e| CoreError::Internal(e.to_string()).to_payload())?
        .map_err(|e| e.to_payload())
}

#[tauri::command]
pub async fn list_sources(state: State<'_, AppState>) -> CmdResult<Vec<SourceInfo>> {
    blocking(&state, |e| e.list_sources()).await
}

#[tauri::command]
pub async fn add_sources(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> CmdResult<Vec<AddSourceResult>> {
    blocking(&state, move |e| {
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        e.add_sources(&paths)
    })
    .await
}

#[tauri::command]
pub async fn remove_source(state: State<'_, AppState>, source_id: String) -> CmdResult<()> {
    blocking(&state, move |e| e.remove_source(&source_id)).await
}

#[tauri::command]
pub async fn rename_table(
    state: State<'_, AppState>,
    table_id: TableId,
    name: String,
) -> CmdResult<TableInfo> {
    blocking(&state, move |e| e.rename_table(table_id, &name)).await
}

#[tauri::command]
pub async fn relocate_source(
    state: State<'_, AppState>,
    source_id: String,
    path: String,
) -> CmdResult<()> {
    blocking(&state, move |e| {
        e.relocate_source(&source_id, &PathBuf::from(path))
    })
    .await
}

#[tauri::command]
pub async fn get_read_options(
    state: State<'_, AppState>,
    table_id: TableId,
) -> CmdResult<ReadOptions> {
    blocking(&state, move |e| e.read_options(table_id)).await
}

#[tauri::command]
pub async fn set_read_options(
    state: State<'_, AppState>,
    table_id: TableId,
    options: ReadOptions,
) -> CmdResult<TableMeta> {
    blocking(&state, move |e| e.set_read_options(table_id, options)).await
}

#[tauri::command]
pub async fn open_table(state: State<'_, AppState>, table_id: TableId) -> CmdResult<TableMeta> {
    blocking(&state, move |e| e.open_table(table_id)).await
}

#[tauri::command]
pub async fn table_meta(state: State<'_, AppState>, table_id: TableId) -> CmdResult<TableMeta> {
    blocking(&state, move |e| e.table_meta(table_id)).await
}

#[tauri::command]
pub async fn reload_table(state: State<'_, AppState>, table_id: TableId) -> CmdResult<TableMeta> {
    blocking(&state, move |e| e.reload_table(table_id)).await
}

#[tauri::command]
pub async fn fetch_rows(
    state: State<'_, AppState>,
    table_id: TableId,
    view_id: Option<ViewId>,
    offset: u64,
    limit: u32,
) -> CmdResult<RowPage> {
    blocking(&state, move |e| {
        e.fetch_rows(table_id, view_id, offset, limit)
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewCreated {
    view_id: ViewId,
    total: u64,
}

#[tauri::command]
pub async fn create_view(
    state: State<'_, AppState>,
    table_id: TableId,
    spec: ViewSpec,
) -> CmdResult<ViewCreated> {
    blocking(&state, move |e| {
        let (view_id, total) = e.create_view(table_id, spec)?;
        Ok(ViewCreated { view_id, total })
    })
    .await
}

#[tauri::command]
pub async fn close_view(state: State<'_, AppState>, view_id: ViewId) -> CmdResult<()> {
    blocking(&state, move |e| e.close_view(view_id)).await
}

#[tauri::command]
pub async fn apply_edit(
    state: State<'_, AppState>,
    table_id: TableId,
    op: EditOp,
) -> CmdResult<EditResult> {
    blocking(&state, move |e| e.apply_edit(table_id, op)).await
}

#[tauri::command]
pub async fn undo(state: State<'_, AppState>, table_id: TableId) -> CmdResult<TableMeta> {
    blocking(&state, move |e| e.undo(table_id)).await
}

#[tauri::command]
pub async fn redo(state: State<'_, AppState>, table_id: TableId) -> CmdResult<TableMeta> {
    blocking(&state, move |e| e.redo(table_id)).await
}

#[tauri::command]
pub async fn save_table(
    state: State<'_, AppState>,
    table_id: TableId,
    force: bool,
) -> CmdResult<TableMeta> {
    blocking(&state, move |e| e.save_table(table_id, force)).await
}

#[tauri::command]
pub async fn save_table_as(
    state: State<'_, AppState>,
    table_id: TableId,
    path: String,
    format: Option<ExportFormat>,
) -> CmdResult<TableMeta> {
    blocking(&state, move |e| {
        e.save_table_as(table_id, &PathBuf::from(path), format)
    })
    .await
}

#[tauri::command]
pub async fn export_table(
    state: State<'_, AppState>,
    table_id: TableId,
    path: String,
    format: ExportFormat,
) -> CmdResult<()> {
    blocking(&state, move |e| {
        e.export_table(table_id, &PathBuf::from(path), format)
    })
    .await
}

#[tauri::command]
pub async fn export_result(
    state: State<'_, AppState>,
    result_id: ViewId,
    path: String,
    format: ExportFormat,
) -> CmdResult<()> {
    blocking(&state, move |e| {
        e.export_result(result_id, &PathBuf::from(path), format)
    })
    .await
}

#[tauri::command]
pub async fn save_result_as_source(
    state: State<'_, AppState>,
    result_id: ViewId,
    path: String,
    format: ExportFormat,
) -> CmdResult<String> {
    blocking(&state, move |e| {
        e.save_result_as_source(result_id, &PathBuf::from(path), format)
    })
    .await
}

#[tauri::command]
pub async fn run_sql(
    state: State<'_, AppState>,
    text: String,
    allow_no_undo: bool,
) -> CmdResult<SqlRun> {
    blocking(&state, move |e| {
        if text.len() < 4096 {
            tracing::debug!(sql = %text, "run_sql");
        }
        Ok(e.run_sql(
            &text,
            &SqlOptions {
                read_only: false,
                allow_no_undo,
            },
        ))
    })
    .await
}

#[tauri::command]
pub async fn fetch_result(
    state: State<'_, AppState>,
    result_id: ViewId,
    offset: u64,
    limit: u32,
) -> CmdResult<RowPage> {
    blocking(&state, move |e| e.fetch_result(result_id, offset, limit)).await
}

#[tauri::command]
pub fn interrupt(state: State<'_, AppState>) {
    // Runs on the IPC thread, so it does not wait for the running operation.
    state.engine.interrupt();
}

#[tauri::command]
pub async fn completion(state: State<'_, AppState>) -> CmdResult<Vec<CompletionTable>> {
    blocking(&state, |e| e.completion()).await
}

#[tauri::command]
pub async fn dirty_tables(state: State<'_, AppState>) -> CmdResult<Vec<TableInfo>> {
    blocking(&state, |e| e.dirty_tables()).await
}

#[tauri::command]
pub async fn check_external_changes(state: State<'_, AppState>) -> CmdResult<Vec<TableId>> {
    blocking(&state, |e| e.check_external_changes()).await
}

#[tauri::command]
pub async fn diff_table(state: State<'_, AppState>, table_id: TableId) -> CmdResult<TableDiff> {
    blocking(&state, move |e| e.diff_table(table_id, 100)).await
}

/// Shows the file in Explorer (`explorer /select,`).
#[tauri::command]
pub fn show_in_explorer(path: String) -> CmdResult<()> {
    let p = PathBuf::from(&path);
    #[cfg(windows)]
    let r = std::process::Command::new("explorer")
        .arg(format!("/select,{}", p.display()))
        .spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg("-R").arg(&p).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open")
        .arg(p.parent().unwrap_or(std::path::Path::new("/")))
        .spawn();
    r.map(|_| ())
        .map_err(|e| CoreError::from_io(e, &p).to_payload())
}

fn ui_state_path() -> PathBuf {
    armarium_core::fsutil::config_dir().join("ui-state.json")
}

/// Reads `ui-state.json` (window state, open tabs, SQL text, history). The content
/// belongs to the frontend.
#[tauri::command]
pub fn load_ui_state() -> CmdResult<serde_json::Value> {
    match std::fs::read_to_string(ui_state_path()) {
        Ok(t) => Ok(serde_json::from_str(&t).unwrap_or(serde_json::Value::Null)),
        Err(_) => Ok(serde_json::Value::Null),
    }
}

#[tauri::command]
pub async fn save_ui_state(state: serde_json::Value) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = ui_state_path();
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let text =
            serde_json::to_string_pretty(&state).map_err(|e| CoreError::Internal(e.to_string()))?;
        armarium_core::fsutil::atomic_write(&path, |w| {
            use std::io::Write;
            w.write_all(text.as_bytes())?;
            Ok(())
        })
    })
    .await
    .map_err(|e| CoreError::Internal(e.to_string()).to_payload())?
    .map_err(|e| e.to_payload())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpInfo {
    /// The path of `armarium-mcp` next to the application, if it exists.
    command: Option<String>,
    workspace: String,
}

/// Information for the MCP client configuration (08. MCP server, section 2.1).
#[tauri::command]
pub fn mcp_info(state: State<'_, AppState>) -> McpInfo {
    let exe = std::env::current_exe().ok().and_then(|p| {
        let name = if cfg!(windows) {
            "armarium-mcp.exe"
        } else {
            "armarium-mcp"
        };
        let candidate = p.parent()?.join(name);
        candidate.exists().then(|| candidate.display().to_string())
    });
    McpInfo {
        command: exe,
        workspace: state.engine.workspace_path().display().to_string(),
    }
}
