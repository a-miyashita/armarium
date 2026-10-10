// User operations: they call the API, update the store, and show dialogs.
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { api, CoreError, formatForPath } from "./api";
import type { EditOp } from "./bindings/EditOp";
import type { ExportFormat } from "./bindings/ExportFormat";
import type { LogicalType } from "./bindings/LogicalType";
import type { ViewSpec } from "./bindings/ViewSpec";
import i18n from "./i18n";
import { ask, findTable, promptText, showError, showInfo, useStore } from "./store";

const t = (key: string, opts?: Record<string, unknown>) => i18n.t(key, opts);

const FILE_FILTERS = [
  { name: "CSV, TSV, Excel", extensions: ["csv", "tsv", "tab", "txt", "xlsx", "xlsm", "xls", "xlsb", "ods"] },
  { name: "All files", extensions: ["*"] },
];

export async function refreshSources() {
  try {
    const sources = await api.listSources();
    const s = useStore.getState();
    // Close tabs of tables that do not exist any more.
    const ids = new Set(sources.flatMap((x) => x.tables.map((t) => t.id)));
    for (const tab of s.tabs) {
      if (!ids.has(tab.tableId)) s.closeTab(tab.tableId);
    }
    useStore.setState({ sources });
  } catch (e) {
    showError(e);
  }
}

export async function addFiles(paths?: string[]) {
  let list = paths;
  if (!list) {
    const picked = await openDialog({ multiple: true, filters: FILE_FILTERS });
    if (!picked) return;
    list = Array.isArray(picked) ? picked : [picked];
  }
  try {
    const results = await api.addSources(list);
    await refreshSources();
    const errors = results.filter((r) => r.error).map((r) => `${r.path}: ${r.error!.message}`);
    if (errors.length) showError(errors.join("\n"));
    // Open the first table of the first file.
    const first = results.find((r) => r.sourceId);
    if (first) {
      const src = useStore.getState().sources.find((s) => s.id === first.sourceId);
      if (src && src.tables.length) await openTable(src.tables[0].id);
    }
  } catch (e) {
    showError(e);
  }
}

export async function openTable(tableId: number) {
  const s = useStore.getState();
  s.openTab(tableId);
  try {
    const meta = await api.openTable(tableId);
    s.setMeta(meta);
    s.bump(tableId);
  } catch (e) {
    showError(e);
  } finally {
    await refreshSources();
  }
}

export async function refreshMeta(tableId: number) {
  try {
    const meta = await api.tableMeta(tableId);
    useStore.getState().setMeta(meta);
  } catch {
    /* The table is not loaded. */
  }
}

function activeTable(): number | null {
  return useStore.getState().activeTable;
}

/** Applies an edit. For a type mismatch in a cell, the user can change the column to Text. */
export async function applyEdit(tableId: number, op: EditOp): Promise<boolean> {
  try {
    const r = await api.applyEdit(tableId, op);
    useStore.getState().setMeta(r.table);
    return true;
  } catch (e) {
    if (e instanceof CoreError && e.code === "TYPE_MISMATCH" && op.op === "setCells" && op.cells.length === 1) {
      const r = await ask({
        title: t("dialog.typeMismatchTitle"),
        body: t("dialog.typeMismatchBody", { message: e.message }),
        buttons: [
          { id: "cancel", label: t("dialog.cancel") },
          { id: "text", label: t("dialog.changeToText"), primary: true },
        ],
      });
      if (r.button === "text") {
        try {
          await api.applyEdit(tableId, { op: "changeColumnType", column: op.cells[0].column, logicalType: "text" });
          const res = await api.applyEdit(tableId, op);
          useStore.getState().setMeta(res.table);
          return true;
        } catch (e2) {
          showError(e2);
        }
      }
      return false;
    }
    showError(e);
    return false;
  }
}

/** Creates the sort or filter view of a tab again (after rows were added or deleted). */
export async function refreshView(tableId: number) {
  const tab = useStore.getState().tabs.find((x) => x.tableId === tableId);
  if (tab?.viewSpec) await setView(tableId, tab.viewSpec);
  else useStore.getState().bump(tableId);
}

export async function undo(tableId = activeTable()) {
  if (tableId == null) return;
  try {
    useStore.getState().setMeta(await api.undo(tableId));
    await refreshView(tableId);
  } catch (e) {
    showError(e);
  }
}

export async function redo(tableId = activeTable()) {
  if (tableId == null) return;
  try {
    useStore.getState().setMeta(await api.redo(tableId));
    await refreshView(tableId);
  } catch (e) {
    showError(e);
  }
}

function tableName(tableId: number): string {
  const found = findTable(useStore.getState().sources, tableId);
  if (!found) return String(tableId);
  return found.table.sheet ? `${found.source.fileName} / ${found.table.sheet}` : found.source.fileName;
}

/** Saves a table, with the dialogs of 03. UI, section 5. Returns true if saved. */
export async function saveTable(tableId = activeTable(), force = false): Promise<boolean> {
  if (tableId == null) return false;
  const meta = useStore.getState().metas[tableId];
  if (meta?.saveAsOnly) {
    showInfo(t("dialog.saveAsOnlyBody"));
    return saveTableAs(tableId);
  }
  try {
    useStore.getState().setMeta(await api.saveTable(tableId, force));
    await refreshSources();
    return true;
  } catch (e) {
    if (!(e instanceof CoreError)) {
      showError(e);
      return false;
    }
    if (e.code === "EXTERNAL_MODIFICATION") {
      const r = await ask({
        title: t("dialog.externalTitle"),
        body: t("dialog.externalBody", { name: tableName(tableId) }),
        buttons: [
          { id: "cancel", label: t("dialog.cancel") },
          { id: "saveas", label: t("dialog.saveAs") },
          { id: "reload", label: t("dialog.reloadDiscard") },
          { id: "overwrite", label: t("dialog.overwrite"), primary: true },
        ],
      });
      if (r.button === "overwrite") return saveTable(tableId, true);
      if (r.button === "reload") {
        await reloadTable(tableId, false);
        return false;
      }
      if (r.button === "saveas") return saveTableAs(tableId);
      return false;
    }
    if (e.code === "FILE_LOCKED") {
      const r = await ask({
        title: t("dialog.lockedTitle"),
        body: `${t("dialog.lockedBody")}\n${e.message}`,
        buttons: [
          { id: "cancel", label: t("dialog.cancel") },
          { id: "saveas", label: t("dialog.saveAs") },
          { id: "retry", label: t("dialog.retry"), primary: true },
        ],
      });
      if (r.button === "retry") return saveTable(tableId, force);
      if (r.button === "saveas") return saveTableAs(tableId);
      return false;
    }
    if (e.code === "UNENCODABLE") {
      const r = await ask({
        title: t("dialog.unencodableTitle"),
        body: t("dialog.unencodableBody", { message: e.message }),
        buttons: [
          { id: "cancel", label: t("dialog.cancel") },
          { id: "utf8", label: t("dialog.saveUtf8"), primary: true },
        ],
      });
      if (r.button === "utf8") {
        const found = findTable(useStore.getState().sources, tableId);
        const d = useStore.getState().metas[tableId]?.dialect;
        if (!found || !d) return false;
        const format: ExportFormat = {
          format: "csv",
          encoding: "UTF-8",
          bom: true,
          delimiter: d.delimiter,
          lineEnding: d.lineEnding,
          quoteAll: false,
          header: d.hasHeader,
        };
        try {
          useStore.getState().setMeta(await api.saveTableAs(tableId, found.source.path, format));
          await refreshSources();
          return true;
        } catch (e2) {
          showError(e2);
        }
      }
      return false;
    }
    showError(e);
    return false;
  }
}

export async function saveTableAs(tableId = activeTable()): Promise<boolean> {
  if (tableId == null) return false;
  const found = findTable(useStore.getState().sources, tableId);
  const isExcel = found?.source.kind === "excel";
  const base = found ? found.source.path.replace(/\.[^.\\/]+$/, "") : "table";
  const defaultPath = isExcel ? `${base}_${found?.table.sheet ?? "sheet"}.xlsx` : found?.source.path;
  const path = await saveDialog({
    defaultPath,
    filters: isExcel
      ? [{ name: "Excel", extensions: ["xlsx"] }, { name: "CSV", extensions: ["csv"] }, { name: "TSV", extensions: ["tsv"] }]
      : [{ name: "CSV", extensions: ["csv"] }, { name: "TSV", extensions: ["tsv"] }, { name: "Text", extensions: ["txt"] }],
  });
  if (!path) return false;
  try {
    useStore.getState().setMeta(await api.saveTableAs(tableId, path, isExcel ? formatForPath(path) : null));
    await refreshSources();
    return true;
  } catch (e) {
    showError(e);
    return false;
  }
}

export async function exportTo(kind: { table: number } | { result: number }, format: ExportFormat, path: string) {
  try {
    if ("table" in kind) await api.exportTable(kind.table, path, format);
    else await api.exportResult(kind.result, path, format);
    showInfo(path);
  } catch (e) {
    showError(e);
  }
}

export async function reloadTable(tableId: number, confirm = true) {
  const meta = useStore.getState().metas[tableId];
  if (confirm && meta?.dirty) {
    const r = await ask({
      title: t("tree.reload"),
      body: t("dialog.confirmReload"),
      buttons: [
        { id: "cancel", label: t("dialog.cancel") },
        { id: "ok", label: t("dialog.ok"), primary: true },
      ],
    });
    if (r.button !== "ok") return;
  }
  try {
    useStore.getState().setMeta(await api.reloadTable(tableId));
    useStore.getState().updateTab(tableId, { viewId: null, viewSpec: null, viewTotal: null });
    useStore.getState().bump(tableId);
  } catch (e) {
    showError(e);
  } finally {
    await refreshSources();
  }
}

export async function unregisterSource(sourceId: string) {
  const src = useStore.getState().sources.find((s) => s.id === sourceId);
  if (!src) return;
  const dirty = src.tables.some((t) => t.load.state === "loaded" && t.load.dirty);
  if (dirty) {
    const r = await ask({
      title: t("tree.unregister"),
      body: t("dialog.confirmUnregister", { name: src.fileName }),
      buttons: [
        { id: "cancel", label: t("dialog.cancel") },
        { id: "ok", label: t("tree.unregister"), primary: true },
      ],
    });
    if (r.button !== "ok") return;
  }
  try {
    for (const tb of src.tables) useStore.getState().closeTab(tb.id);
    await api.removeSource(sourceId);
  } catch (e) {
    showError(e);
  }
  await refreshSources();
}

export async function renameTable(tableId: number) {
  const found = findTable(useStore.getState().sources, tableId);
  if (!found) return;
  const name = await promptText(t("dialog.renameTitle"), "SQL", found.table.sqlName, t("dialog.ok"), t("dialog.cancel"));
  if (name == null || name === found.table.sqlName) return;
  try {
    await api.renameTable(tableId, name);
    await refreshSources();
    await refreshMeta(tableId);
  } catch (e) {
    showError(e);
  }
}

export async function relocateSource(sourceId: string) {
  const picked = await openDialog({ multiple: false, filters: FILE_FILTERS });
  if (!picked || Array.isArray(picked)) return;
  try {
    await api.relocateSource(sourceId, picked);
  } catch (e) {
    showError(e);
  }
  await refreshSources();
}

export async function setView(tableId: number, spec: ViewSpec | null) {
  const s = useStore.getState();
  const tab = s.tabs.find((x) => x.tableId === tableId);
  if (!tab) return;
  const old = tab.viewId;
  try {
    if (spec && (spec.sort.length || spec.filters.length)) {
      const { viewId, total } = await api.createView(tableId, spec);
      s.updateTab(tableId, { viewId, viewSpec: spec, viewTotal: total });
    } else {
      s.updateTab(tableId, { viewId: null, viewSpec: null, viewTotal: null });
    }
    s.bump(tableId);
    if (old != null) await api.closeView(old);
  } catch (e) {
    showError(e);
  }
}

export async function changeColumn(tableId: number, op: EditOp) {
  if (await applyEdit(tableId, op)) useStore.getState().bump(tableId);
}

export async function insertColumn(tableId: number, position: number) {
  const name = await promptText(t("dialog.newColumnTitle"), t("dialog.columnName"), "", t("dialog.ok"), t("dialog.cancel"));
  if (name == null) return;
  await changeColumn(tableId, { op: "insertColumn", name, logicalType: "text" as LogicalType, position });
}

export async function renameColumn(tableId: number, column: string, current: string) {
  const name = await promptText(t("dialog.renameColumnTitle"), t("dialog.columnName"), current, t("dialog.ok"), t("dialog.cancel"));
  if (name == null || name === current) return;
  await changeColumn(tableId, { op: "renameColumn", column, to: name });
}

/** Runs SQL. If a statement is too large to undo, asks the user and runs the rest again. */
export async function runSql(text: string) {
  const s = useStore.getState();
  if (!text.trim() || s.running) return;
  s.addHistory(text.trim());
  // Close the old results.
  for (const r of s.results) {
    if (r.resultId != null) void api.closeView(r.resultId).catch(() => {});
  }
  useStore.setState({ running: true, sqlError: null, results: [], activeResult: null });
  let pending = text;
  let allow = false;
  const all = [];
  try {
    for (;;) {
      const run = await api.runSql(pending, allow);
      all.push(...run.results);
      if (run.error?.code === "UNDO_LIMIT_EXCEEDED") {
        const r = await ask({
          title: t("sql.run"),
          body: t("sql.undoLimit"),
          buttons: [
            { id: "no", label: t("dialog.no") },
            { id: "yes", label: t("dialog.yes"), primary: true },
          ],
        });
        if (r.button === "yes") {
          // Continue from the statement that failed.
          const bytes = new TextEncoder().encode(pending);
          pending = new TextDecoder().decode(bytes.slice(run.error.byteOffset));
          allow = true;
          continue;
        }
      }
      useStore.setState({ sqlError: run.error });
      break;
    }
  } catch (e) {
    showError(e);
  } finally {
    const lastRows = [...all].reverse().find((r) => r.resultId != null);
    useStore.setState({ running: false, results: all, activeResult: lastRows?.resultId ?? null });
    // Tables that DML changed.
    const changed = new Set(all.filter((r) => r.kind === "changed").map((r) => (r as { tableId: number }).tableId));
    for (const id of changed) {
      await refreshMeta(id);
      await refreshView(id);
    }
    if (changed.size) await refreshSources();
  }
}

export async function saveResultAsSource(resultId: number) {
  const path = await saveDialog({
    filters: [{ name: "CSV", extensions: ["csv"] }, { name: "Excel", extensions: ["xlsx"] }, { name: "TSV", extensions: ["tsv"] }],
  });
  if (!path) return;
  try {
    await api.saveResultAsSource(resultId, path, formatForPath(path));
    await refreshSources();
  } catch (e) {
    showError(e);
  }
}

/** Saves all dirty tables. Returns false if one of them was not saved. */
export async function saveAll(): Promise<boolean> {
  const dirty = await api.dirtyTables();
  let ok = true;
  for (const tb of dirty) {
    if (!(await saveTable(tb.id))) ok = false;
  }
  return ok;
}

export function insertIntoSql(text: string) {
  window.dispatchEvent(new CustomEvent("armarium:insert-sql", { detail: text }));
}

export function quoteName(name: string): string {
  return /^[\p{L}_][\p{L}\p{N}_]*$/u.test(name) && !SQL_KEYWORDS.has(name.toLowerCase())
    ? name
    : `"${name.replace(/"/g, '""')}"`;
}

const SQL_KEYWORDS = new Set(
  "abort action add after all alter analyze and as asc attach autoincrement before begin between by cascade case cast check collate column commit conflict constraint create cross current current_date current_time current_timestamp database default deferrable deferred delete desc detach distinct do drop each else end escape except exclusive exists explain fail filter first following for foreign from full glob group having if ignore immediate in index indexed initially inner insert instead intersect into is isnull join key last left like limit match natural no not nothing notnull null nulls of offset on or order others outer over partition plan pragma preceding primary query raise range recursive references regexp reindex release rename replace restrict returning right rollback row rows savepoint select set table temp temporary then ties to transaction trigger unbounded union unique update using vacuum values view virtual when where window with without".split(
    " ",
  ),
);
