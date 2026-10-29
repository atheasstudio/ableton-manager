import { useEffect, useRef, useState } from "react";
import "../ui.css";

/**
 * Notification (contrat entre lots, évènement DOM « am:toast ») :
 * - sans commit : simple message, 4 s ; si undo, « Annuler » l'appelle ;
 * - avec commit (action destructrice) : rien n'est fait tout de suite, commit()
 *   est appelé à l'expiration (6 s) ; « Annuler » abandonne. Si undo est aussi
 *   donné, il est appelé à l'abandon (pour réafficher ce qu'on avait masqué).
 */
export interface ToastDetail {
  message: string;
  undo?: () => void | Promise<void>;
  commit?: () => Promise<void>;
  /** Millisecondes. */
  duration?: number;
}

/** Affiche une notification (raccourci pour l'évènement am:toast). */
export function toast(detail: ToastDetail | string) {
  window.dispatchEvent(
    new CustomEvent<ToastDetail>("am:toast", {
      detail: typeof detail === "string" ? { message: detail } : detail,
    }),
  );
}

/** Message d'erreur d'une promesse rejetée. */
export function toastError(prefix: string) {
  return (e: unknown) => toast(`${prefix} : ${e}`);
}

interface Item extends ToastDetail {
  id: number;
}

let nextId = 1;
/** Actions destructrices en attente, exécutées si la fenêtre se ferme avant l'expiration. */
const pending = new Map<number, () => Promise<void>>();

function ToastItem({ item, onDone }: { item: Item; onDone: (id: number) => void }) {
  const duration = item.duration ?? (item.commit ? 6000 : 4000);
  const [left, setLeft] = useState(duration);
  const [paused, setPaused] = useState(false);
  const settled = useRef(false);

  function finish(cancelled: boolean) {
    if (settled.current) return;
    settled.current = true;
    pending.delete(item.id);
    onDone(item.id);
    if (cancelled) {
      Promise.resolve(item.undo?.()).catch((e) => toast(`Annulation impossible : ${e}`));
    } else if (item.commit) {
      item.commit().catch((e) => toast(`Erreur : ${e}`));
    }
  }

  useEffect(() => {
    if (item.commit) pending.set(item.id, item.commit);
  }, []);

  // Le survol suspend le décompte.
  useEffect(() => {
    if (paused) return;
    const start = Date.now();
    const timer = setTimeout(() => finish(false), left);
    return () => {
      clearTimeout(timer);
      setLeft((l) => Math.max(0, l - (Date.now() - start)));
    };
  }, [paused]);

  const error = /^(Erreur|.* impossible)/.test(item.message);
  return (
    <div
      className={`toast ${error ? "error" : ""} ${item.commit ? "pending" : ""}`}
      role="status"
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
    >
      <span className="toast-msg">{item.message}</span>
      {(item.undo || item.commit) && (
        <button className="link" onClick={() => finish(true)}>
          Annuler
        </button>
      )}
      <button className="icon" onClick={() => finish(false)} aria-label="Fermer">
        ×
      </button>
      <span
        className="toast-bar"
        style={{
          animationDuration: `${duration}ms`,
          animationPlayState: paused ? "paused" : "running",
        }}
      />
    </div>
  );
}

/** Pile de notifications en bas de la fenêtre. */
export default function Toasts() {
  const [items, setItems] = useState<Item[]>([]);

  useEffect(() => {
    const onToast = (e: Event) => {
      const d = (e as CustomEvent<ToastDetail>).detail;
      if (!d?.message) return;
      // Au-delà de 5, les plus anciennes simples partent (les actions en attente restent).
      setItems((all) => {
        const next = [...all, { ...d, id: nextId++ }];
        while (next.length > 5) {
          const i = next.findIndex((t) => !t.commit);
          if (i < 0) break;
          next.splice(i, 1);
        }
        return next;
      });
    };
    // Fenêtre fermée : les suppressions demandées partent quand même.
    const flush = () => {
      for (const commit of pending.values()) commit().catch(() => {});
      pending.clear();
    };
    window.addEventListener("am:toast", onToast);
    window.addEventListener("beforeunload", flush);
    return () => {
      window.removeEventListener("am:toast", onToast);
      window.removeEventListener("beforeunload", flush);
    };
  }, []);

  const done = (id: number) => setItems((all) => all.filter((t) => t.id !== id));

  return (
    <div className="toasts" aria-live="polite">
      {items.map((t) => (
        <ToastItem key={t.id} item={t} onDone={done} />
      ))}
    </div>
  );
}
