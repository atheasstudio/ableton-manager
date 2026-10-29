import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, formatAgo, formatDateTime, formatDuration, ProjectTime, WorkSession } from "../api";
import { toast } from "./Toasts";
import "../ui.css";

interface Props {
  projectId: number;
  /** Appelé quand le temps du projet change (session supprimée…). */
  onChange?: () => void;
}

const WEEKS = 12;
const DAY_NAMES = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];

/** « AAAA-MM-JJ » en heure locale. */
function dayKey(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** Intensité d'une case, de 0 (rien) à 4 (grosse journée). */
function level(seconds: number): number {
  if (seconds <= 0) return 0;
  if (seconds < 30 * 60) return 1;
  if (seconds < 90 * 60) return 2;
  if (seconds < 3 * 3600) return 3;
  return 4;
}

/** Projet de l'évènement worktime-changed (un id, ou un objet { project_id }). */
export function worktimeProject(payload: unknown): number | null {
  if (typeof payload === "number") return payload;
  if (payload && typeof payload === "object" && "project_id" in payload) {
    const id = (payload as { project_id: unknown }).project_id;
    return typeof id === "number" ? id : null;
  }
  return null;
}

/**
 * Grille des 12 dernières semaines : une colonne par semaine (lundi en haut),
 * une case par jour.
 */
function DayGrid({ days }: { days: [string, number][] }) {
  const seconds = new Map(days);
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  // Lundi de la semaine en cours, puis on recule de 11 semaines.
  const start = new Date(today);
  start.setDate(start.getDate() - ((start.getDay() + 6) % 7) - (WEEKS - 1) * 7);

  const columns: { key: string; s: number; future: boolean; date: Date }[][] = [];
  for (let w = 0; w < WEEKS; w++) {
    const col = [];
    for (let d = 0; d < 7; d++) {
      const date = new Date(start);
      date.setDate(start.getDate() + w * 7 + d);
      const key = dayKey(date);
      col.push({ key, s: seconds.get(key) ?? 0, future: date > today, date });
    }
    columns.push(col);
  }

  return (
    <div className="wt-grid" role="img" aria-label="Temps de travail par jour, 12 dernières semaines">
      <div className="wt-grid-days">
        {DAY_NAMES.map((n, i) => (
          <span key={n}>{i % 2 === 0 ? n : ""}</span>
        ))}
      </div>
      {columns.map((col) => (
        <div key={col[0].key} className="wt-grid-week">
          {col.map((c) => (
            <span
              key={c.key}
              className={`wt-cell l${level(c.s)} ${c.future ? "future" : ""}`}
              title={
                c.future
                  ? undefined
                  : `${c.date.toLocaleDateString("fr-FR", { weekday: "long", day: "numeric", month: "long" })} : ${
                      c.s > 0 ? formatDuration(c.s) : "rien"
                    }`
              }
            />
          ))}
        </div>
      ))}
    </div>
  );
}

/** Onglet « Temps » du dock : temps passé dans Live sur ce projet. */
export default function WorktimeTab({ projectId, onChange }: Props) {
  const [time, setTime] = useState<ProjectTime | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** Sessions dont la suppression attend l'expiration de la notification. */
  const [hidden, setHidden] = useState<number[]>([]);

  function load() {
    api.projectTime(projectId).then(
      (t) => {
        setTime(t);
        setError(null);
      },
      (e) => setError(String(e)),
    );
  }

  useEffect(() => {
    setTime(null);
    load();
    const unlisten = listen("worktime-changed", (e) => {
      if (worktimeProject(e.payload) === projectId) load();
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [projectId]);

  /** Suppression différée : la session disparaît, « Annuler » la fait revenir. */
  function remove(s: WorkSession) {
    setHidden((h) => [...h, s.id]);
    toast({
      message: `Session de ${formatDuration(s.ended_at - s.started_at)} supprimée`,
      undo: () => setHidden((h) => h.filter((x) => x !== s.id)),
      commit: async () => {
        try {
          await api.deleteWorkSession(s.id);
        } catch (e) {
          setHidden((h) => h.filter((x) => x !== s.id));
          throw e;
        }
        load();
        onChange?.();
      },
    });
  }

  if (error && !time) return <p className="missing">Temps indisponible : {error}</p>;
  if (!time) return null;

  if (time.sessions === 0) {
    return (
      <div className="worktime empty">
        <p>
          Aucune session pour l'instant. Le temps se compte tout seul quand ce set est ouvert dans
          Live (et que tu y travailles).
        </p>
      </div>
    );
  }

  return (
    <div className="worktime">
      <div className="wt-stats">
        <div>
          <strong>{formatDuration(time.total)}</strong>
          <small>
            au total · {time.sessions} session{time.sessions > 1 ? "s" : ""}
          </small>
        </div>
        <div>
          <strong>{formatDuration(time.week)}</strong>
          <small>7 derniers jours</small>
        </div>
        <div>
          <strong>{formatDuration(time.month)}</strong>
          <small>30 derniers jours</small>
        </div>
        {time.first !== null && (
          <div>
            <strong>{formatAgo(time.first)}</strong>
            <small>première session</small>
          </div>
        )}
      </div>

      <DayGrid days={time.days} />

      <div className="wt-sessions">
        <h4>Sessions récentes</h4>
        <ul>
          {time.recent.filter((s) => !hidden.includes(s.id)).map((s) => (
            <li key={s.id}>
              <span className="num">{formatDuration(s.ended_at - s.started_at)}</span>
              <span title={formatDateTime(s.started_at)}>
                {formatAgo(s.started_at)}, {new Date(s.started_at * 1000).toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit" })}
              </span>
              {s.branch && <span className="wt-branch">{s.branch}</span>}
              <button
                className="icon"
                onClick={() => remove(s)}
                title="Supprimer cette session (elle ne compte plus dans le temps)"
                aria-label="Supprimer la session"
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      </div>
      {error && <small className="missing">{error}</small>}
    </div>
  );
}
