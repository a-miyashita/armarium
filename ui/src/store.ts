// Application state (Zustand).
import { create } from "zustand";
import type { SourceInfo } from "./bindings/SourceInfo";
import type { StatementResult } from "./bindings/StatementResult";
import type { TableMeta } from "./bindings/TableMeta";
import type { ViewSpec } from "./bindings/ViewSpec";
import type { SqlError } from "./bindings/SqlError";

export interface TableTab {
  tableId: number;
  viewId: number | null;
  viewSpec: ViewSpec | null;
  viewTotal: number | null;
}

export interface Settings {
  language: string;
  theme: "light" | "dark" | "system";
  nullDisplay: "empty" | "null";
}

export interface Layout {
  leftWidth: number;
  leftCollapsed: boolean;
  sqlHeight: number;
  sqlVisible: boolean;
}

export interface DialogButton {
  id: string;
  label: string;
  primary?: boolean;
}

export interface DialogRequest {
  title: string;
  body?: string;
  list?: string[];
  buttons: DialogButton[];
  input?: { label: string; value: string };
  resolve: (result: { button: string; value?: string }) => void;
}

export interface State {
  sources: SourceInfo[];
  metas: Record<number, TableMeta>;
  tabs: TableTab[];
  activeTable: number | null;
  settings: Settings;
  layout: Layout;
  sqlText: string;
  history: string[];
  results: StatementResult[];
  sqlError: SqlError | null;
  activeResult: number | null;
  running: boolean;
  selectionCount: number;
  /** Incremented when a table changes, so that the grid fetches again. */
  versions: Record<number, number>;
  dialogs: DialogRequest[];
  message: { kind: "error" | "info"; text: string } | null;
  progress: { task: string; done: number; total: number | null } | null;
  modal: string | null;

  set: (patch: Partial<State>) => void;
  setMeta: (meta: TableMeta) => void;
  bump: (tableId: number) => void;
  openTab: (tableId: number) => void;
  closeTab: (tableId: number) => void;
  updateTab: (tableId: number, patch: Partial<TableTab>) => void;
  setSettings: (patch: Partial<Settings>) => void;
  setLayout: (patch: Partial<Layout>) => void;
  addHistory: (sql: string) => void;
}

export const useStore = create<State>((set) => ({
  sources: [],
  metas: {},
  tabs: [],
  activeTable: null,
  settings: { language: "en", theme: "system", nullDisplay: "empty" },
  layout: { leftWidth: 260, leftCollapsed: false, sqlHeight: 260, sqlVisible: true },
  sqlText: "",
  history: [],
  results: [],
  sqlError: null,
  activeResult: null,
  running: false,
  selectionCount: 0,
  versions: {},
  dialogs: [],
  message: null,
  progress: null,
  modal: null,

  set: (patch) => set(patch),
  setMeta: (meta) => set((s) => ({ metas: { ...s.metas, [meta.id]: meta } })),
  bump: (tableId) => set((s) => ({ versions: { ...s.versions, [tableId]: (s.versions[tableId] ?? 0) + 1 } })),
  openTab: (tableId) =>
    set((s) => ({
      tabs: s.tabs.some((t) => t.tableId === tableId)
        ? s.tabs
        : [...s.tabs, { tableId, viewId: null, viewSpec: null, viewTotal: null }],
      activeTable: tableId,
    })),
  closeTab: (tableId) =>
    set((s) => {
      const i = s.tabs.findIndex((t) => t.tableId === tableId);
      const tabs = s.tabs.filter((t) => t.tableId !== tableId);
      let active = s.activeTable;
      if (active === tableId) {
        active = tabs.length ? tabs[Math.max(0, i - 1)].tableId : null;
      }
      return { tabs, activeTable: active };
    }),
  updateTab: (tableId, patch) =>
    set((s) => ({ tabs: s.tabs.map((t) => (t.tableId === tableId ? { ...t, ...patch } : t)) })),
  setSettings: (patch) => set((s) => ({ settings: { ...s.settings, ...patch } })),
  setLayout: (patch) => set((s) => ({ layout: { ...s.layout, ...patch } })),
  addHistory: (sql) =>
    set((s) => ({ history: [sql, ...s.history.filter((h) => h !== sql)].slice(0, 100) })),
}));

/** Shows a modal dialog and waits for the answer. */
export function ask(req: Omit<DialogRequest, "resolve">): Promise<{ button: string; value?: string }> {
  return new Promise((resolve) => {
    useStore.setState((s) => ({ dialogs: [...s.dialogs, { ...req, resolve }] }));
  });
}

/** Asks for a line of text. Returns null if the user cancels. */
export async function promptText(title: string, label: string, value: string, ok: string, cancel: string) {
  const r = await ask({
    title,
    input: { label, value },
    buttons: [
      { id: "cancel", label: cancel },
      { id: "ok", label: ok, primary: true },
    ],
  });
  return r.button === "ok" ? (r.value ?? "") : null;
}

export function showError(e: unknown) {
  const text = e instanceof Error ? e.message : String(e);
  useStore.setState({ message: { kind: "error", text } });
}

export function showInfo(text: string) {
  useStore.setState({ message: { kind: "info", text } });
}

/** Finds a table and its source in the source list. */
export function findTable(sources: SourceInfo[], tableId: number) {
  for (const s of sources) {
    const t = s.tables.find((t) => t.id === tableId);
    if (t) return { source: s, table: t };
  }
  return undefined;
}
