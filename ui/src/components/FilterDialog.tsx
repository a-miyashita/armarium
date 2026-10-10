// A filter condition for the grid view.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { ColumnSchema } from "../bindings/ColumnSchema";
import type { FilterCond } from "../bindings/FilterCond";
import type { FilterOp } from "../bindings/FilterOp";

const OPS: FilterOp[] = ["contains", "equals", "notEquals", "greater", "less", "isEmpty", "isNotEmpty"];

export function FilterDialog(props: {
  columns: ColumnSchema[];
  initialColumn: string | null;
  onClose: () => void;
  onApply: (f: FilterCond) => void;
}) {
  const { t } = useTranslation();
  const [column, setColumn] = useState<string>(props.initialColumn ?? "");
  const [op, setOp] = useState<FilterOp>("contains");
  const [value, setValue] = useState("");
  const needsValue = op !== "isEmpty" && op !== "isNotEmpty";
  const apply = () => props.onApply({ column: column || null, op: column ? op : "contains", value: needsValue ? value : null });
  return (
    <div className="modal-backdrop">
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-label={t("filter.title")}
        onKeyDown={(e) => {
          if (e.key === "Escape") props.onClose();
          if (e.key === "Enter") apply();
        }}
      >
        <h2>{t("filter.title")}</h2>
        <label className="field">
          <span>{t("filter.column")}</span>
          <select value={column} onChange={(e) => setColumn(e.target.value)}>
            <option value="">{t("filter.anyColumn")}</option>
            {props.columns.map((c) => (
              <option key={c.sqlName} value={c.sqlName}>
                {c.displayName || c.sqlName}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          <span>{t("filter.op")}</span>
          <select value={column ? op : "contains"} disabled={!column} onChange={(e) => setOp(e.target.value as FilterOp)}>
            {OPS.map((o) => (
              <option key={o} value={o}>
                {t(`filter.${o}`)}
              </option>
            ))}
          </select>
        </label>
        {needsValue && (
          <label className="field">
            <span>{t("filter.value")}</span>
            <input autoFocus value={value} onChange={(e) => setValue(e.target.value)} />
          </label>
        )}
        <div className="buttons">
          <button onClick={props.onClose}>{t("dialog.cancel")}</button>
          <button className="primary" onClick={apply}>
            {t("filter.apply")}
          </button>
        </div>
      </div>
    </div>
  );
}
