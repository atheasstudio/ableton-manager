import { useEffect } from "react";

export interface MenuItem {
  label: string;
  action: () => void;
  danger?: boolean;
  disabled?: boolean;
}

export interface MenuState {
  x: number;
  y: number;
  items: MenuItem[];
}

/** Menu du clic droit, fermé au clic suivant ou par Échap. */
export default function ContextMenu({ menu, onClose }: { menu: MenuState; onClose: () => void }) {
  useEffect(() => {
    const close = () => onClose();
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("click", close);
    window.addEventListener("blur", close);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", key);
    };
  }, [onClose]);
  const x = Math.min(menu.x, window.innerWidth - 230);
  const y = Math.min(menu.y, window.innerHeight - menu.items.length * 30 - 10);
  return (
    <div className="ctxmenu" style={{ left: x, top: y }} onContextMenu={(e) => e.preventDefault()}>
      {menu.items.map((it) => (
        <button
          key={it.label}
          className={it.danger ? "danger" : ""}
          disabled={it.disabled}
          onClick={() => {
            onClose();
            it.action();
          }}
        >
          {it.label}
        </button>
      ))}
    </div>
  );
}
