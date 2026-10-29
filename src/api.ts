import { invoke } from "@tauri-apps/api/core";

export type Status = "idee" | "en_cours" | "fini" | "abandonne";

export const STATUS_LABELS: Record<Status, string> = {
  idee: "Idée",
  en_cours: "En cours",
  fini: "Fini",
  abandonne: "Abandonné",
};

export interface Root {
  id: number;
  path: string;
}

export interface ProjectSummary {
  id: number;
  name: string;
  folder: string;
  tempo: number | null;
  live_version: string | null;
  modified_at: number;
  status: Status;
  favorite: boolean;
  tags: string[];
}

export interface Item {
  name: string;
  path: string | null;
  missing: boolean;
}

export interface ProjectDetail extends ProjectSummary {
  als_path: string;
  time_sig: string | null;
  note: string;
  tracks: { name: string; kind: "audio" | "midi" | "group" | "return" }[];
  plugins: Item[];
  samples: Item[];
  other_sets: Item[];
}

export interface Filter {
  query?: string;
  status?: Status;
  tag?: string;
  favorites_only?: boolean;
}

export interface ScanReport {
  found: number;
  updated: number;
  removed: number;
  errors: [string, string][];
}

export interface ScanProgress {
  root: string;
  done: number;
  total: number;
  current: string;
}

export const api = {
  listRoots: () => invoke<Root[]>("list_roots"),
  addRoot: (path: string) => invoke<number>("add_root", { path }),
  removeRoot: (id: number) => invoke<void>("remove_root", { id }),
  scan: () => invoke<ScanReport>("scan"),
  searchProjects: (filter: Filter) => invoke<ProjectSummary[]>("search_projects", { filter }),
  getProject: (id: number) => invoke<ProjectDetail | null>("get_project", { id }),
  allTags: () => invoke<string[]>("all_tags"),
  setStatus: (id: number, status: Status) => invoke<void>("set_status", { id, status }),
  setFavorite: (id: number, favorite: boolean) => invoke<void>("set_favorite", { id, favorite }),
  setNote: (id: number, note: string) => invoke<void>("set_note", { id, note }),
  addTag: (id: number, tag: string) => invoke<void>("add_tag", { id, tag }),
  removeTag: (id: number, tag: string) => invoke<void>("remove_tag", { id, tag }),
  openInAbleton: (id: number) => invoke<void>("open_in_ableton", { id }),
  openFolder: (id: number) => invoke<void>("open_folder", { id }),
};

export function formatDate(seconds: number): string {
  return new Date(seconds * 1000).toLocaleDateString("fr-FR", {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}
