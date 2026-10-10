// A virtual grid that fetches only the visible pages ([ADR-0014]).
// It uses Glide Data Grid (ADR-0005, amendment 2026-10-11).
import {
  CompactSelection,
  DataEditor,
  GridCellKind,
  GridColumnIcon,
  type DataEditorRef,
  type EditableGridCell,
  type GridCell,
  type GridColumn,
  type GridSelection,
  type Item,
  type Rectangle,
  type Theme,
} from "@glideapps/glide-data-grid";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { LogicalType } from "../bindings/LogicalType";
import { PageCache, type Page } from "../lib/pageCache";
import { useStore } from "../store";

export interface GridColumnDef {
  id: string;
  title: string;
  type: LogicalType;
}

export interface CellRef {
  /** Display row index. */
  row: number;
  /** Data column index (without the row number column). */
  col: number;
}

export interface PagedGridProps {
  /** Changes when the data source changes (another table or view). */
  sourceKey: string;
  /** Changes when the data changes. */
  version: number;
  total: number;
  columns: GridColumnDef[];
  fetch: (offset: number, limit: number) => Promise<Page>;
  editable: boolean;
  onEdit?: (cell: CellRef, rid: number, value: string | null) => void;
  onPaste?: (anchor: CellRef, rows: string[][], rids: number[]) => void;
  onClear?: (cells: { rid: number; col: number }[]) => void;
  onHeaderMenu?: (col: number, x: number, y: number) => void;
  onCellMenu?: (cell: CellRef, rids: number[], x: number, y: number) => void;
  onSelectionCount?: (n: number) => void;
  /** Gives the component a function that returns the rids of the selected rows. */
  bindSelection?: (get: () => Promise<number[]>) => void;
}

const ICONS: Record<LogicalType, string> = {
  integer: GridColumnIcon.HeaderNumber,
  real: GridColumnIcon.HeaderNumber,
  text: GridColumnIcon.HeaderString,
  date: GridColumnIcon.HeaderDate,
  datetime: GridColumnIcon.HeaderTime,
  boolean: GridColumnIcon.HeaderBoolean,
};

function useDark(): boolean {
  const theme = useStore((s) => s.settings.theme);
  const [systemDark, setSystemDark] = useState(() => window.matchMedia("(prefers-color-scheme: dark)").matches);
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const on = () => setSystemDark(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return theme === "dark" || (theme === "system" && systemDark);
}

const darkTheme: Partial<Theme> = {
  accentColor: "#5aa9e6",
  accentLight: "rgba(90, 169, 230, 0.2)",
  textDark: "#e6e6e6",
  textMedium: "#b0b0b0",
  textLight: "#8a8a8a",
  textBubble: "#ffffff",
  bgIconHeader: "#b0b0b0",
  fgIconHeader: "#1e1e1e",
  textHeader: "#d0d0d0",
  textHeaderSelected: "#000000",
  bgCell: "#1e1e1e",
  bgCellMedium: "#262626",
  bgHeader: "#2a2a2a",
  bgHeaderHasFocus: "#333333",
  bgHeaderHovered: "#383838",
  bgBubble: "#2f2f2f",
  bgBubbleSelected: "#3a3a3a",
  bgSearchResult: "#5c4b00",
  borderColor: "rgba(255, 255, 255, 0.12)",
  drilldownBorder: "rgba(255, 255, 255, 0.2)",
  linkColor: "#6cb6ff",
};

export function PagedGrid(props: PagedGridProps) {
  const { sourceKey, version, total, columns, fetch, editable } = props;
  const ref = useRef<DataEditorRef>(null);
  const mouse = useRef({ x: 0, y: 0 });
  const nullDisplay = useStore((s) => s.settings.nullDisplay);
  const dark = useDark();
  const [widths, setWidths] = useState<Record<string, number>>({});
  const [selection, setSelection] = useState<GridSelection>({
    columns: CompactSelection.empty(),
    rows: CompactSelection.empty(),
  });
  const visible = useRef<Rectangle>({ x: 0, y: 0, width: 10, height: 50 });
  const fetchRef = useRef(fetch);
  fetchRef.current = fetch;

  const cache = useMemo(
    () =>
      new PageCache(
        (offset, limit) => fetchRef.current(offset, limit),
        (p) => {
          const size = cache.pageSizeValue;
          const cells: { cell: Item }[] = [];
          const r = visible.current;
          const from = Math.max(p * size, r.y);
          const to = Math.min((p + 1) * size, r.y + r.height + 1);
          for (let row = from; row < to; row++) {
            for (let c = 0; c <= columns.length; c++) cells.push({ cell: [c, row] });
          }
          if (cells.length) ref.current?.updateCells(cells);
        },
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [sourceKey, columns.length],
  );

  // A new version: discard the pages and draw again.
  useEffect(() => {
    cache.clear();
    const r = visible.current;
    cache.prefetch(r.y, r.y + r.height, total);
  }, [cache, version, total]);

  const gridColumns = useMemo<GridColumn[]>(
    () => [
      { id: "__rownum", title: "#", width: widths["__rownum"] ?? 64 },
      ...columns.map((c) => ({
        id: c.id,
        title: c.title,
        icon: ICONS[c.type],
        width: widths[c.id] ?? Math.min(320, Math.max(80, c.title.length * 9 + 48)),
        hasMenu: !!props.onHeaderMenu,
      })),
    ],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [columns, widths, !!props.onHeaderMenu],
  );

  const getCellContent = useCallback(
    ([col, row]: Item): GridCell => {
      const hit = cache.row(row);
      if (!hit) return { kind: GridCellKind.Loading, allowOverlay: false };
      const { page, index } = hit;
      if (col === 0) {
        const n = String(page.rowNumbers[index] ?? row + 1);
        return {
          kind: GridCellKind.Text,
          data: n,
          displayData: n,
          allowOverlay: false,
          readonly: true,
          contentAlign: "right",
          themeOverride: { bgCell: dark ? "#2a2a2a" : "#f3f3f3", textDark: dark ? "#9a9a9a" : "#777777" },
        };
      }
      const def = columns[col - 1];
      const v = page.data[col - 1]?.[index] ?? null;
      const note = page.notes.get(`${index}:${col - 1}`);
      const numeric = def?.type === "integer" || def?.type === "real";
      if (note?.kind === "error") {
        return {
          kind: GridCellKind.Text,
          data: "",
          displayData: note.text,
          allowOverlay: false,
          readonly: true,
          themeOverride: { textDark: "#d33" },
        };
      }
      const display = v ?? (nullDisplay === "null" ? "NULL" : "");
      return {
        kind: GridCellKind.Text,
        data: v ?? "",
        displayData: display,
        allowOverlay: true,
        readonly: !editable || note?.kind === "formula",
        contentAlign: numeric ? "right" : "left",
        copyData: v ?? "",
        themeOverride:
          v == null && nullDisplay === "null"
            ? { textDark: dark ? "#777" : "#aaa" }
            : note?.kind === "formula"
              ? { bgCell: dark ? "#23303a" : "#eef6fb" }
              : undefined,
      };
    },
    [cache, columns, nullDisplay, editable, dark],
  );

  const ridsFor = useCallback(
    async (fromRow: number, count: number): Promise<number[]> => {
      const out: number[] = [];
      for (let r = fromRow; r < Math.min(total, fromRow + count); r++) {
        const hit = await cache.ensure(r);
        if (hit) out.push(hit.page.rids[hit.index]);
      }
      return out;
    },
    [cache, total],
  );

  const selectedRows = useCallback(async (): Promise<number[]> => {
    const rows = new Set<number>();
    for (const r of selection.rows) rows.add(r);
    const cur = selection.current;
    if (cur) {
      for (const rect of [cur.range, ...cur.rangeStack]) {
        for (let r = rect.y; r < rect.y + rect.height; r++) rows.add(r);
      }
    }
    const sorted = [...rows].sort((a, b) => a - b);
    const out: number[] = [];
    for (const r of sorted) out.push(...(await ridsFor(r, 1)));
    return out;
  }, [selection, ridsFor]);

  useEffect(() => props.bindSelection?.(selectedRows), [props.bindSelection, selectedRows]);

  const onCellEdited = useCallback(
    ([col, row]: Item, value: EditableGridCell) => {
      if (col === 0 || value.kind !== GridCellKind.Text) return;
      const hit = cache.row(row);
      if (!hit) return;
      const text = value.data;
      props.onEdit?.({ row, col: col - 1 }, hit.page.rids[hit.index], text === "" ? null : text);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [cache, props.onEdit],
  );

  return (
    <div
      className="grid-host"
      onContextMenuCapture={(e) => {
        mouse.current = { x: e.clientX, y: e.clientY };
      }}
      onMouseDownCapture={(e) => {
        mouse.current = { x: e.clientX, y: e.clientY };
      }}
    >
      <DataEditor
        ref={ref}
        columns={gridColumns}
        rows={total}
        getCellContent={getCellContent}
        freezeColumns={1}
        rowMarkers="none"
        smoothScrollX
        smoothScrollY
        width="100%"
        height="100%"
        theme={dark ? darkTheme : undefined}
        gridSelection={selection}
        onGridSelectionChange={(sel) => {
          setSelection(sel);
          let n = 0;
          if (sel.current) {
            for (const r of [sel.current.range, ...sel.current.rangeStack]) n += r.width * r.height;
          }
          n += sel.rows.length * columns.length;
          props.onSelectionCount?.(n);
        }}
        onVisibleRegionChanged={(r) => {
          visible.current = r;
          cache.prefetch(r.y, r.y + r.height, total);
        }}
        onColumnResize={(col, w) => setWidths((s) => ({ ...s, [String(col.id)]: w }))}
        onCellEdited={editable ? onCellEdited : undefined}
        getCellsForSelection={(rect) => async () => {
          const out: GridCell[][] = [];
          for (let r = rect.y; r < rect.y + rect.height; r++) {
            await cache.ensure(r);
            const row: GridCell[] = [];
            for (let c = rect.x; c < rect.x + rect.width; c++) row.push(getCellContent([c, r]));
            out.push(row);
          }
          return out;
        }}
        onPaste={
          editable
            ? (target: Item, values: readonly (readonly string[])[]) => {
                const [col, row] = target;
                if (col === 0) return false;
                void ridsFor(row, values.length).then((rids) =>
                  props.onPaste?.({ row, col: col - 1 }, values.map((r) => [...r]), rids),
                );
                return false;
              }
            : false
        }
        onDelete={
          editable
            ? (sel) => {
                const cur = sel.current;
                if (!cur) return false;
                const rect = cur.range;
                void ridsFor(rect.y, rect.height).then((rids) => {
                  const cells: { rid: number; col: number }[] = [];
                  for (const rid of rids) {
                    for (let c = Math.max(1, rect.x); c < rect.x + rect.width; c++) cells.push({ rid, col: c - 1 });
                  }
                  props.onClear?.(cells);
                });
                return false;
              }
            : undefined
        }
        onHeaderContextMenu={(col, e) => {
          e.preventDefault();
          if (col > 0) props.onHeaderMenu?.(col - 1, mouse.current.x, mouse.current.y);
        }}
        onHeaderMenuClick={(col, bounds) => {
          if (col > 0) props.onHeaderMenu?.(col - 1, bounds.x, bounds.y + bounds.height);
        }}
        onCellContextMenu={([col, row], e) => {
          e.preventDefault();
          const inSel =
            selection.current &&
            row >= selection.current.range.y &&
            row < selection.current.range.y + selection.current.range.height;
          if (!inSel) {
            setSelection({
              columns: CompactSelection.empty(),
              rows: CompactSelection.empty(),
              current: { cell: [col, row], range: { x: col, y: row, width: 1, height: 1 }, rangeStack: [] },
            });
          }
          const rows = inSel ? selectedRows() : ridsFor(row, 1);
          void rows.then((rids) => props.onCellMenu?.({ row, col: col - 1 }, rids, mouse.current.x, mouse.current.y));
        }}
      />
    </div>
  );
}

/** Converts a page from the API to the cache form. */
export function toPage(p: {
  rids: number[];
  rowNumbers: number[];
  data: (string | null)[][];
  notes: { row: number; col: number; kind: string; text: string }[];
}): Page {
  const notes = new Map<string, { kind: string; text: string }>();
  for (const n of p.notes) notes.set(`${n.row}:${n.col}`, { kind: n.kind, text: n.text });
  return { rids: p.rids, rowNumbers: p.rowNumbers, data: p.data, notes };
}
