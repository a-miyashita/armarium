// The main window: toolbar, source tree, tabs, grid, SQL panel, and status bar (03. UI).
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { addFiles, redo, refreshMeta, refreshSources, saveAll, saveTable, saveTableAs, setView, undo } from "./actions";
import { api } from "./api";
import type { CoreEvent } from "./bindings/CoreEvent";
import { ContextMenu } from "./components/ContextMenu";
import { DialogHost } from "./components/DialogHost";
import { ExportDialog } from "./components/ExportDialog";
import { SettingsDialog } from "./components/SettingsDialog";
import { SourceTree } from "./components/SourceTree";
import { SqlPanel } from "./components/SqlPanel";
import { TableGrid } from "./components/TableGrid";
import { ask, findTable, useStore } from "./store";

function Toolbar({ onSettings, onExport }: { onSettings: () => void; onExport: () => void }) {
  const { t } = useTranslation();
  const active = useStore((s) => s.activeTable);
  const meta = useStore((s) => (s.activeTable != null ? s.metas[s.activeTable] : undefined));
  const layout = useStore((s) => s.layout);
  const [search, setSearch] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const onFind = () => searchRef.current?.focus();
    window.addEventListener("armarium:find", onFind);
    return () => window.removeEventListener("armarium:find", onFind);
  }, []);

  return (
    <div className="toolbar" role="toolbar">
      <button onClick={() => void addFiles()} title="Ctrl+O">
        {t("toolbar.add")}
      </button>
      <button onClick={() => void saveTable()} disabled={active == null} title="Ctrl+S">
        {t("toolbar.save")}
      </button>
      <button onClick={() => void saveTableAs()} disabled={active == null} title="Ctrl+Shift+S">
        {t("toolbar.saveAs")}
      </button>
      <button onClick={onExport} disabled={active == null}>
        {t("toolbar.export")}
      </button>
      <span className="sep" />
      <button onClick={() => void undo()} disabled={!meta?.canUndo} title="Ctrl+Z">
        ↶ {t("toolbar.undo")}
      </button>
      <button onClick={() => void redo()} disabled={!meta?.canRedo} title="Ctrl+Y">
        ↷ {t("toolbar.redo")}
      </button>
      <span className="sep" />
      <button
        aria-pressed={layout.sqlVisible}
        onClick={() => useStore.getState().setLayout({ sqlVisible: !layout.sqlVisible })}
      >
        {t("toolbar.sql")} ▶
      </button>
      <span className="grow" />
      <input
        ref={searchRef}
        className="search"
        type="search"
        placeholder={t("toolbar.search")}
        value={search}
        disabled={active == null}
        onChange={(e) => setSearch(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && active != null) {
            const text = search.trim();
            void setView(active, text ? { sort: [], filters: [{ column: null, op: "contains", value: text }] } : null);
          }
          if (e.key === "Escape" && active != null) {
            setSearch("");
            void setView(active, null);
          }
        }}
      />
      <button onClick={onSettings} title={t("toolbar.settings")}>
        ⚙
      </button>
    </div>
  );
}

function TabBar() {
  const tabs = useStore((s) => s.tabs);
  const active = useStore((s) => s.activeTable);
  const sources = useStore((s) => s.sources);
  const metas = useStore((s) => s.metas);
  return (
    <div className="tabbar" role="tablist">
      {tabs.map((tab) => {
        const found = findTable(sources, tab.tableId);
        const label = found ? (found.table.sheet ? `${found.source.fileName} / ${found.table.sheet}` : found.source.fileName) : "?";
        const dirty = metas[tab.tableId]?.dirty;
        return (
          <div
            key={tab.tableId}
            role="tab"
            aria-selected={tab.tableId === active}
            className={`tab ${tab.tableId === active ? "active" : ""}`}
            onClick={() => useStore.setState({ activeTable: tab.tableId })}
            onAuxClick={(e) => e.button === 1 && useStore.getState().closeTab(tab.tableId)}
            title={found?.source.path}
          >
            <span>{label}</span>
            {dirty && <span className="dirty">●</span>}
            <button
              className="tab-close"
              aria-label="close"
              onClick={(e) => {
                e.stopPropagation();
                useStore.getState().closeTab(tab.tableId);
              }}
            >
              ×
            </button>
          </div>
        );
      })}
    </div>
  );
}

function StatusBar() {
  const { t } = useTranslation();
  const meta = useStore((s) => (s.activeTable != null ? s.metas[s.activeTable] : undefined));
  const selected = useStore((s) => s.selectionCount);
  const progress = useStore((s) => s.progress);
  const d = meta?.dialect;
  const delim = d ? ({ ",": "Comma", "\t": "Tab", ";": "Semicolon", "|": "Pipe" } as Record<string, string>)[d.delimiter] ?? d.delimiter : null;
  return (
    <div className="statusbar">
      {d && <span>{`${d.encoding}${d.bom ? " (BOM)" : ""}`}</span>}
      {d && <span>{delim}</span>}
      {d && <span>{d.lineEnding.toUpperCase()}</span>}
      {d && !d.hasHeader && <span>{t("status.noHeader")}</span>}
      {meta && <span>{t("status.rows", { rows: meta.rowCount.toLocaleString(), cols: meta.columns.length })}</span>}
      {selected > 0 && <span>{t("status.selected", { count: selected })}</span>}
      {meta && meta.warnings.length > 0 && (
        <span className="warn" title={meta.warnings.join("\n")}>
          ⚠ {t("status.warnings", { count: meta.warnings.length })}
        </span>
      )}
      {progress && (
        <span className="progress">
          {progress.total ? `${Math.round((progress.done / progress.total) * 100)}%` : progress.done.toLocaleString()}
          <button onClick={() => void api.interrupt()}>{t("dialog.cancel")}</button>
        </span>
      )}
    </div>
  );
}

function Splitter({ onDrag, vertical }: { onDrag: (delta: number) => void; vertical: boolean }) {
  return (
    <div
      className={vertical ? "splitter-v" : "splitter-h"}
      role="separator"
      aria-orientation={vertical ? "vertical" : "horizontal"}
      onMouseDown={(e) => {
        e.preventDefault();
        let last = vertical ? e.clientX : e.clientY;
        const move = (ev: MouseEvent) => {
          const now = vertical ? ev.clientX : ev.clientY;
          onDrag(now - last);
          last = now;
        };
        const up = () => {
          window.removeEventListener("mousemove", move);
          window.removeEventListener("mouseup", up);
        };
        window.addEventListener("mousemove", move);
        window.addEventListener("mouseup", up);
      }}
    />
  );
}

function Message() {
  const message = useStore((s) => s.message);
  useEffect(() => {
    if (!message) return;
    const h = setTimeout(() => useStore.setState({ message: null }), message.kind === "error" ? 12000 : 4000);
    return () => clearTimeout(h);
  }, [message]);
  if (!message) return null;
  return (
    <div className={`message ${message.kind}`} role={message.kind === "error" ? "alert" : "status"} onClick={() => useStore.setState({ message: null })}>
      {message.text}
    </div>
  );
}

export function App() {
  const { t } = useTranslation();
  const layout = useStore((s) => s.layout);
  const active = useStore((s) => s.activeTable);
  const theme = useStore((s) => s.settings.theme);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [dragging, setDragging] = useState(false);

  // Core events.
  useEffect(() => {
    const un = listen<CoreEvent>("core-event", (e) => {
      const ev = e.payload;
      if (ev.type === "tableChanged") {
        const s = useStore.getState();
        const meta = s.metas[ev.tableId];
        if (meta && (meta.version !== ev.version || meta.rowCount !== ev.rowCount || meta.dirty !== ev.dirty)) {
          void refreshMeta(ev.tableId).then(() => useStore.getState().bump(ev.tableId));
        }
        void refreshSources();
      } else if (ev.type === "workspaceChanged") {
        void refreshSources();
      } else if (ev.type === "progress") {
        useStore.setState({ progress: ev.done >= (ev.total ?? Infinity) ? null : { task: ev.task, done: ev.done, total: ev.total } });
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  // Drag and drop from Explorer.
  useEffect(() => {
    const un = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "over" || e.payload.type === "enter") setDragging(true);
      else if (e.payload.type === "leave") setDragging(false);
      else if (e.payload.type === "drop") {
        setDragging(false);
        if (e.payload.paths.length) void addFiles(e.payload.paths);
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  // Unsaved changes on exit.
  useEffect(() => {
    const win = getCurrentWindow();
    const un = win.onCloseRequested(async (event) => {
      const dirty = await api.dirtyTables().catch(() => []);
      if (!dirty.length) return;
      event.preventDefault();
      const sources = useStore.getState().sources;
      const list = dirty.map((d) => {
        const f = findTable(sources, d.id);
        return f ? (f.table.sheet ? `${f.source.fileName} / ${f.table.sheet}` : f.source.fileName) : d.sqlName;
      });
      const r = await ask({
        title: t("dialog.exitTitle"),
        body: t("dialog.exitBody"),
        list,
        buttons: [
          { id: "cancel", label: t("dialog.cancel") },
          { id: "discard", label: t("dialog.dontSave") },
          { id: "save", label: t("dialog.saveAll"), primary: true },
        ],
      });
      if (r.button === "cancel") return;
      if (r.button === "save" && !(await saveAll())) return;
      await win.destroy();
    });
    return () => {
      void un.then((f) => f());
    };
  }, [t]);

  // Check external changes when the window gets the focus.
  useEffect(() => {
    const onFocus = () => void api.checkExternalChanges().catch(() => {});
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  // Keyboard shortcuts (03. UI, section 3.3).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      const inEditor = !!target?.closest(".cm-editor") || target?.tagName === "INPUT" || target?.tagName === "TEXTAREA" || target?.tagName === "SELECT";
      const mod = e.ctrlKey || e.metaKey;
      if (!mod) return;
      const key = e.key.toLowerCase();
      if (key === "s") {
        e.preventDefault();
        if (e.shiftKey) void saveTableAs();
        else void saveTable();
      } else if (key === "o") {
        e.preventDefault();
        void addFiles();
      } else if (key === "f") {
        e.preventDefault();
        window.dispatchEvent(new Event("armarium:find"));
      } else if (!inEditor && key === "z" && !e.shiftKey) {
        e.preventDefault();
        void undo();
      } else if (!inEditor && (key === "y" || (key === "z" && e.shiftKey))) {
        e.preventDefault();
        void redo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Font size: Ctrl + mouse wheel.
  useEffect(() => {
    let size = 13;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      size = Math.min(24, Math.max(10, size + (e.deltaY < 0 ? 1 : -1)));
      document.documentElement.style.setProperty("--font-size", `${size}px`);
    };
    window.addEventListener("wheel", onWheel, { passive: false });
    return () => window.removeEventListener("wheel", onWheel);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  const activeName = (() => {
    const f = active != null ? findTable(useStore.getState().sources, active) : undefined;
    return f ? (f.table.sheet ? `${f.source.fileName.replace(/\.[^.]+$/, "")}_${f.table.sheet}` : f.source.fileName.replace(/\.[^.]+$/, "")) : "table";
  })();

  return (
    <div className={`app ${dragging ? "dragging" : ""}`}>
      <Toolbar onSettings={() => setSettingsOpen(true)} onExport={() => setExportOpen(true)} />
      <div className="main">
        {!layout.leftCollapsed && (
          <>
            <aside className="left" style={{ width: layout.leftWidth }}>
              <SourceTree />
            </aside>
            <Splitter
              vertical
              onDrag={(d) => useStore.getState().setLayout({ leftWidth: Math.min(800, Math.max(140, useStore.getState().layout.leftWidth + d)) })}
            />
          </>
        )}
        <section className="right">
          <div className="tabs-row">
            <button
              className="collapse"
              aria-label="toggle left pane"
              onClick={() => useStore.getState().setLayout({ leftCollapsed: !layout.leftCollapsed })}
            >
              {layout.leftCollapsed ? "»" : "«"}
            </button>
            <TabBar />
          </div>
          <div className="grid-area">{active != null ? <TableGrid key={active} tableId={active} /> : <div className="placeholder">{t("grid.noTable")}</div>}</div>
          {layout.sqlVisible && (
            <>
              <Splitter
                vertical={false}
                onDrag={(d) =>
                  useStore.getState().setLayout({ sqlHeight: Math.min(window.innerHeight - 160, Math.max(120, useStore.getState().layout.sqlHeight - d)) })
                }
              />
              <div className="bottom" style={{ height: layout.sqlHeight }}>
                <SqlPanel />
              </div>
            </>
          )}
        </section>
      </div>
      <StatusBar />
      <Message />
      <ContextMenu />
      <DialogHost />
      {settingsOpen && <SettingsDialog onClose={() => setSettingsOpen(false)} />}
      {exportOpen && active != null && <ExportDialog target={{ table: active }} defaultName={activeName} onClose={() => setExportOpen(false)} />}
    </div>
  );
}
