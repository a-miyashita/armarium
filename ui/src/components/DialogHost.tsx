// Shows the dialogs that `ask` requested, one at a time.
import { useEffect, useRef, useState } from "react";
import { useStore } from "../store";

export function DialogHost() {
  const dialog = useStore((s) => s.dialogs[0]);
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!dialog) return;
    setValue(dialog.input?.value ?? "");
    setTimeout(() => {
      if (inputRef.current) {
        inputRef.current.focus();
        inputRef.current.select();
      } else {
        boxRef.current?.querySelector<HTMLButtonElement>("button.primary")?.focus();
      }
    }, 0);
  }, [dialog]);

  if (!dialog) return null;
  const finish = (button: string) => {
    useStore.setState((s) => ({ dialogs: s.dialogs.slice(1) }));
    dialog.resolve({ button, value });
  };
  const cancelId = dialog.buttons.find((b) => b.id === "cancel" || b.id === "no")?.id ?? dialog.buttons[0]?.id;
  const primary = dialog.buttons.find((b) => b.primary)?.id;
  return (
    <div className="modal-backdrop">
      <div
        ref={boxRef}
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-label={dialog.title}
        onKeyDown={(e) => {
          if (e.key === "Escape" && cancelId) finish(cancelId);
          if (e.key === "Enter" && dialog.input && primary) finish(primary);
        }}
      >
        <h2>{dialog.title}</h2>
        {dialog.body && <p className="pre">{dialog.body}</p>}
        {dialog.list && (
          <ul className="dialog-list">
            {dialog.list.map((l) => (
              <li key={l}>{l}</li>
            ))}
          </ul>
        )}
        {dialog.input && (
          <label className="field">
            <span>{dialog.input.label}</span>
            <input ref={inputRef} value={value} onChange={(e) => setValue(e.target.value)} />
          </label>
        )}
        <div className="buttons">
          {dialog.buttons.map((b) => (
            <button key={b.id} className={b.primary ? "primary" : ""} onClick={() => finish(b.id)}>
              {b.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
