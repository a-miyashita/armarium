// Export (03. UI, section 5): format, encoding, line ending, BOM, and header row.
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { exportTo } from "../actions";
import type { ExportFormat } from "../bindings/ExportFormat";
import type { LineEnding } from "../bindings/LineEnding";

type Kind = "csv" | "tsv" | "xlsx";

export function ExportDialog(props: { target: { table: number } | { result: number }; defaultName: string; onClose: () => void }) {
  const { t } = useTranslation();
  const [kind, setKind] = useState<Kind>("csv");
  const [encoding, setEncoding] = useState("UTF-8");
  const [lineEnding, setLineEnding] = useState<LineEnding>("crlf");
  const [bom, setBom] = useState(true);
  const [header, setHeader] = useState(true);
  const [quoteAll, setQuoteAll] = useState(false);
  const [sheetName, setSheetName] = useState("");

  const run = async () => {
    const path = await saveDialog({
      defaultPath: `${props.defaultName}.${kind}`,
      filters: [{ name: kind.toUpperCase(), extensions: [kind] }],
    });
    if (!path) return;
    const format: ExportFormat =
      kind === "xlsx"
        ? { format: "xlsx", sheetName: sheetName.trim() || null }
        : {
            format: "csv",
            encoding,
            bom: bom && encoding.startsWith("UTF"),
            delimiter: kind === "tsv" ? "\t" : ",",
            lineEnding,
            quoteAll,
            header,
          };
    props.onClose();
    await exportTo(props.target, format, path);
  };

  return (
    <div className="modal-backdrop">
      <div className="modal" role="dialog" aria-modal="true" aria-label={t("export.title")} onKeyDown={(e) => e.key === "Escape" && props.onClose()}>
        <h2>{t("export.title")}</h2>
        <label className="field">
          <span>{t("export.format")}</span>
          <select value={kind} onChange={(e) => setKind(e.target.value as Kind)}>
            <option value="csv">CSV</option>
            <option value="tsv">TSV</option>
            <option value="xlsx">XLSX</option>
          </select>
        </label>
        {kind === "xlsx" ? (
          <label className="field">
            <span>{t("export.sheetName")}</span>
            <input value={sheetName} onChange={(e) => setSheetName(e.target.value)} placeholder="Sheet1" />
          </label>
        ) : (
          <>
            <label className="field">
              <span>{t("export.encoding")}</span>
              <select value={encoding} onChange={(e) => setEncoding(e.target.value)}>
                {["UTF-8", "Shift_JIS", "EUC-JP", "UTF-16LE", "UTF-16BE"].map((x) => (
                  <option key={x}>{x}</option>
                ))}
              </select>
            </label>
            <label className="field">
              <span>{t("export.lineEnding")}</span>
              <select value={lineEnding} onChange={(e) => setLineEnding(e.target.value as LineEnding)}>
                <option value="crlf">CRLF</option>
                <option value="lf">LF</option>
                <option value="cr">CR</option>
              </select>
            </label>
            <label className="check">
              <input type="checkbox" checked={bom} disabled={!encoding.startsWith("UTF")} onChange={(e) => setBom(e.target.checked)} />
              {t("export.bom")}
            </label>
            <label className="check">
              <input type="checkbox" checked={header} onChange={(e) => setHeader(e.target.checked)} />
              {t("export.header")}
            </label>
            <label className="check">
              <input type="checkbox" checked={quoteAll} onChange={(e) => setQuoteAll(e.target.checked)} />
              {t("export.quoteAll")}
            </label>
          </>
        )}
        <div className="buttons">
          <button onClick={props.onClose}>{t("dialog.cancel")}</button>
          <button className="primary" onClick={() => void run()}>
            {t("export.choose")}
          </button>
        </div>
      </div>
    </div>
  );
}
