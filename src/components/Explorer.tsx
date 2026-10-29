import { useEffect, useMemo, useRef, useState } from "react";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { api, Filter, formatAgo, formatDate, formatDuration, ProjectSummary, STATUS_LABELS, Status } from "../api";
import ContextMenu, { MenuItem, MenuState } from "./ContextMenu";

type Sort = "recent" | "worked" | "session" | "name" | "tempo" | "folder" | "status";
const SORTS: Record<Sort, string> = {
  recent: "Plus récents",
  worked: "Plus travaillés",
  session: "Dernière session",
  name: "Nom A → Z",
  tempo: "BPM",
  folder: "Dossier A → Z",
  status: "Statut",
};
const STATUS_ORDER: Status[] = ["en_cours", "idee", "fini", "abandonne"];
const collator = new Intl.Collator("fr", { numeric: true, sensitivity: "base" });

function loadSort(): Sort {
  try {
    const s = localStorage.getItem("explorer-sort") as Sort | null;
    if (s && s in SORTS) return s;
  } catch {}
  return "recent";
}

function parentDir(path: string): string {
  const i = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return i > 0 ? path.slice(0, i) : path;
}

interface Props {
  projects: ProjectSummary[];
  tags: string[];
  filter: Filter;
  onFilter: (f: Filter) => void;
  selected: number | null;
  onSelect: (id: number) => void;
  /** Projet en cours d'écoute. */
  playing: number | null;
  onPlay: (p: ProjectSummary) => void;
  /** Après un changement sur le disque (renommage, suppression…). */
  onChanged: () => void;
  onMessage: (m: string) => void;
}

/** Dernier élément d'un chemin Windows ou Unix. */
export function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/**
 * Explorateur de projets : recherche, filtres et liste. Les sets qui partagent
 * un dossier sont regroupés sous ce dossier. ↑ ↓ passent d'un projet à l'autre.
 */
export default function Explorer(props: Props) {
  const { projects, filter, selected } = props;
  const list = useRef<HTMLDivElement>(null);
  const [sort, setSortState] = useState<Sort>(loadSort);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const setSort = (s: Sort) => {
    setSortState(s);
    try {
      localStorage.setItem("explorer-sort", s);
    } catch {}
  };

  const sorted = useMemo(() => {
    const cmp: Record<Sort, (a: ProjectSummary, b: ProjectSummary) => number> = {
      recent: (a, b) => b.modified_at - a.modified_at,
      // Temps passé dans Live ; à égalité (souvent 0), les plus récents d'abord.
      worked: (a, b) => b.time_spent - a.time_spent || b.modified_at - a.modified_at,
      // Jamais travaillés en dernier.
      session: (a, b) => (b.last_worked_at ?? 0) - (a.last_worked_at ?? 0) || b.modified_at - a.modified_at,
      name: (a, b) => collator.compare(a.name, b.name),
      tempo: (a, b) => (a.tempo ?? 1e9) - (b.tempo ?? 1e9) || collator.compare(a.name, b.name),
      folder: (a, b) => collator.compare(a.folder, b.folder) || collator.compare(a.name, b.name),
      status: (a, b) =>
        STATUS_ORDER.indexOf(a.status) - STATUS_ORDER.indexOf(b.status) || b.modified_at - a.modified_at,
    };
    return [...projects].sort(cmp[sort]);
  }, [projects, sort]);

  // Groupes dans l'ordre du tri choisi.
  const groups = useMemo(() => {
    const byFolder = new Map<string, ProjectSummary[]>();
    for (const p of sorted) {
      const g = byFolder.get(p.folder);
      if (g) g.push(p);
      else byFolder.set(p.folder, [p]);
    }
    return [...byFolder.entries()];
  }, [sorted]);
  const order = useMemo(() => groups.flatMap(([, ps]) => ps.map((p) => p.id)), [groups]);

  // Garde le projet choisi visible.
  useEffect(() => {
    list.current?.querySelector(`[data-id="${selected}"]`)?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    if ((e.target as HTMLElement).closest("select, textarea")) return;
    e.preventDefault();
    const i = selected === null ? -1 : order.indexOf(selected);
    const next = e.key === "ArrowDown" ? Math.min(order.length - 1, i + 1) : Math.max(0, i - 1);
    if (order[next] !== undefined) props.onSelect(order[next]);
  }

  /** Lance une action sur le disque, puis rafraîchit la liste. */
  async function act(work: () => Promise<string | void>) {
    try {
      const m = await work();
      if (m) props.onMessage(m);
    } catch (e) {
      props.onMessage(`Erreur : ${e}`);
    }
    props.onChanged();
  }

  /** Actions communes à un dossier ou à un .als (`path`). */
  function pathItems(path: string, label: string, hidden: boolean): MenuItem[] {
    const name = baseName(path);
    return [
      {
        label: "Renommer…",
        action: () => {
          const n = window.prompt(`Nouveau nom pour « ${name} » :`, name.replace(/\.als$/i, ""));
          if (n?.trim() && n !== name) act(async () => void (await api.renamePath(path, n)));
        },
      },
      {
        label: "Déplacer vers…",
        action: async () => {
          const dest = await open({ directory: true, defaultPath: parentDir(path), title: `Déplacer « ${name} »` });
          if (typeof dest === "string") act(async () => `Déplacé dans ${await api.movePath(path, dest)}`);
        },
      },
      {
        label: "Ranger dans un nouveau dossier…",
        action: () => {
          const n = window.prompt(`Nom du nouveau dossier (créé à côté de « ${name} ») :`);
          if (!n?.trim()) return;
          act(async () => {
            const dir = await api.createDir(parentDir(path), n);
            return `Rangé dans ${await api.movePath(path, dir)}`;
          });
        },
      },
      {
        label: hidden ? "Ne plus masquer" : `Masquer ${label}`,
        action: () => act(() => api.setHidden(path, !hidden)),
      },
      {
        label: `Supprimer ${label}…`,
        danger: true,
        action: async () => {
          const ok = await ask(`Envoyer « ${name} » à la corbeille ?\n${path}`, {
            title: "Supprimer",
            kind: "warning",
            okLabel: "Corbeille",
            cancelLabel: "Annuler",
          });
          if (ok) act(async () => `${await api.deletePath(path)} projet(s) supprimé(s)`);
        },
      },
    ];
  }

  function projectMenu(e: React.MouseEvent, p: ProjectSummary, inGroup: boolean) {
    e.preventDefault();
    props.onSelect(p.id);
    // Seul dans son dossier : on agit sur le dossier du projet entier.
    const path = inGroup || !p.als_path ? (p.als_path ?? p.folder) : p.folder;
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: "▶ Ouvrir dans Live", action: () => api.openInAbleton(p.id) },
        { label: "Ouvrir le dossier", action: () => api.openFolder(p.id) },
        ...pathItems(path, inGroup ? "ce set" : "ce projet", p.hidden),
      ],
    });
  }

  function folderMenu(e: React.MouseEvent, folder: string, ps: ProjectSummary[]) {
    e.preventDefault();
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: "Ouvrir le dossier", action: () => api.openFolder(ps[0].id) },
        ...pathItems(folder, "ce dossier", ps.every((p) => p.hidden)),
      ],
    });
  }

  const row = (p: ProjectSummary, inGroup: boolean) => (
    <div
      key={p.id}
      data-id={p.id}
      className={`erow ${p.id === selected ? "selected" : ""} ${inGroup ? "nested" : ""} ${p.hidden ? "hidden-item" : ""}`}
      onClick={() => props.onSelect(p.id)}
      onContextMenu={(e) => projectMenu(e, p, inGroup)}
      title={p.folder}
    >
      <button
        className={`icon listen ${props.playing === p.id ? "active" : ""}`}
        disabled={!p.preview_hash}
        title={p.preview_hash ? "Écouter l'aperçu" : "Pas encore d'aperçu"}
        onClick={(e) => {
          e.stopPropagation();
          props.onPlay(p);
        }}
      >
        ▶
      </button>
      <div className="erow-text">
        <span className="erow-name">
          {p.favorite && <span className="fav">★ </span>}
          {p.name}
        </span>
        <small>
          <span className={`sdot ${p.status}`} title={STATUS_LABELS[p.status]} />
          {p.tempo ? `${p.tempo} BPM · ` : ""}
          {sort === "worked"
            ? formatDuration(p.time_spent)
            : sort === "session"
              ? p.last_worked_at
                ? formatAgo(p.last_worked_at)
                : "jamais travaillé"
              : formatDate(p.modified_at)}
          {p.tags.length > 0 && ` · ${p.tags.join(", ")}`}
        </small>
      </div>
    </div>
  );

  return (
    <div className="explorer-body" onKeyDown={onKeyDown}>
      <input
        className="search"
        type="search"
        placeholder="Projet, plugin, sample, tag…"
        value={filter.query ?? ""}
        onChange={(e) => props.onFilter({ ...filter, query: e.target.value })}
        autoFocus
      />
      <div className="filters">
        <select
          value={filter.status ?? ""}
          onChange={(e) =>
            props.onFilter({
              ...filter,
              status: (e.target.value || undefined) as Status | undefined,
            })
          }
          title="Statut"
        >
          <option value="">Tous statuts</option>
          {Object.entries(STATUS_LABELS).map(([k, label]) => (
            <option key={k} value={k}>
              {label}
            </option>
          ))}
        </select>
        {props.tags.length > 0 && (
          <select
            value={filter.tag ?? ""}
            onChange={(e) => props.onFilter({ ...filter, tag: e.target.value || undefined })}
            title="Tag"
          >
            <option value="">Tous tags</option>
            {props.tags.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        )}
        <select value={sort} onChange={(e) => setSort(e.target.value as Sort)} title="Trier">
          {Object.entries(SORTS).map(([k, label]) => (
            <option key={k} value={k}>
              {label}
            </option>
          ))}
        </select>
        <button
          className={`fav-filter ${filter.show_hidden ? "active" : ""}`}
          onClick={() => props.onFilter({ ...filter, show_hidden: !filter.show_hidden })}
          title="Afficher les éléments masqués"
        >
          👁
        </button>
        <button
          className={`fav-filter ${filter.favorites_only ? "active" : ""}`}
          onClick={() => props.onFilter({ ...filter, favorites_only: !filter.favorites_only })}
          title="Favoris seulement"
        >
          ★
        </button>
      </div>

      <div className="elist" ref={list} tabIndex={0}>
        {projects.length === 0 && <p className="empty">Aucun projet trouvé.</p>}
        {groups.map(([folder, ps]) =>
          ps.length === 1 ? (
            row(ps[0], false)
          ) : (
            <div
              key={folder}
              className={`egroup ${ps.every((p) => p.hidden) ? "hidden-item" : ""} ${collapsed.has(folder) ? "collapsed" : ""}`}
            >
              <div
                className="egroup-head"
                title={folder}
                onClick={() =>
                  setCollapsed((c) => {
                    const n = new Set(c);
                    if (!n.delete(folder)) n.add(folder);
                    return n;
                  })
                }
                onContextMenu={(e) => folderMenu(e, folder, ps)}
              >
                <span>
                  {collapsed.has(folder) ? "▸" : "▾"} 📁 {baseName(folder)}
                </span>
                <small>{ps.length} sets</small>
              </div>
              {ps.map((p) => row(p, true))}
            </div>
          ),
        )}
      </div>
      {menu && <ContextMenu menu={menu} onClose={() => setMenu(null)} />}
      <small className="ecount">
        {projects.length} projet{projects.length > 1 ? "s" : ""} · ↑ ↓ pour naviguer · clic droit : actions
      </small>
    </div>
  );
}
