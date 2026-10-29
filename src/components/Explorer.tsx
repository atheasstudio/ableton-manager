import { useEffect, useMemo, useRef } from "react";
import { Filter, formatDate, ProjectSummary, STATUS_LABELS, Status } from "../api";

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

  // Groupes dans l'ordre de la liste (le plus récent d'abord).
  const groups = useMemo(() => {
    const byFolder = new Map<string, ProjectSummary[]>();
    for (const p of projects) {
      const g = byFolder.get(p.folder);
      if (g) g.push(p);
      else byFolder.set(p.folder, [p]);
    }
    return [...byFolder.entries()];
  }, [projects]);
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

  const row = (p: ProjectSummary, inGroup: boolean) => (
    <div
      key={p.id}
      data-id={p.id}
      className={`erow ${p.id === selected ? "selected" : ""} ${inGroup ? "nested" : ""}`}
      onClick={() => props.onSelect(p.id)}
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
          {formatDate(p.modified_at)}
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
            <div key={folder} className="egroup">
              <div className="egroup-head" title={folder}>
                <span>📁 {baseName(folder)}</span>
                <small>{ps.length} sets</small>
              </div>
              {ps.map((p) => row(p, true))}
            </div>
          ),
        )}
      </div>
      <small className="ecount">
        {projects.length} projet{projects.length > 1 ? "s" : ""} · ↑ ↓ pour naviguer
      </small>
    </div>
  );
}
