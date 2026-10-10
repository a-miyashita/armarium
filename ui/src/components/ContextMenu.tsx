// A simple context menu. `showMenu` opens it at a position.
import { useEffect, useRef } from "react";
import { create } from "zustand";

export interface MenuItem {
  label?: string;
  action?: () => void;
  disabled?: boolean;
  separator?: boolean;
}

interface MenuState {
  menu: { x: number; y: number; items: MenuItem[] } | null;
}

const useMenu = create<MenuState>(() => ({ menu: null }));

export function showMenu(x: number, y: number, items: MenuItem[]) {
  useMenu.setState({ menu: { x, y, items } });
}

export function ContextMenu() {
  const menu = useMenu((s) => s.menu);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menu) return;
    const close = (e: Event) => {
      if (ref.current && e.target instanceof Node && ref.current.contains(e.target)) return;
      useMenu.setState({ menu: null });
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") useMenu.setState({ menu: null });
    };
    window.addEventListener("mousedown", close, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("blur", close);
    ref.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    return () => {
      window.removeEventListener("mousedown", close, true);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("blur", close);
    };
  }, [menu]);

  if (!menu) return null;
  const left = Math.min(menu.x, window.innerWidth - 240);
  const top = Math.min(menu.y, window.innerHeight - menu.items.length * 28 - 8);
  return (
    <div ref={ref} className="context-menu" role="menu" style={{ left, top }}>
      {menu.items.map((it, i) =>
        it.separator ? (
          <div key={i} className="menu-separator" role="separator" />
        ) : (
          <button
            key={i}
            role="menuitem"
            disabled={it.disabled}
            onClick={() => {
              useMenu.setState({ menu: null });
              it.action?.();
            }}
            onKeyDown={(e) => {
              const buttons = Array.from(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
              const idx = buttons.indexOf(e.currentTarget);
              if (e.key === "ArrowDown") buttons[(idx + 1) % buttons.length]?.focus();
              if (e.key === "ArrowUp") buttons[(idx - 1 + buttons.length) % buttons.length]?.focus();
            }}
          >
            {it.label}
          </button>
        ),
      )}
    </div>
  );
}
