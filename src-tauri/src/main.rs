//! The Armarium desktop application (Tauri 2).
//!
//! The commands are thin adapters over `armarium-core` ([ADR-0004]).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod watcher;

use armarium_core::model::CoreEvent;
use armarium_core::{Engine, EngineConfig, EventSink};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

/// Sends core events to the WebView as `core-event`.
struct TauriSink {
    app: AppHandle,
    watcher: watcher::Trigger,
}

impl EventSink for TauriSink {
    fn emit(&self, event: CoreEvent) {
        if matches!(event, CoreEvent::WorkspaceChanged) {
            self.watcher.refresh_paths();
        }
        if let Err(e) = self.app.emit("core-event", &event) {
            tracing::warn!("cannot emit an event: {e}");
        }
    }
}

pub struct AppState {
    pub engine: Arc<Engine>,
}

fn init_logs() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = armarium_core::fsutil::local_data_dir().join("logs");
    let filter = tracing_subscriber::EnvFilter::try_from_env("ARMARIUM_LOG")
        .unwrap_or_else(|_| "info".into());
    match std::fs::create_dir_all(&dir) {
        Ok(()) => {
            let appender = tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("armarium")
                .filename_suffix("log")
                .max_log_files(14)
                .build(&dir)
                .ok()?;
            let (writer, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(writer)
                .init();
            Some(guard)
        }
        Err(_) => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(std::io::stderr)
                .init();
            None
        }
    }
}

fn main() {
    let _log_guard = init_logs();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let trigger = watcher::Trigger::new();
            let sink = Arc::new(TauriSink {
                app: app.handle().clone(),
                watcher: trigger.clone(),
            });
            let engine = Arc::new(Engine::new(EngineConfig::default(), sink)?);
            watcher::start(engine.clone(), trigger);
            app.manage(AppState { engine });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_sources,
            commands::add_sources,
            commands::remove_source,
            commands::rename_table,
            commands::relocate_source,
            commands::get_read_options,
            commands::set_read_options,
            commands::open_table,
            commands::table_meta,
            commands::reload_table,
            commands::fetch_rows,
            commands::create_view,
            commands::close_view,
            commands::apply_edit,
            commands::undo,
            commands::redo,
            commands::save_table,
            commands::save_table_as,
            commands::export_table,
            commands::export_result,
            commands::save_result_as_source,
            commands::run_sql,
            commands::fetch_result,
            commands::interrupt,
            commands::completion,
            commands::dirty_tables,
            commands::check_external_changes,
            commands::diff_table,
            commands::show_in_explorer,
            commands::load_ui_state,
            commands::save_ui_state,
            commands::mcp_info,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Armarium application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<AppState>() {
                    state.engine.shutdown();
                }
            }
        });
}
