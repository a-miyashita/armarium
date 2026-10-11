// The left pane: registered sources in a tree (03. UI, section 2).
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  addFiles,
  insertIntoSql,
  openTable,
  quoteName,
  relocateSource,
  reloadTable,
  renameTable,
  unregisterSource,
} from "../actions";
import { api } from "../api";
import type { SourceInfo } from "../bindings/SourceInfo";
import type { TableInfo } from "../bindings/TableInfo";
import { showError, useStore } from "../store";
import { showMenu, type MenuItem } from "./ContextMenu";
import { ReadSettingsDialog } from "./ReadSettingsDialog";

function formatSize(n: number | null): string {
  if (n == null) return "-";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

function statusIcon(src: SourceInfo, t: TableInfo): string {
  if (!src.exists || t.load.state === "error") return "⚠";
  if (t.externallyModified) return "↻";
  if (t.load.state === "loaded" && t.load.dirty) return "●";
  return "";
}

interface Row {
  key: string;
  depth: number;
  label: string;
  icon: string;
  status: string;
  source: SourceInfo;
  table?: TableInfo;
  expandable?: boolean;
  expanded?: boolean;
  dim?: boolean;
}

export function SourceTree() {
  const { t } = useTranslation();
  const sources = useStore((s) => s.sources);
  const active = useStore((s) => s.activeTable);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [focused, setFocused] = useState<string | null>(null);
  const [settingsFor, setSettingsFor] = useState<number | null>(null);

  const rows = useMemo(() => {
    const out: Row[] = [];
    const group = (key: string, label: string, list: SourceInfo[]) => {
      if (!list.length) return;
      const open = !collapsed[key];
      out.push({ key, depth: 0, label, icon: open ? "▼" : "▶", status: "", source: list[0], expandable: true, expanded: open });
      if (!open) return;
      for (const s of list) {
        if (s.kind === "csv") {
          const tb = s.tables[0];
          if (!tb) continue;
          out.push({ key: `t${tb.id}`, depth: 1, label: s.fileName, icon: "📄", status: statusIcon(s, tb), source: s, table: tb });
        } else {
          const k = `s${s.id}`;
          const sopen = !collapsed[k];
          const status = s.exists ? (s.tables.some((x) => x.load.state === "loaded" && x.load.dirty) ? "●" : "") : "⚠";
          out.push({ key: k, depth: 1, label: s.fileName, icon: sopen ? "📗" : "📗", status, source: s, expandable: true, expanded: sopen });
          if (!sopen) continue;
          for (const tb of s.tables) {
            out.push({
              key: `t${tb.id}`,
              depth: 2,
              label: tb.sheet ?? tb.sqlName,
              icon: "▦",
              status: statusIcon(s, tb),
              source: s,
              table: tb,
              dim: tb.hidden,
            });
          }
        }
      }
    };
    group("g-csv", t("tree.csv"), sources.filter((s) => s.kind === "csv"));
    group("g-excel", t("tree.excel"), sources.filter((s) => s.kind === "excel"));
    return out;
  }, [sources, collapsed, t]);

  const toggle = (key: string) => setCollapsed((c) => ({ ...c, [key]: !c[key] }));

  const menuFor = (r: Row): MenuItem[] => {
    const items: MenuItem[] = [];
    const tb = r.table;
    if (tb) {
      items.push(
        { label: t("tree.rename"), action: () => void renameTable(tb.id) },
        { label: t("tree.insertName"), action: () => insertIntoSql(quoteName(tb.sqlName)) },
        { label: t("tree.selectTemplate"), action: () => insertIntoSql(`SELECT * FROM ${quoteName(tb.sqlName)} LIMIT 100;`) },
        { separator: true },
        { label: t("tree.reload"), action: () => void reloadTable(tb.id), disabled: tb.load.state !== "loaded" },
        {
          label: t("tree.discard"),
          action: () => void reloadTable(tb.id),
          disabled: !(tb.load.state === "loaded" && tb.load.dirty),
        },
        { label: t("tree.readSettings"), action: () => setSettingsFor(tb.id) },
        { separator: true },
      );
    }
    if (!r.expandable || r.depth === 1) {
      items.push(
        { label: t("tree.showInExplorer"), action: () => void api.showInExplorer(r.source.path).catch(showError) },
        { label: t("tree.relocate"), action: () => void relocateSource(r.source.id) },
        { separator: true },
        { label: t("tree.unregister"), action: () => void unregisterSource(r.source.id) },
      );
    }
    return items;
  };

  const activate = (r: Row) => {
    if (r.table) void openTable(r.table.id);
    else if (r.expandable) toggle(r.key);
  };

  const onKey = (e: React.KeyboardEvent) => {
    const i = rows.findIndex((r) => r.key === focused);
    const r = rows[i];
    if (e.key === "ArrowDown") {
      setFocused(rows[Math.min(rows.length - 1, i + 1)]?.key ?? null);
      e.preventDefault();
    } else if (e.key === "ArrowUp") {
      setFocused(rows[Math.max(0, i - 1)]?.key ?? null);
      e.preventDefault();
    } else if (!r) {
      return;
    } else if (e.key === "Enter") {
      activate(r);
    } else if (e.key === "ArrowRight" && r.expandable && !r.expanded) {
      toggle(r.key);
    } else if (e.key === "ArrowLeft" && r.expandable && r.expanded) {
      toggle(r.key);
    } else if (e.key === "Delete" && (r.table || r.depth === 1)) {
      void unregisterSource(r.source.id);
    } else if (e.key === "F2" && r.table) {
      void renameTable(r.table.id);
    } else if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
      const el = document.getElementById(`tree-${r.key}`);
      const rect = el?.getBoundingClientRect();
      showMenu(rect?.left ?? 0, rect?.bottom ?? 0, menuFor(r));
      e.preventDefault();
    }
  };

  return (
    <div className="source-tree" role="tree" tabIndex={0} onKeyDown={onKey} aria-activedescendant={focused ? `tree-${focused}` : undefined}>
      {rows.length === 0 && (
        <div className="tree-empty">
          <p>{t("tree.empty")}</p>
          <button onClick={() => void addFiles()}>{t("toolbar.add")}</button>
        </div>
      )}
      {rows.map((r) => {
        const tip = r.table
          ? t("tree.tooltip", {
              path: r.source.path,
              sqlName: r.table.sqlName,
              size: formatSize(r.source.size),
              modified: r.source.modifiedMs ? new Date(r.source.modifiedMs).toLocaleString() : "-",
            }) +
            (r.source.exists ? "" : `\n${t("tree.missing")}`) +
            (r.table.externallyModified ? `\n${t("tree.changedOutside")}` : "") +
            (r.table.load.state === "error" ? `\n${r.table.load.message}` : "") +
            (r.table.hidden ? `\n${t("tree.hidden")}` : "")
          : r.depth === 1
            ? r.source.path
            : undefined;
        return (
          <div
            id={`tree-${r.key}`}
            key={r.key}
            role="treeitem"
            aria-expanded={r.expandable ? r.expanded : undefined}
            aria-selected={r.table?.id === active}
            className={[
              "tree-row",
              r.table?.id === active ? "active" : "",
              focused === r.key ? "focused" : "",
              r.dim ? "dim" : "",
              r.depth === 0 ? "group" : "",
            ].join(" ")}
            style={{ paddingLeft: 8 + r.depth * 16 }}
            title={tip}
            onClick={() => {
              setFocused(r.key);
              if (r.expandable) toggle(r.key);
              else activate(r);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              setFocused(r.key);
              if (r.depth > 0) showMenu(e.clientX, e.clientY, menuFor(r));
            }}
          >
            <span className="tree-icon">{r.icon}</span>
            <span className="tree-label">{r.label}</span>
            {r.status && <span className={`tree-status s-${r.status === "●" ? "dirty" : r.status === "↻" ? "changed" : "warn"}`}>{r.status}</span>}
          </div>
        );
      })}
      {settingsFor != null && <ReadSettingsDialog tableId={settingsFor} onClose={() => setSettingsFor(null)} />}
    </div>
  );
}
