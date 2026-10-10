// Typed functions that call the Tauri commands (src-tauri/src/commands.rs).
// The types come from Rust through ts-rs (src/bindings).
import { invoke } from "@tauri-apps/api/core";
import type { AddSourceResult } from "./bindings/AddSourceResult";
import type { CompletionTable } from "./bindings/CompletionTable";
import type { EditOp } from "./bindings/EditOp";
import type { EditResult } from "./bindings/EditResult";
import type { ErrorPayload } from "./bindings/ErrorPayload";
import type { ExportFormat } from "./bindings/ExportFormat";
import type { ReadOptions } from "./bindings/ReadOptions";
import type { RowPage } from "./bindings/RowPage";
import type { SourceInfo } from "./bindings/SourceInfo";
import type { SqlRun } from "./bindings/SqlRun";
import type { TableDiff } from "./bindings/TableDiff";
import type { TableInfo } from "./bindings/TableInfo";
import type { TableMeta } from "./bindings/TableMeta";
import type { ViewSpec } from "./bindings/ViewSpec";

/** An error from the core, with a stable code (02. Architecture, section 7). */
export class CoreError extends Error {
  code: string;
  constructor(payload: ErrorPayload) {
    super(payload.message);
    this.code = payload.code;
  }
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    if (e && typeof e === "object" && "code" in e && "message" in e) {
      throw new CoreError(e as ErrorPayload);
    }
    throw new CoreError({ code: "INTERNAL", message: String(e) });
  }
}

export const api = {
  listSources: () => call<SourceInfo[]>("list_sources"),
  addSources: (paths: string[]) => call<AddSourceResult[]>("add_sources", { paths }),
  removeSource: (sourceId: string) => call<void>("remove_source", { sourceId }),
  renameTable: (tableId: number, name: string) => call<TableInfo>("rename_table", { tableId, name }),
  relocateSource: (sourceId: string, path: string) => call<void>("relocate_source", { sourceId, path }),
  getReadOptions: (tableId: number) => call<ReadOptions>("get_read_options", { tableId }),
  setReadOptions: (tableId: number, options: ReadOptions) =>
    call<TableMeta>("set_read_options", { tableId, options }),
  openTable: (tableId: number) => call<TableMeta>("open_table", { tableId }),
  tableMeta: (tableId: number) => call<TableMeta>("table_meta", { tableId }),
  reloadTable: (tableId: number) => call<TableMeta>("reload_table", { tableId }),
  fetchRows: (tableId: number, viewId: number | null, offset: number, limit: number) =>
    call<RowPage>("fetch_rows", { tableId, viewId, offset, limit }),
  createView: (tableId: number, spec: ViewSpec) =>
    call<{ viewId: number; total: number }>("create_view", { tableId, spec }),
  closeView: (viewId: number) => call<void>("close_view", { viewId }),
  applyEdit: (tableId: number, op: EditOp) => call<EditResult>("apply_edit", { tableId, op }),
  undo: (tableId: number) => call<TableMeta>("undo", { tableId }),
  redo: (tableId: number) => call<TableMeta>("redo", { tableId }),
  saveTable: (tableId: number, force: boolean) => call<TableMeta>("save_table", { tableId, force }),
  saveTableAs: (tableId: number, path: string, format: ExportFormat | null) =>
    call<TableMeta>("save_table_as", { tableId, path, format }),
  exportTable: (tableId: number, path: string, format: ExportFormat) =>
    call<void>("export_table", { tableId, path, format }),
  exportResult: (resultId: number, path: string, format: ExportFormat) =>
    call<void>("export_result", { resultId, path, format }),
  saveResultAsSource: (resultId: number, path: string, format: ExportFormat) =>
    call<string>("save_result_as_source", { resultId, path, format }),
  runSql: (text: string, allowNoUndo = false) => call<SqlRun>("run_sql", { text, allowNoUndo }),
  fetchResult: (resultId: number, offset: number, limit: number) =>
    call<RowPage>("fetch_result", { resultId, offset, limit }),
  interrupt: () => call<void>("interrupt"),
  completion: () => call<CompletionTable[]>("completion"),
  dirtyTables: () => call<TableInfo[]>("dirty_tables"),
  checkExternalChanges: () => call<number[]>("check_external_changes"),
  diffTable: (tableId: number) => call<TableDiff>("diff_table", { tableId }),
  showInExplorer: (path: string) => call<void>("show_in_explorer", { path }),
  loadUiState: () => call<unknown>("load_ui_state"),
  saveUiState: (state: unknown) => call<void>("save_ui_state", { state }),
  mcpInfo: () => call<{ command: string | null; workspace: string }>("mcp_info"),
};

/** The default export format for a file name. */
export function formatForPath(path: string, encoding = "UTF-8", bom = true): ExportFormat {
  const lower = path.toLowerCase();
  if (lower.endsWith(".xlsx")) return { format: "xlsx", sheetName: null };
  const tsv = lower.endsWith(".tsv") || lower.endsWith(".tab");
  return {
    format: "csv",
    encoding,
    bom,
    delimiter: tsv ? "\t" : ",",
    lineEnding: "crlf",
    quoteAll: false,
    header: true,
  };
}
