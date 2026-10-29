import { invoke } from "@tauri-apps/api/core";
import type { Filter } from "./api";

/**
 * Filtre de l'explorateur : celui de src/api.ts plus les champs ajoutés par le
 * back-end (lot C). Les durées sont en secondes, comptées depuis maintenant.
 */
export type ExplorerFilter = Filter & {
  /** Travaillé dans Live dans cette durée (604800 → les 7 derniers jours). */
  worked_since?: number;
  /** Pas travaillé depuis cette durée (jamais travaillé compris). */
  not_worked_since?: number;
  /** Modifié sur le disque dans cette durée. */
  modified_since?: number;
  /** Nom exact d'un plugin utilisé. */
  plugin?: string;
};

/** Collection intelligente : un filtre enregistré sous un nom. */
export interface Collection {
  id: string;
  name: string;
  filter: ExplorerFilter;
}

export const explorerApi = {
  listCollections: () => invoke<Collection[]>("list_collections"),
  saveCollections: (collections: Collection[]) => invoke<void>("save_collections", { collections }),
};
export const { listCollections, saveCollections } = explorerApi;

/** Champs vides retirés : deux filtres équivalents ont la même forme. */
export function cleanFilter(f: ExplorerFilter): ExplorerFilter {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(f)) {
    if (v === undefined || v === null || v === "" || v === false) continue;
    out[k] = typeof v === "string" ? v.trim() : v;
    if (out[k] === "") delete out[k];
  }
  return out as ExplorerFilter;
}

export function sameFilter(a: ExplorerFilter, b: ExplorerFilter): boolean {
  const x = cleanFilter(a);
  const y = cleanFilter(b);
  const kx = Object.keys(x);
  return (
    kx.length === Object.keys(y).length &&
    kx.every((k) => (x as Record<string, unknown>)[k] === (y as Record<string, unknown>)[k])
  );
}

export function newId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  }
}

/** « 7 jours », « 3 mois », « 1 an ». */
export function formatSpan(seconds: number): string {
  const days = Math.round(seconds / 86_400);
  if (days >= 365 && days % 365 === 0) return days === 365 ? "1 an" : `${days / 365} ans`;
  if (days >= 30 && days % 30 === 0) return `${days / 30} mois`;
  if (days === 1) return "24 h";
  return `${days} jours`;
}

/** Détail de l'évènement `am:toast` (affiché par src/components/Toasts.tsx). */
export interface ToastDetail {
  message: string;
  /** Bouton « Annuler » : défait une action déjà faite. */
  undo?: () => void | Promise<void>;
  /** Action destructrice : exécutée seulement à l'expiration, « Annuler » l'abandonne. */
  commit?: () => Promise<void>;
  duration?: number;
}

export function toast(detail: ToastDetail | string) {
  window.dispatchEvent(
    new CustomEvent<ToastDetail>("am:toast", { detail: typeof detail === "string" ? { message: detail } : detail }),
  );
}
