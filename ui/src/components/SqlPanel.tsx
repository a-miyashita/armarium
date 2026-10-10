// The SQL editor and the results (03. UI, section 4).
import { autocompletion } from "@codemirror/autocomplete";
import { sql, SQLite } from "@codemirror/lang-sql";
import { Compartment, EditorState, Prec } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { basicSetup } from "codemirror";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { runSql, saveResultAsSource } from "../actions";
import { api } from "../api";
import type { StatementResult } from "../bindings/StatementResult";
import { useStore } from "../store";
import { ExportDialog } from "./ExportDialog";
import { PagedGrid, toPage, type GridColumnDef } from "./PagedGrid";

const languageConf = new Compartment();

async function loadSchema() {
  try {
    const tables = await api.completion();
    const schema: Record<string, string[]> = {};
    for (const t of tables) schema[t.sqlName] = t.columns ?? [];
    return schema;
  } catch {
    return {};
  }
}

export function SqlPanel() {
  const { t } = useTranslation();
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const running = useStore((s) => s.running);
  const results = useStore((s) => s.results);
  const sqlError = useStore((s) => s.sqlError);
  const activeResult = useStore((s) => s.activeResult);
  const history = useStore((s) => s.history);
  const sources = useStore((s) => s.sources);
  const [exporting, setExporting] = useState<number | null>(null);

  const run = useCallback(() => {
    const v = view.current;
    if (!v) return;
    const sel = v.state.selection.main;
    const text = sel.empty ? v.state.doc.toString() : v.state.sliceDoc(sel.from, sel.to);
    void runSql(text);
  }, []);

  // Create the editor one time.
  useEffect(() => {
    if (!host.current) return;
    const state = EditorState.create({
      doc: useStore.getState().sqlText,
      extensions: [
        basicSetup,
        languageConf.of(sql({ dialect: SQLite, upperCaseKeywords: true })),
        autocompletion({ activateOnTyping: true }),
        Prec.highest(
          keymap.of([
            {
              key: "Mod-Enter",
              run: () => {
                run();
                return true;
              },
            },
          ]),
        ),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) useStore.setState({ sqlText: u.state.doc.toString() });
        }),
        EditorView.lineWrapping,
      ],
    });
    const v = new EditorView({ state, parent: host.current });
    view.current = v;
    const onInsert = (e: Event) => {
      const text = (e as CustomEvent<string>).detail;
      const sel = v.state.selection.main;
      v.dispatch({ changes: { from: sel.from, to: sel.to, insert: text }, selection: { anchor: sel.from + text.length } });
      v.focus();
    };
    window.addEventListener("armarium:insert-sql", onInsert);
    return () => {
      window.removeEventListener("armarium:insert-sql", onInsert);
      v.destroy();
      view.current = null;
    };
  }, [run]);

  // Completion: table names and column names of loaded tables.
  useEffect(() => {
    let cancelled = false;
    void loadSchema().then((schema) => {
      if (cancelled || !view.current) return;
      view.current.dispatch({
        effects: languageConf.reconfigure(sql({ dialect: SQLite, upperCaseKeywords: true, schema })),
      });
    });
    return () => {
      cancelled = true;
    };
  }, [sources]);

  // Show the position of an error.
  useEffect(() => {
    const v = view.current;
    if (!v || !sqlError) return;
    const pos = Math.min(sqlError.offset, v.state.doc.length);
    v.dispatch({ selection: { anchor: pos }, scrollIntoView: true });
  }, [sqlError]);

  const active = results.find((r) => r.resultId === activeResult);

  return (
    <div className="sql-panel">
      <div className="sql-toolbar">
        <button className="primary" onClick={run} disabled={running} title="Ctrl+Enter">
          ▶ {t("sql.run")}
        </button>
        {running && <button onClick={() => void api.interrupt()}>{t("sql.cancel")}</button>}
        <select
          aria-label={t("sql.history")}
          value=""
          onChange={(e) => {
            const v = view.current;
            const text = e.target.value;
            if (!v || !text) return;
            v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: text } });
            v.focus();
          }}
        >
          <option value="">{t("sql.history")}…</option>
          {history.map((h, i) => (
            <option key={i} value={h}>
              {h.length > 80 ? `${h.slice(0, 80)}…` : h}
            </option>
          ))}
        </select>
      </div>
      <div className="sql-body">
        <div className="sql-editor" ref={host} />
        <div className="sql-results">
          <ResultList results={results} />
          {sqlError && (
            <div className="sql-error" role="alert">
              {t("sql.errorAt", { index: sqlError.statementIndex + 1, message: `${sqlError.code}: ${sqlError.message}` })}
            </div>
          )}
          {active && active.resultId != null ? (
            <>
              <div className="result-actions">
                <button onClick={() => setExporting(active.resultId!)}>{t("toolbar.export")}…</button>
                <button onClick={() => void saveResultAsSource(active.resultId!)}>{t("sql.saveAsSource")}</button>
              </div>
              <ResultGrid result={active} />
            </>
          ) : (
            !sqlError && results.length === 0 && <div className="placeholder">{t("sql.noResult")}</div>
          )}
        </div>
      </div>
      {exporting != null && (
        <ExportDialog target={{ result: exporting }} defaultName="result" onClose={() => setExporting(null)} />
      )}
    </div>
  );
}

function ResultList({ results }: { results: StatementResult[] }) {
  const { t } = useTranslation();
  const activeResult = useStore((s) => s.activeResult);
  if (!results.length) return null;
  return (
    <div className="result-list" role="tablist">
      {results.map((r, i) => {
        const label =
          r.kind === "rows"
            ? `#${i + 1} ${t("sql.rows", { count: r.rowCount })}`
            : `#${i + 1} ${t("sql.changed", { count: r.changedRows, table: r.sqlName })}`;
        const selectable = r.resultId != null;
        return (
          <button
            key={i}
            role="tab"
            aria-selected={selectable && r.resultId === activeResult}
            className={`result-chip ${selectable && r.resultId === activeResult ? "active" : ""}`}
            disabled={!selectable}
            title={`${r.sql}\n${t("sql.elapsed", { ms: r.elapsedMs })}`}
            onClick={() => selectable && useStore.setState({ activeResult: r.resultId })}
          >
            {label}
          </button>
        );
      })}
    </div>
  );
}

function ResultGrid({ result }: { result: StatementResult }) {
  const id = result.resultId!;
  const columns = useMemo<GridColumnDef[]>(
    () => result.columns.map((c, i) => ({ id: `c${i}`, title: c.name, type: c.logicalType })),
    [result.columns],
  );
  const fetch = useCallback(async (offset: number, limit: number) => toPage(await api.fetchResult(id, offset, limit)), [id]);
  return (
    <PagedGrid
      sourceKey={`r${id}`}
      version={0}
      total={result.rowCount}
      columns={columns}
      fetch={fetch}
      editable={false}
      onSelectionCount={(n) => useStore.setState({ selectionCount: n })}
    />
  );
}
