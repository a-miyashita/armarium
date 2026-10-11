// Read settings (03. UI, section 5): override the detection and read the file again.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { refreshSources } from "../actions";
import { api } from "../api";
import type { ReadOptions } from "../bindings/ReadOptions";
import { ask, showError, useStore } from "../store";

const ENCODINGS = ["UTF-8", "Shift_JIS", "EUC-JP", "UTF-16LE", "UTF-16BE", "windows-1252"];

export function ReadSettingsDialog({ tableId, onClose }: { tableId: number; onClose: () => void }) {
  const { t } = useTranslation();
  const [opts, setOpts] = useState<ReadOptions | null>(null);
  const meta = useStore((s) => s.metas[tableId]);
  const isExcel = useStore((s) => s.sources.some((x) => x.kind === "excel" && x.tables.some((tb) => tb.id === tableId)));

  useEffect(() => {
    api.getReadOptions(tableId).then(setOpts, showError);
  }, [tableId]);

  if (!opts) return null;
  const set = (patch: Partial<ReadOptions>) => setOpts({ ...opts, ...patch });

  const apply = async () => {
    if (meta?.dirty) {
      const r = await ask({
        title: t("readSettings.title"),
        body: t("dialog.confirmReload"),
        buttons: [
          { id: "cancel", label: t("dialog.cancel") },
          { id: "ok", label: t("dialog.ok"), primary: true },
        ],
      });
      if (r.button !== "ok") return;
    }
    try {
      const m = await api.setReadOptions(tableId, opts);
      useStore.getState().setMeta(m);
      useStore.getState().updateTab(tableId, { viewId: null, viewSpec: null, viewTotal: null });
      useStore.getState().bump(tableId);
      onClose();
    } catch (e) {
      showError(e);
    }
    await refreshSources();
  };

  const delimiterValue = opts.delimiter == null ? "" : opts.delimiter === "\t" ? "\\t" : opts.delimiter;
  const quoteValue = opts.quote == null ? "" : opts.quote === "\0" ? "none" : opts.quote;
  return (
    <div className="modal-backdrop">
      <div className="modal" role="dialog" aria-modal="true" aria-label={t("readSettings.title")} onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <h2>{t("readSettings.title")}</h2>
        {!isExcel && (
          <>
            <label className="field">
              <span>{t("readSettings.encoding")}</span>
              <select value={opts.encoding ?? ""} onChange={(e) => set({ encoding: e.target.value || null })}>
                <option value="">{t("readSettings.auto")}</option>
                {ENCODINGS.map((x) => (
                  <option key={x} value={x}>
                    {x}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span>{t("readSettings.delimiter")}</span>
              <select
                value={delimiterValue}
                onChange={(e) => set({ delimiter: e.target.value === "" ? null : e.target.value === "\\t" ? "\t" : e.target.value })}
              >
                <option value="">{t("readSettings.auto")}</option>
                <option value=",">,</option>
                <option value="\t">{t("readSettings.tab")}</option>
                <option value=";">;</option>
                <option value="|">|</option>
              </select>
            </label>
            <label className="field">
              <span>{t("readSettings.quote")}</span>
              <select value={quoteValue} onChange={(e) => set({ quote: e.target.value === "" ? null : e.target.value === "none" ? "\0" : e.target.value })}>
                <option value="">{t("readSettings.auto")}</option>
                <option value={'"'}>"</option>
                <option value="'">'</option>
                <option value="none">{t("readSettings.none")}</option>
              </select>
            </label>
          </>
        )}
        <label className="field">
          <span>{t("readSettings.header")}</span>
          <select
            value={opts.hasHeader == null ? "" : opts.hasHeader ? "yes" : "no"}
            onChange={(e) => set({ hasHeader: e.target.value === "" ? null : e.target.value === "yes" })}
          >
            <option value="">{t("readSettings.auto")}</option>
            <option value="yes">{t("readSettings.yes")}</option>
            <option value="no">{t("readSettings.no")}</option>
          </select>
        </label>
        <label className="field">
          <span>{t("readSettings.skipRows")}</span>
          <input type="number" min={0} value={opts.skipRows ?? 0} onChange={(e) => set({ skipRows: Math.max(0, Number(e.target.value) || 0) })} />
        </label>
        <label className="check">
          <input type="checkbox" checked={opts.typeInference ?? true} onChange={(e) => set({ typeInference: e.target.checked })} />
          {t("readSettings.typeInference")}
        </label>
        <div className="buttons">
          <button onClick={onClose}>{t("dialog.cancel")}</button>
          <button className="primary" onClick={() => void apply()}>
            {t("readSettings.apply")}
          </button>
        </div>
      </div>
    </div>
  );
}
