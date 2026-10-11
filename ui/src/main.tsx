import "@glideapps/glide-data-grid/dist/index.css";
import "./styles.css";
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { api } from "./api";
import { openTable, refreshSources } from "./actions";
import { defaultLanguage, initI18n } from "./i18n";
import { useStore, type Layout, type Settings } from "./store";

/** The content of ui-state.json (04. Workspace, section 5.2). */
interface UiState {
  settings?: Partial<Settings>;
  layout?: Partial<Layout>;
  sqlText?: string;
  history?: string[];
  /** Open tabs, by source ID and sheet name (table IDs change at each start). */
  tabs?: { sourceId: string; sheet: string | null }[];
  active?: { sourceId: string; sheet: string | null } | null;
}

async function restore() {
  const saved = ((await api.loadUiState().catch(() => null)) ?? {}) as UiState;
  const s = useStore.getState();
  const settings = { ...s.settings, language: defaultLanguage(), ...saved.settings };
  useStore.setState({
    settings,
    layout: { ...s.layout, ...saved.layout },
    sqlText: saved.sqlText ?? "",
    history: saved.history ?? [],
  });
  initI18n(settings.language);
  await refreshSources();
  const sources = useStore.getState().sources;
  const find = (ref: { sourceId: string; sheet: string | null }) =>
    sources.find((x) => x.id === ref.sourceId)?.tables.find((tb) => (tb.sheet ?? null) === (ref.sheet ?? null))?.id;
  for (const tab of saved.tabs ?? []) {
    const id = find(tab);
    if (id != null) useStore.getState().openTab(id);
  }
  const activeId = saved.active ? find(saved.active) : undefined;
  const target = activeId ?? useStore.getState().tabs[0]?.tableId;
  if (target != null) void openTable(target);
}

function persist() {
  let timer: ReturnType<typeof setTimeout> | undefined;
  useStore.subscribe((s, prev) => {
    if (
      s.settings === prev.settings &&
      s.layout === prev.layout &&
      s.sqlText === prev.sqlText &&
      s.history === prev.history &&
      s.tabs === prev.tabs &&
      s.activeTable === prev.activeTable
    ) {
      return;
    }
    clearTimeout(timer);
    timer = setTimeout(() => {
      const st = useStore.getState();
      const ref = (id: number) => {
        for (const src of st.sources) {
          const tb = src.tables.find((x) => x.id === id);
          if (tb) return { sourceId: src.id, sheet: tb.sheet };
        }
        return null;
      };
      const state: UiState = {
        settings: st.settings,
        layout: st.layout,
        sqlText: st.sqlText,
        history: st.history,
        tabs: st.tabs.map((t) => ref(t.tableId)).filter((x): x is { sourceId: string; sheet: string | null } => !!x),
        active: st.activeTable != null ? ref(st.activeTable) : null,
      };
      void api.saveUiState(state).catch(() => {});
    }, 800);
  });
}

void restore().finally(() => {
  persist();
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
});
