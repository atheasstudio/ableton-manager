import { useEffect, useRef } from "react";

/** Raccourcis affichés dans l'aide (touche ?). */
export const SHORTCUTS: [string, string][] = [
  ["Espace", "Écouter / arrêter l'aperçu du projet sélectionné"],
  ["↑ ↓", "Projet précédent / suivant (l'écoute passe au suivant, au même endroit)"],
  ["/", "Rechercher dans la bibliothèque"],
  ["F", "Ajouter ou retirer des favoris"],
  ["Ctrl + K", "Palette : projets et actions"],
  ["Ctrl + O", "Ouvrir le projet dans Live"],
  ["?", "Afficher les raccourcis"],
  ["Échap", "Fermer la palette ou l'aide"],
];

export interface ShortcutHandlers {
  togglePlay: () => void;
  palette: () => void;
  openInLive: () => void;
  help: () => void;
}

/** Vrai quand la frappe va dans un champ : les raccourcis d'une touche ne s'appliquent pas. */
export function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el || !el.tagName) return false;
  if (el.isContentEditable) return true;
  if (el.tagName === "TEXTAREA" || el.tagName === "SELECT") return true;
  if (el.tagName === "INPUT") {
    const type = (el as HTMLInputElement).type;
    return !["checkbox", "radio", "button", "submit", "range", "color", "file"].includes(type);
  }
  return false;
}

/** Une fenêtre (réglages, palette, docteur…) est ouverte par-dessus. */
function modalOpen(): boolean {
  return document.querySelector(".overlay") !== null;
}

export function emit(name: string, detail?: unknown) {
  window.dispatchEvent(new CustomEvent(name, { detail }));
}

/**
 * Raccourcis globaux de la fenêtre. Ils sont ignorés quand on tape dans un
 * champ ou qu'une fenêtre est ouverte par-dessus (elle gère ses propres touches).
 * ↑ ↓ restent à l'explorateur.
 */
export function useShortcuts(handlers: ShortcutHandlers) {
  const ref = useRef(handlers);
  ref.current = handlers;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || (e.repeat && e.key !== " ")) return;
      if (modalOpen() || isTyping(e.target)) return;
      const h = ref.current;
      const ctrl = e.ctrlKey || e.metaKey;

      if (ctrl && !e.altKey && !e.shiftKey) {
        const k = e.key.toLowerCase();
        if (k === "k") {
          e.preventDefault();
          h.palette();
        } else if (k === "o") {
          e.preventDefault();
          h.openInLive();
        }
        return;
      }
      if (ctrl || e.altKey) return;

      switch (e.key) {
        case " ":
          e.preventDefault();
          if (!e.repeat) h.togglePlay();
          break;
        case "/":
          e.preventDefault();
          emit("am:focus-search");
          break;
        case "f":
        case "F":
          e.preventDefault();
          emit("am:toggle-favorite");
          break;
        case "?":
          e.preventDefault();
          h.help();
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
