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
  /** Aperçu audio (fichier du coffre). */
  preview_hash: string | null;
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

export interface Branch {
  id: number;
  project_id: number;
  name: string;
  folder: string;
  als_path: string;
  /** Version d'où part la branche. */
  base_id: number | null;
  /** Dernière version de la branche. */
  head_id: number | null;
  is_main: boolean;
  created_at: number;
}

export type VersionSource = "save" | "backup" | "manual" | "restore";

export interface Version {
  id: number;
  branch_id: number;
  parent_id: number | null;
  created_at: number;
  source: VersionSource;
  name: string | null;
  /** Résumé des changements par rapport à la version parente. */
  auto_name: string;
  als_name: string;
  tempo: number | null;
  live_version: string | null;
  files: number;
  missing: number;
  /** Aperçu audio (fichier du coffre). */
  preview_hash: string | null;
}

export interface VersionsView {
  branches: Branch[];
  versions: Version[];
}

type Change<T> = [T, T] | null;

export interface Diff {
  tempo: Change<number | null>;
  time_sig: Change<string | null>;
  live_version: Change<string | null>;
  tracks_added: { name: string; kind: string }[];
  tracks_removed: { name: string; kind: string }[];
  plugins_added: string[];
  plugins_removed: string[];
  samples_added: string[];
  samples_removed: string[];
  locators_added: string[];
  locators_removed: string[];
}

export interface RestoreReport {
  version_id: number | null;
  lost: string[];
}

export interface NewBranch {
  branch: Branch;
  lost: string[];
}

export interface VaultInfo {
  path: string;
  files: number;
  bytes: number;
}

export type PreviewSource = "capture" | "render" | "file";

export const PREVIEW_SOURCES: Record<PreviewSource, string> = {
  capture: "Écoutes dans Live",
  render: "Rendu complet",
  file: "Fichier importé",
};

export interface PreviewInfo {
  hash: string;
  /** Secondes. */
  duration: number;
  /** Forme d'onde : niveau crête de chaque tranche, de 0 à 255. */
  peaks: number[];
  source: PreviewSource;
  /** Tempo qui aligne l'aperçu sur les mesures (seconde = beats × 60 / tempo). */
  tempo: number | null;
  created_at: number;
}

export interface RenderState {
  project_id: number;
  /** Position atteinte et fin du morceau, en beats. */
  beats: number;
  end: number;
}

export interface CaptureStatus {
  listening: boolean;
  /** Le device Max for Live a donné signe de vie récemment. */
  device: boolean;
  recording: boolean;
  /** Set ouvert dans Live. */
  open_set: string | null;
  render: RenderState | null;
}

export interface RenderDone {
  project_id: number | null;
  error: string | null;
}

export interface Annotation {
  id: number;
  /** Position en beats (noires) depuis la mesure 1. */
  beat: number;
  label: string;
  created_at: number;
}

export interface Timeline {
  tempo: number | null;
  /** [numérateur, dénominateur] */
  time_sig: [number, number];
  /** Fin du dernier clip de l'arrangement, en beats. */
  length: number | null;
  /** Locators du set, tels que sauvegardés. */
  locators: { time: number; name: string }[];
  annotations: Annotation[];
}

export interface LiveStatus {
  /** Version du Remote Script, null s'il ne répond pas. */
  script: number | null;
  /** Version que l'appli installe. */
  expected_script: number;
  /** Set ouvert dans Live, d'après le titre de sa fenêtre. */
  open_set: string | null;
  /** Le set ouvert est-il ce projet ? null si on ne peut pas le savoir. */
  is_open: boolean | null;
  transport: { playing: boolean; beat: number; tempo: number } | null;
}

/** Note posée dans l'espace de l'arbre des versions. */
export interface TreeNote {
  id: number;
  /** Version à laquelle la note est accrochée (elle la suit). */
  version_id: number | null;
  /** Décalage par rapport à la version, ou position si la note est libre. */
  x: number;
  y: number;
  text: string;
  created_at: number;
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

  getVersions: (projectId: number) => invoke<VersionsView>("get_versions", { projectId }),
  enableVersions: (projectId: number, importBackups: boolean) =>
    invoke<number>("enable_versions", { projectId, importBackups }),
  snapshotNow: (branchId: number) => invoke<number | null>("snapshot_now", { branchId }),
  renameVersion: (id: number, name: string) => invoke<void>("rename_version", { id, name }),
  restoreVersion: (id: number) => invoke<RestoreReport>("restore_version", { id }),
  createBranch: (versionId: number, name: string) =>
    invoke<NewBranch>("create_branch", { versionId, name }),
  deleteVersion: (id: number) => invoke<number>("delete_version", { id }),
  compareVersions: (a: number, b: number) => invoke<Diff>("compare_versions", { a, b }),
  openBranch: (branchId: number) => invoke<void>("open_branch", { branchId }),
  openBranchFolder: (branchId: number) => invoke<void>("open_branch_folder", { branchId }),
  vaultInfo: () => invoke<VaultInfo>("vault_info"),
  setVaultDir: (path: string) => invoke<void>("set_vault_dir", { path }),

  getPreview: (hash: string) => invoke<PreviewInfo | null>("get_preview", { hash }),
  previewAudio: (hash: string) => invoke<ArrayBuffer>("preview_audio", { hash }),
  findBounce: (projectId: number) => invoke<string | null>("find_bounce", { projectId }),
  importPreview: (projectId: number, path: string) =>
    invoke<string>("import_preview", { projectId, path }),
  captureStatus: () => invoke<CaptureStatus>("capture_status"),
  renderPreview: (projectId: number) => invoke<void>("render_preview", { projectId }),
  cancelRender: () => invoke<void>("cancel_render"),
  /** Sans dossier, cherche la bibliothèque utilisateur de Live dans Documents. */
  installDevice: (userLibrary?: string) =>
    invoke<string>("install_device", { userLibrary: userLibrary ?? null }),
  getTimeline: (projectId: number) => invoke<Timeline>("get_timeline", { projectId }),
  addAnnotation: (projectId: number, beat: number, label: string) =>
    invoke<Annotation>("add_annotation", { projectId, beat, label }),
  updateAnnotation: (id: number, beat: number, label: string) =>
    invoke<void>("update_annotation", { id, beat, label }),
  deleteAnnotation: (id: number) => invoke<void>("delete_annotation", { id }),
  importLocators: (projectId: number) => invoke<number>("import_locators", { projectId }),
  liveStatus: (projectId: number) => invoke<LiveStatus>("live_status", { projectId }),
  /** Renvoie vrai si le set a dû être ouvert dans Live. */
  liveJump: (projectId: number, beat: number, play: boolean) =>
    invoke<boolean>("live_jump", { projectId, beat, play }),
  /** Renvoie le dossier d'installation, ou null si la User Library n'est pas trouvée. */
  installRemoteScript: (userLibrary: string | null) =>
    invoke<string | null>("install_remote_script", { userLibrary }),
  listTreeNotes: (projectId: number) => invoke<TreeNote[]>("list_tree_notes", { projectId }),
  addTreeNote: (projectId: number, versionId: number | null, x: number, y: number, text = "") =>
    invoke<TreeNote>("add_tree_note", { projectId, versionId, x, y, text }),
  updateTreeNote: (n: TreeNote) =>
    invoke<void>("update_tree_note", {
      id: n.id,
      versionId: n.version_id,
      x: n.x,
      y: n.y,
      text: n.text,
    }),
  deleteTreeNote: (id: number) => invoke<void>("delete_tree_note", { id }),
};

/** Erreur d'installation : la bibliothèque utilisateur de Live n'est pas à sa place habituelle. */
export const NO_USER_LIBRARY = "NO_USER_LIBRARY";

export function formatDateTime(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString("fr-FR", {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatSize(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} Ko`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} Mo`;
  return `${(bytes / 1024 ** 3).toFixed(2)} Go`;
}

/** Nom affiché d'une version : celui donné par l'utilisateur, sinon le résumé. */
export function versionLabel(v: Version): string {
  return v.name ?? v.auto_name;
}

export function formatDate(seconds: number): string {
  return new Date(seconds * 1000).toLocaleDateString("fr-FR", {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

/** mm:ss */
export function formatTime(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Numéro de mesure (à partir de 1) d'une position en secondes. */
export function barAt(seconds: number, tempo: number, timeSig: string | null): number {
  const [num, den] = (timeSig ?? "4/4").split("/").map(Number);
  // Live compte en noires : une mesure de 6/8 fait 3 beats.
  const beatsPerBar = num && den ? (num * 4) / den : 4;
  return Math.floor((seconds * tempo) / 60 / beatsPerBar) + 1;
}
