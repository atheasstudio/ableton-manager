import { useEffect, useMemo, useRef, useState } from "react";
import { api, Collection, ProjectSummary, STATUS_LABELS } from "../api";
import { emit, SHORTCUTS } from "../shortcuts";
import "../ui.css";

export interface PaletteAction {
  id: string;
  label: string;
  /** Texte en plus pour la recherche (synonymes). */
  keywords?: string;
  hint?: string;
  disabled?: boolean;
  run: () => void;
}

interface Props {
  /** Actions de l'application (ouvrir dans Live, scanner…). */
  actions: PaletteAction[];
  onSelectProject: (id: number) => void;
  onClose: () => void;
}

type Entry =
  | { kind: "action"; action: PaletteAction }
  | { kind: "project"; project: ProjectSummary };

/** Minuscules, sans accents. */
function norm(s: string): string {
  return s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
}

/** Tous les mots de la recherche apparaissent dans le texte. */
function matches(query: string[], text: string): boolean {
  const t = norm(text);
  return query.every((w) => t.includes(w));
}

const MAX_PROJECTS = 30;

/** Palette (Ctrl+K) : chercher un projet ou lancer une action. */
export default function CommandPalette({ actions, onSelectProject, onClose }: Props) {
  const [query, setQuery] = useState("");
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [collections, setCollections] = useState<Collection[]>([]);
  const [active, setActive] = useState(0);
  const list = useRef<HTMLUListElement>(null);

  useEffect(() => {
    api.searchProjects({}).then(setProjects, () => {});
    // Les collections viennent du back-end (lot C) ; sans elles, la palette marche quand même.
    api.listCollections().then(setCollections, () => setCollections([]));
  }, []);

  const entries = useMemo<Entry[]>(() => {
    const words = norm(query).split(/\s+/).filter(Boolean);
    const all: PaletteAction[] = [
      ...actions,
      ...collections.map((c) => ({
        id: `collection:${c.id}`,
        label: `Collection : ${c.name}`,
        keywords: "filtre collection",
        run: () => emit("am:apply-filter", { filter: c.filter }),
      })),
    ];
    const acts = all
      .filter((a) => !a.disabled && matches(words, `${a.label} ${a.keywords ?? ""}`))
      .map((action) => ({ kind: "action" as const, action }));
    const projs = words.length
      ? projects
          .filter((p) => matches(words, `${p.name} ${p.tags.join(" ")} ${STATUS_LABELS[p.status]}`))
          .slice(0, MAX_PROJECTS)
          .map((project) => ({ kind: "project" as const, project }))
      : [];
    // Sans recherche : les actions ; avec : les projets d'abord, c'est le plus courant.
    return words.length ? [...projs, ...acts] : acts;
  }, [query, actions, collections, projects]);

  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    list.current?.querySelector(".active")?.scrollIntoView({ block: "nearest" });
  }, [active]);

  function choose(e: Entry | undefined) {
    if (!e) return;
    onClose();
    if (e.kind === "action") e.action.run();
    else onSelectProject(e.project.id);
  }

  function onKey(e: React.KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(entries.length - 1, a + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(0, a - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      choose(entries[active]);
    } else if (e.key === "Escape" || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k")) {
      e.preventDefault();
      onClose();
    }
  }

  return (
    <div className="overlay palette-overlay" onClick={onClose}>
      <div className="palette" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Palette">
        <input
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKey}
          placeholder="Projet ou action…"
          aria-label="Chercher un projet ou une action"
        />
        <ul ref={list}>
          {entries.map((e, i) => (
            <li
              key={e.kind === "action" ? e.action.id : `p${e.project.id}`}
              className={i === active ? "active" : ""}
              onMouseMove={() => setActive(i)}
              onClick={() => choose(e)}
            >
              {e.kind === "action" ? (
                <>
                  <span className="palette-kind">Action</span>
                  <span className="palette-label">{e.action.label}</span>
                  {e.action.hint && <small>{e.action.hint}</small>}
                </>
              ) : (
                <>
                  <span className="palette-kind">{e.project.favorite ? "★" : "Projet"}</span>
                  <span className="palette-label">{e.project.name}</span>
                  <small>{STATUS_LABELS[e.project.status]}</small>
                </>
              )}
            </li>
          ))}
          {entries.length === 0 && <li className="empty">Rien ne correspond.</li>}
        </ul>
      </div>
    </div>
  );
}

/** Aide (touche ?) : la liste des raccourcis. */
export function ShortcutsHelp({ onClose }: { onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" || e.key === "?") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="overlay" onClick={onClose}>
      <div className="dialog shortcuts-help" onClick={(e) => e.stopPropagation()} role="dialog">
        <header>
          <h2>Raccourcis</h2>
          <button className="icon" onClick={onClose} aria-label="Fermer">
            ×
          </button>
        </header>
        <dl>
          {SHORTCUTS.map(([keys, label]) => (
            <div key={keys}>
              <dt>
                {keys.split(" + ").map((k, i) => (
                  <span key={i}>
                    {i > 0 && " + "}
                    <kbd>{k}</kbd>
                  </span>
                ))}
              </dt>
              <dd>{label}</dd>
            </div>
          ))}
        </dl>
        <p className="muted">
          Les touches seules (Espace, /, F, ?) ne font rien quand tu tapes dans un champ.
        </p>
      </div>
    </div>
  );
}
