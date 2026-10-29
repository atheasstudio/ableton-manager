import { ReactNode, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Branch, formatDateTime, TreeNote, Version, versionLabel } from "../api";

/** Une couleur par branche, la principale en orange. */
export const LANE_COLORS = ["#ff7a2f", "#6cb6ff", "#57ab5a", "#d2a8ff", "#e3b341", "#f47067"];

export function laneColor(branches: Branch[], branchId: number): string {
  const i = branches.findIndex((b) => b.id === branchId);
  return LANE_COLORS[Math.max(i, 0) % LANE_COLORS.length];
}

// Géométrie du monde (en pixels à l'échelle 1).
const NODE_W = 190;
const NODE_H = 58;
const COL = 240;
const LANE = 130;
const NOTE_W = 170;
const MIN_ZOOM = 0.15;
const MAX_ZOOM = 2.5;

const SOURCE_LABELS: Record<Version["source"], string> = {
  save: "",
  manual: "manuelle",
  backup: "Backup de Live",
  restore: "restauration",
};

interface View {
  x: number;
  y: number;
  k: number;
}

interface Props {
  branches: Branch[];
  /** De la plus récente à la plus ancienne (ordre de l'API). */
  versions: Version[];
  notes: TreeNote[];
  selected: number | null;
  onSelect: (id: number | null) => void;
  /** Versions cochées pour la comparaison (Maj + clic). */
  checked: number[];
  onToggleCheck: (id: number) => void;
  /** Ouvre le set d'une branche dans Live. */
  onOpenBranch: (branchId: number) => void;
  /** Clic droit sur une version ou sur le nom d'une branche. */
  onVersionMenu: (v: Version, e: React.MouseEvent) => void;
  onBranchMenu: (b: Branch, e: React.MouseEvent) => void;
  onAddNote: (versionId: number | null, x: number, y: number) => Promise<TreeNote>;
  onUpdateNote: (note: TreeNote) => void;
  onDeleteNote: (id: number) => void;
  /** Contenu posé en haut à gauche du graphe (détail de la version, messages…). */
  overlay?: ReactNode;
}

/**
 * Arbre des versions dans un espace 2D : le temps va de gauche à droite, une
 * ligne par branche. On s'y déplace en glissant, on zoome à la molette, et
 * on y pose des notes, libres ou accrochées à une version.
 */
export default function VersionGraph(props: Props) {
  const { branches, versions, notes, selected, checked } = props;
  const viewport = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<View>({ x: 40, y: 40, k: 1 });
  const [fullscreen, setFullscreen] = useState(false);
  const [editing, setEditing] = useState<number | null>(null);
  /** Note en cours de déplacement : position provisoire dans le monde. */
  const [dragNote, setDragNote] = useState<{ id: number; x: number; y: number } | null>(null);
  const pan = useRef<{ px: number; py: number; x: number; y: number; moved: boolean } | null>(null);

  // Position de chaque version : colonne = ordre chronologique, ligne = branche.
  const layout = useMemo(() => {
    const lane = new Map(branches.map((b, i) => [b.id, i]));
    const chrono = [...versions].sort((a, b) => a.created_at - b.created_at || a.id - b.id);
    const pos = new Map<number, { x: number; y: number }>();
    chrono.forEach((v, i) => pos.set(v.id, { x: i * COL, y: (lane.get(v.branch_id) ?? 0) * LANE }));
    // Branche créée mais pas encore sauvegardée : un fantôme après sa version d'origine.
    const ghosts = branches.flatMap((b) => {
      if (b.is_main || b.head_id === null || b.head_id !== b.base_id) return [];
      const from = pos.get(b.head_id);
      if (!from) return [];
      return [{ branch: b, from, x: from.x + COL, y: (lane.get(b.id) ?? 0) * LANE }];
    });
    return { pos, ghosts };
  }, [branches, versions]);

  /** Position d'une note dans le monde. */
  function notePos(n: TreeNote): { x: number; y: number } {
    if (dragNote?.id === n.id) return dragNote;
    const anchor = n.version_id === null ? undefined : layout.pos.get(n.version_id);
    return anchor ? { x: anchor.x + n.x, y: anchor.y + n.y } : { x: n.x, y: n.y };
  }

  /** Coordonnées écran (client) → monde. */
  function toWorld(clientX: number, clientY: number) {
    const r = viewport.current!.getBoundingClientRect();
    return { x: (clientX - r.left - view.x) / view.k, y: (clientY - r.top - view.y) / view.k };
  }

  /** Version sous un point du monde. */
  function versionAt(x: number, y: number): number | null {
    for (const [id, p] of layout.pos) {
      if (x >= p.x && x <= p.x + NODE_W && y >= p.y && y <= p.y + NODE_H) return id;
    }
    return null;
  }

  /** Cadre tout le graphe ; `minZoom` garde les nœuds lisibles à l'ouverture. */
  function fit(minZoom = MIN_ZOOM) {
    const el = viewport.current;
    if (!el) return;
    const boxes = [
      ...[...layout.pos.values()].map((p) => [p.x, p.y, p.x + NODE_W, p.y + NODE_H]),
      ...layout.ghosts.map((g) => [g.x, g.y, g.x + NODE_W, g.y + NODE_H]),
      ...notes.map((n) => {
        const p = notePos(n);
        return [p.x, p.y, p.x + NOTE_W, p.y + 70];
      }),
    ];
    if (boxes.length === 0) return;
    const [x0, y0, x1, y1] = [
      Math.min(...boxes.map((b) => b[0])),
      Math.min(...boxes.map((b) => b[1])),
      Math.max(...boxes.map((b) => b[2])),
      Math.max(...boxes.map((b) => b[3])),
    ];
    const pad = 40;
    const k = Math.min(
      1,
      Math.max(
        minZoom,
        Math.min((el.clientWidth - 2 * pad) / (x1 - x0), (el.clientHeight - 2 * pad) / (y1 - y0)),
      ),
    );
    // Si tout ne tient pas, on montre la fin (les versions récentes).
    const width = (x1 - x0) * k;
    const x = width > el.clientWidth - 2 * pad ? el.clientWidth - pad - x1 * k : pad - x0 * k;
    setView({ x, y: pad - y0 * k, k });
  }

  // Ajuste à l'ouverture et au passage en plein écran.
  const fitted = useRef(false);
  useLayoutEffect(() => {
    if (!fitted.current && versions.length > 0) {
      fitted.current = true;
      fit(0.6);
    }
  }, [versions.length]);
  useLayoutEffect(() => fit(0.6), [fullscreen]);

  // Molette : zoom autour du curseur (écouteur non passif pour bloquer le défilement).
  useEffect(() => {
    const el = viewport.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = el.getBoundingClientRect();
      const [mx, my] = [e.clientX - r.left, e.clientY - r.top];
      setView((v) => {
        const k = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, v.k * Math.exp(-e.deltaY * 0.0015)));
        return { k, x: mx - ((mx - v.x) * k) / v.k, y: my - ((my - v.y) * k) / v.k };
      });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  function zoomBy(factor: number) {
    const el = viewport.current!;
    const [mx, my] = [el.clientWidth / 2, el.clientHeight / 2];
    setView((v) => {
      const k = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, v.k * factor));
      return { k, x: mx - ((mx - v.x) * k) / v.k, y: my - ((my - v.y) * k) / v.k };
    });
  }

  // Glisser le fond : se déplacer. Un clic sans glisser désélectionne.
  function onPointerDown(e: React.PointerEvent) {
    if (e.button !== 0 || (e.target as HTMLElement).closest(".gnode, .gnote, .graph-ui")) return;
    pan.current = { px: e.clientX, py: e.clientY, x: view.x, y: view.y, moved: false };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onPointerMove(e: React.PointerEvent) {
    const p = pan.current;
    if (!p) return;
    const [dx, dy] = [e.clientX - p.px, e.clientY - p.py];
    if (Math.abs(dx) + Math.abs(dy) > 3) p.moved = true;
    setView((v) => ({ ...v, x: p.x + dx, y: p.y + dy }));
  }
  function onPointerUp() {
    if (pan.current && !pan.current.moved) {
      props.onSelect(null);
      setEditing(null);
    }
    pan.current = null;
  }

  // Double-clic dans le vide : une note libre à cet endroit.
  async function onDoubleClick(e: React.MouseEvent) {
    if ((e.target as HTMLElement).closest(".gnode, .gnote, .graph-ui")) return;
    const p = toWorld(e.clientX, e.clientY);
    const note = await props.onAddNote(null, p.x - NOTE_W / 2, p.y - 20);
    setEditing(note.id);
  }

  async function addNoteFromToolbar() {
    const el = viewport.current!;
    let note: TreeNote;
    if (selected !== null) {
      // Au-dessus de la version choisie, en la suivant.
      note = await props.onAddNote(selected, 10, -90);
    } else {
      const c = toWorld(
        el.getBoundingClientRect().left + el.clientWidth / 2,
        el.getBoundingClientRect().top + el.clientHeight / 2,
      );
      note = await props.onAddNote(null, c.x - NOTE_W / 2, c.y - 30);
    }
    setEditing(note.id);
  }

  // Déplacer une note ; lâchée sur une version, elle s'y accroche.
  function startNoteDrag(e: React.PointerEvent, n: TreeNote) {
    if (e.button !== 0 || editing === n.id) return;
    e.stopPropagation();
    const start = toWorld(e.clientX, e.clientY);
    const origin = notePos(n);
    let current = origin;
    let moved = false;
    const move = (ev: PointerEvent) => {
      const p = toWorld(ev.clientX, ev.clientY);
      current = { x: origin.x + p.x - start.x, y: origin.y + p.y - start.y };
      moved = moved || Math.abs(p.x - start.x) + Math.abs(p.y - start.y) > 2;
      setDragNote({ id: n.id, ...current });
    };
    const up = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      setDragNote(null);
      if (!moved) return;
      const drop = toWorld(ev.clientX, ev.clientY);
      const target = versionAt(drop.x, drop.y) ?? n.version_id;
      const anchor = target === null ? undefined : layout.pos.get(target);
      props.onUpdateNote(
        anchor
          ? { ...n, version_id: target, x: current.x - anchor.x, y: current.y - anchor.y }
          : { ...n, version_id: null, x: current.x, y: current.y },
      );
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  // Flèches : ← version parente, → version suivante (enfant le plus récent).
  function onKeyDown(e: React.KeyboardEvent) {
    if ((e.target as HTMLElement).closest("input, textarea") || selected === null) return;
    const v = versions.find((x) => x.id === selected);
    if (!v) return;
    let next: number | null = null;
    if (e.key === "ArrowLeft") next = v.parent_id;
    if (e.key === "ArrowRight") next = versions.find((x) => x.parent_id === v.id)?.id ?? null;
    if (next !== null) {
      e.preventDefault();
      props.onSelect(next);
      centerOn(next);
    }
  }

  function centerOn(id: number) {
    const p = layout.pos.get(id);
    const el = viewport.current;
    if (!p || !el) return;
    setView((v) => ({
      ...v,
      x: el.clientWidth / 2 - (p.x + NODE_W / 2) * v.k,
      y: el.clientHeight / 2 - (p.y + NODE_H / 2) * v.k,
    }));
  }

  const edges = versions.flatMap((v) => {
    const a = v.parent_id === null ? undefined : layout.pos.get(v.parent_id);
    const b = layout.pos.get(v.id);
    if (!a || !b) return [];
    const [x1, y1, x2, y2] = [a.x + NODE_W, a.y + NODE_H / 2, b.x, b.y + NODE_H / 2];
    const c = Math.max(40, (x2 - x1) / 2);
    return [
      <path
        key={v.id}
        d={`M ${x1} ${y1} C ${x1 + c} ${y1}, ${x2 - c} ${y2}, ${x2} ${y2}`}
        stroke={laneColor(branches, v.branch_id)}
      />,
    ];
  });

  const bounds = useMemo(() => {
    const xs = [...layout.pos.values()].map((p) => p.x);
    const ys = [...layout.pos.values()].map((p) => p.y);
    return { w: Math.max(0, ...xs) + COL * 2, h: Math.max(0, ...ys) + LANE * 2 };
  }, [layout]);

  return (
    <div className={`graph ${fullscreen ? "fullscreen" : ""}`}>
      <div
        ref={viewport}
        className="graph-viewport"
        tabIndex={0}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onDoubleClick={onDoubleClick}
        onKeyDown={onKeyDown}
        style={{
          backgroundPosition: `${view.x}px ${view.y}px`,
          backgroundSize: `${24 * view.k}px ${24 * view.k}px`,
        }}
      >
        <div
          className="graph-world"
          style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.k})` }}
        >
          <svg
            className="graph-edges"
            width={bounds.w}
            height={bounds.h}
            style={{ left: -COL, top: -LANE }}
            viewBox={`${-COL} ${-LANE} ${bounds.w} ${bounds.h}`}
          >
            <g fill="none" strokeWidth={2.5}>
              {edges}
              {layout.ghosts.map((g) => (
                <path
                  key={g.branch.id}
                  d={`M ${g.from.x + NODE_W} ${g.from.y + NODE_H / 2} C ${g.from.x + NODE_W + 25} ${g.from.y + NODE_H / 2}, ${g.x - 25} ${g.y + NODE_H / 2}, ${g.x} ${g.y + NODE_H / 2}`}
                  stroke={laneColor(branches, g.branch.id)}
                  strokeDasharray="6 5"
                />
              ))}
            </g>
            <g fill="none" strokeWidth={1.5} strokeDasharray="3 4" stroke="var(--note)">
              {notes.map((n) => {
                const anchor = n.version_id === null ? undefined : layout.pos.get(n.version_id);
                if (!anchor) return null;
                const p = notePos(n);
                return (
                  <line
                    key={n.id}
                    x1={p.x + NOTE_W / 2}
                    y1={p.y + 20}
                    x2={anchor.x + NODE_W / 2}
                    y2={anchor.y + NODE_H / 2}
                  />
                );
              })}
            </g>
          </svg>

          {branches.map((b, i) => {
            const first = [...layout.pos.entries()]
              .filter(([id]) => versions.find((v) => v.id === id)?.branch_id === b.id)
              .map(([, p]) => p.x);
            const ghost = layout.ghosts.find((g) => g.branch.id === b.id);
            const x = first.length ? Math.min(...first) : ghost?.x;
            if (x === undefined) return null;
            return (
              <div
                key={b.id}
                className="glane-label"
                style={{ left: x, top: i * LANE - 30, color: laneColor(branches, b.id) }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  props.onBranchMenu(b, e);
                }}
              >
                {b.name}
                <button
                  className="glane-open graph-ui"
                  style={{ borderColor: laneColor(branches, b.id) }}
                  onClick={() => props.onOpenBranch(b.id)}
                  title={`Ouvrir le set de « ${b.name} » dans Live`}
                >
                  ▶ Ouvrir dans Live
                </button>
              </div>
            );
          })}

          {layout.ghosts.map((g) => (
            <div
              key={g.branch.id}
              className="gnode ghost"
              style={{ left: g.x, top: g.y, borderColor: laneColor(branches, g.branch.id) }}
              title="Branche créée, pas encore de sauvegarde"
            >
              <span className="gnode-name">{g.branch.name}</span>
              <small>pas encore de sauvegarde</small>
              <button
                className="gnode-open"
                style={{ background: laneColor(branches, g.branch.id) }}
                onClick={() => props.onOpenBranch(g.branch.id)}
              >
                ▶ Ouvrir dans Live
              </button>
            </div>
          ))}

          {versions.map((v) => {
            const p = layout.pos.get(v.id)!;
            const color = laneColor(branches, v.branch_id);
            const heads = branches.filter((b) => b.head_id === v.id);
            const mark = checked.indexOf(v.id);
            return (
              <div
                key={v.id}
                className={`gnode ${v.id === selected ? "selected" : ""} ${mark >= 0 ? "checked" : ""} ${v.source}`}
                style={{ left: p.x, top: p.y, borderColor: color }}
                onClick={(e) => {
                  if (e.shiftKey) props.onToggleCheck(v.id);
                  else props.onSelect(v.id);
                }}
                onDoubleClick={() => centerOn(v.id)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  props.onSelect(v.id);
                  props.onVersionMenu(v, e);
                }}
                title={`${v.auto_name}\nMaj + clic : cocher pour comparer`}
              >
                <span className="gnode-name">{versionLabel(v)}</span>
                <small>
                  {formatDateTime(v.created_at)}
                  {SOURCE_LABELS[v.source] && ` · ${SOURCE_LABELS[v.source]}`}
                </small>
                {v.missing > 0 && (
                  <small className="missing">{v.missing} audio(s) manquant(s)</small>
                )}
                {mark >= 0 && <span className="gnode-mark">{mark === 0 ? "A" : "B"}</span>}
                {heads.length > 0 && (
                  <span className="gnode-head" style={{ background: color }}>
                    {heads.map((b) => b.name).join(", ")}
                  </span>
                )}
              </div>
            );
          })}

          {notes.map((n) => {
            const p = notePos(n);
            return (
              <div
                key={n.id}
                className={`gnote ${dragNote?.id === n.id ? "dragging" : ""}`}
                style={{ left: p.x, top: p.y, width: NOTE_W }}
                onPointerDown={(e) => startNoteDrag(e, n)}
                onDoubleClick={() => setEditing(n.id)}
              >
                {n.version_id !== null && (
                  <button
                    className="icon gnote-detach"
                    title="Détacher de la version (la note reste en place)"
                    onPointerDown={(e) => e.stopPropagation()}
                    onClick={() => {
                      const p = notePos(n);
                      props.onUpdateNote({ ...n, version_id: null, x: p.x, y: p.y });
                    }}
                  >
                    ⛓
                  </button>
                )}
                <button
                  className="icon gnote-delete"
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={() => props.onDeleteNote(n.id)}
                  aria-label="Supprimer la note"
                >
                  ×
                </button>
                {editing === n.id ? (
                  <textarea
                    autoFocus
                    defaultValue={n.text}
                    placeholder="Ta note…"
                    onPointerDown={(e) => e.stopPropagation()}
                    onBlur={(e) => {
                      setEditing(null);
                      if (e.target.value !== n.text)
                        props.onUpdateNote({ ...n, text: e.target.value });
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Escape" || (e.key === "Enter" && (e.ctrlKey || e.metaKey))) {
                        (e.target as HTMLTextAreaElement).blur();
                      }
                    }}
                  />
                ) : (
                  <p>{n.text || <em>Double-clic pour écrire</em>}</p>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {props.overlay && <div className="graph-overlay graph-ui">{props.overlay}</div>}

      <div className="graph-toolbar graph-ui">
        <button
          onClick={addNoteFromToolbar}
          title="Ajouter une note (sur la version choisie, sinon au centre)"
        >
          + Note
        </button>
        <button onClick={() => zoomBy(1 / 1.25)} aria-label="Dézoomer">
          −
        </button>
        <button onClick={() => zoomBy(1.25)} aria-label="Zoomer">
          +
        </button>
        <button onClick={() => fit()} title="Tout voir">
          Ajuster
        </button>
        <button onClick={() => setFullscreen(!fullscreen)}>
          {fullscreen ? "Réduire" : "Plein écran"}
        </button>
      </div>
      <p className="graph-help graph-ui">
        Glisser : se déplacer · Molette : zoom · Double-clic dans le vide : note · Maj + clic :
        comparer · ← → : versions · ▶ Ouvrir dans Live : travailler sur une branche
      </p>
    </div>
  );
}
