// The data grid of a table tab (03. UI, section 3.2).
import { useCallback, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { applyEdit, changeColumn, insertColumn, refreshView, renameColumn, setView } from "../actions";
import { api } from "../api";
import type { LogicalType } from "../bindings/LogicalType";
import type { ViewSpec } from "../bindings/ViewSpec";
import { useStore } from "../store";
import { showMenu, type MenuItem } from "./ContextMenu";
import { FilterDialog } from "./FilterDialog";
import { PagedGrid, toPage, type GridColumnDef } from "./PagedGrid";

const TYPES: LogicalType[] = ["text", "integer", "real", "date", "datetime", "boolean"];

export function TableGrid({ tableId }: { tableId: number }) {
  const { t } = useTranslation();
  const meta = useStore((s) => s.metas[tableId]);
  const tab = useStore((s) => s.tabs.find((x) => x.tableId === tableId));
  const version = useStore((s) => s.versions[tableId] ?? 0);
  const [filterFor, setFilterFor] = useState<string | null | undefined>(undefined);
  const selectedRows = useRef<() => Promise<number[]>>(async () => []);

  const columns = useMemo<GridColumnDef[]>(
    () => (meta?.columns ?? []).map((c) => ({ id: c.sqlName, title: c.displayName || c.sqlName, type: c.logicalType })),
    [meta?.columns],
  );
  const viewId = tab?.viewId ?? null;
  const total = viewId != null ? (tab?.viewTotal ?? 0) : (meta?.rowCount ?? 0);

  const fetch = useCallback(
    async (offset: number, limit: number) => toPage(await api.fetchRows(tableId, viewId, offset, limit)),
    [tableId, viewId],
  );

  const afterRowChange = useCallback(() => refreshView(tableId), [tableId]);

  const bindSelection = useCallback((get: () => Promise<number[]>) => {
    selectedRows.current = get;
  }, []);

  if (!meta) return <div className="placeholder">{t("grid.loading")}</div>;

  const spec: ViewSpec = tab?.viewSpec ?? { sort: [], filters: [] };

  const headerMenu = (col: number, x: number, y: number) => {
    const c = meta.columns[col];
    if (!c) return;
    const items: MenuItem[] = [
      { label: t("grid.sortAsc"), action: () => void setView(tableId, { ...spec, sort: [{ column: c.sqlName, direction: "asc" }] }) },
      { label: t("grid.sortDesc"), action: () => void setView(tableId, { ...spec, sort: [{ column: c.sqlName, direction: "desc" }] }) },
      { label: t("grid.filter"), action: () => setFilterFor(c.sqlName) },
      { label: t("grid.clearSort"), action: () => void setView(tableId, null), disabled: viewId == null },
      { separator: true },
      ...TYPES.filter((ty) => ty !== c.logicalType).map((ty) => ({
        label: `${t("grid.changeType")}: ${t(`types.${ty}`)}`,
        action: () => void changeColumn(tableId, { op: "changeColumnType", column: c.sqlName, logicalType: ty }),
      })),
      { separator: true },
      { label: t("grid.insertLeft"), action: () => void insertColumn(tableId, col) },
      { label: t("grid.insertRight"), action: () => void insertColumn(tableId, col + 1) },
      { label: t("grid.renameColumn"), action: () => void renameColumn(tableId, c.sqlName, c.displayName || c.sqlName) },
      {
        label: t("grid.deleteColumn"),
        action: () => void changeColumn(tableId, { op: "deleteColumn", column: c.sqlName }),
        disabled: meta.columns.length <= 1,
      },
    ];
    showMenu(x, y, items);
  };

  const insertRows = async (rids: number[], above: boolean) => {
    if (!rids.length) return;
    const op = above
      ? { op: "insertRows" as const, afterRid: rids[0], count: rids.length, above: true }
      : { op: "insertRows" as const, afterRid: rids[rids.length - 1], count: rids.length };
    if (await applyEdit(tableId, op)) await afterRowChange();
  };

  const cellMenu = (rids: number[], x: number, y: number) => {
    showMenu(x, y, [
      { label: t("grid.insertAbove"), action: () => void insertRows(rids, true) },
      { label: t("grid.insertBelow"), action: () => void insertRows(rids, false) },
      {
        label: t("grid.deleteRows"),
        action: async () => {
          if (await applyEdit(tableId, { op: "deleteRows", rids })) await afterRowChange();
        },
      },
      { separator: true },
      { label: t("grid.filter"), action: () => setFilterFor(null) },
      { label: t("grid.clearSort"), action: () => void setView(tableId, null), disabled: viewId == null },
    ]);
  };

  return (
    <div
      className="table-grid"
      onKeyDown={async (e) => {
        if (!e.ctrlKey) return;
        if (e.key === "-") {
          e.preventDefault();
          const rids = await selectedRows.current();
          if (rids.length && (await applyEdit(tableId, { op: "deleteRows", rids }))) await afterRowChange();
        } else if (e.key === "+" || e.key === "=" || e.key === ";") {
          e.preventDefault();
          const rids = await selectedRows.current();
          await insertRows(rids, true);
        }
      }}
    >
      {viewId != null && (
        <div className="view-bar">
          <span>{t("grid.filtered", { count: total })}</span>
          <span className="view-desc">
            {[
              ...spec.sort.map((s) => `${s.column} ${s.direction === "asc" ? "↑" : "↓"}`),
              ...spec.filters.map((f) => `${f.column ?? "*"} ${t(`filter.${f.op}`)} ${f.value ?? ""}`),
            ].join(" · ")}
          </span>
          <button onClick={() => void setView(tableId, null)}>{t("grid.clearFilter")}</button>
        </div>
      )}
      <PagedGrid
        sourceKey={`t${tableId}:${viewId ?? "file"}`}
        version={version}
        total={total}
        columns={columns}
        fetch={fetch}
        editable
        bindSelection={bindSelection}
        onSelectionCount={(n) => useStore.setState({ selectionCount: n })}
        onEdit={async (cell, rid, value) => {
          const c = meta.columns[cell.col];
          if (!c) return;
          await applyEdit(tableId, { op: "setCells", cells: [{ rid, column: c.sqlName, value }] });
          useStore.getState().bump(tableId);
        }}
        onPaste={async (anchor, rows, rids) => {
          const c = meta.columns[anchor.col];
          if (!c || !rids.length) return;
          const tsv = rows.map((r) => r.map((v) => (/[\t\r\n"]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v)).join("\t")).join("\r\n");
          const before = meta.rowCount;
          if (await applyEdit(tableId, { op: "paste", anchorRid: rids[0], anchorColumn: c.sqlName, tsv, rids })) {
            const after = useStore.getState().metas[tableId]?.rowCount ?? before;
            if (after !== before) await afterRowChange();
            else useStore.getState().bump(tableId);
          }
        }}
        onClear={async (cells) => {
          const edits = cells
            .map((x) => ({ rid: x.rid, column: meta.columns[x.col]?.sqlName, value: null }))
            .filter((x): x is { rid: number; column: string; value: null } => !!x.column);
          if (edits.length && (await applyEdit(tableId, { op: "setCells", cells: edits }))) useStore.getState().bump(tableId);
        }}
        onHeaderMenu={headerMenu}
        onCellMenu={(_cell, rids, x, y) => cellMenu(rids, x, y)}
      />
      {filterFor !== undefined && (
        <FilterDialog
          columns={meta.columns}
          initialColumn={filterFor}
          onClose={() => setFilterFor(undefined)}
          onApply={(f) => {
            setFilterFor(undefined);
            void setView(tableId, { ...spec, filters: [...spec.filters, f] });
          }}
        />
      )}
    </div>
  );
}
