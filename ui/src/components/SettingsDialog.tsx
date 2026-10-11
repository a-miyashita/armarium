// Settings (03. UI, section 5): language, theme, NULL display, and MCP information.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../api";
import i18n from "../i18n";
import { useStore, type Settings } from "../store";

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const settings = useStore((s) => s.settings);
  const setSettings = useStore((s) => s.setSettings);
  const [mcp, setMcp] = useState<{ command: string | null; workspace: string } | null>(null);

  useEffect(() => {
    api.mcpInfo().then(setMcp, () => setMcp(null));
  }, []);

  const config = mcp?.command
    ? JSON.stringify({ mcpServers: { armarium: { command: mcp.command, args: [] } } }, null, 2)
    : null;

  return (
    <div className="modal-backdrop">
      <div className="modal wide" role="dialog" aria-modal="true" aria-label={t("settings.title")} onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <h2>{t("settings.title")}</h2>
        <label className="field">
          <span>{t("settings.language")}</span>
          <select
            value={settings.language}
            onChange={(e) => {
              setSettings({ language: e.target.value });
              void i18n.changeLanguage(e.target.value);
            }}
          >
            <option value="ja">日本語</option>
            <option value="en">English</option>
          </select>
        </label>
        <label className="field">
          <span>{t("settings.theme")}</span>
          <select value={settings.theme} onChange={(e) => setSettings({ theme: e.target.value as Settings["theme"] })}>
            <option value="system">{t("settings.system")}</option>
            <option value="light">{t("settings.light")}</option>
            <option value="dark">{t("settings.dark")}</option>
          </select>
        </label>
        <label className="field">
          <span>{t("settings.nullDisplay")}</span>
          <select value={settings.nullDisplay} onChange={(e) => setSettings({ nullDisplay: e.target.value as Settings["nullDisplay"] })}>
            <option value="empty">{t("settings.nullEmpty")}</option>
            <option value="null">{t("settings.nullText")}</option>
          </select>
        </label>
        <h3>{t("settings.mcp")}</h3>
        {config ? (
          <>
            <p>{t("settings.mcpHelp")}</p>
            <pre className="code" tabIndex={0}>
              {config}
            </pre>
          </>
        ) : (
          <p>{t("settings.mcpMissing")}</p>
        )}
        <div className="buttons">
          <button className="primary" onClick={onClose}>
            {t("dialog.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
