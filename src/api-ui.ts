import { invoke } from "@tauri-apps/api/core";
import { Filter } from "./api";

/**
 * Commandes du back-end utilisées par l'interface (lot B) et pas encore dans
 * api.ts. Voir le contrat entre lots.
 */

/** Filtre étendu : durées en secondes comptées depuis maintenant, plugin exact. */
export type CollectionFilter = Filter & {
  worked_since?: number;
  not_worked_since?: number;
  modified_since?: number;
  plugin?: string;
};

export interface Collection {
  id: string;
  name: string;
  filter: CollectionFilter;
}

export const apiUi = {
  listCollections: () => invoke<Collection[]>("list_collections"),
};
