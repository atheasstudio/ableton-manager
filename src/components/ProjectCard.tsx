import { useEffect, useRef, useState } from "react";
import { api, formatAgo, formatDuration, PreviewInfo, ProjectSummary, STATUS_LABELS } from "../api";
import Waveform from "./Waveform";

/** Aperçus déjà demandés, partagés par toutes les cartes (clé : empreinte). */
const previews = new Map<string, Promise<PreviewInfo | null>>();

function loadPreview(hash: string): Promise<PreviewInfo | null> {
  let p = previews.get(hash);
  if (!p) {
    p = api.getPreview(hash).catch(() => {
      previews.delete(hash); // réessayer la prochaine fois
      return null;
    });
    previews.set(hash, p);
  }
  return p;
}

/** Vrai dès que l'élément est passé au moins une fois près de la zone visible. */
function useSeen(ref: React.RefObject<HTMLElement>): boolean {
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el || seen) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setSeen(true);
          observer.disconnect();
        }
      },
      { rootMargin: "200px 0px" },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref, seen]);
  return seen;
}

interface Props {
  project: ProjectSummary;
  selected: boolean;
  playing: boolean;
  nested: boolean;
  onPlay: () => void;
  /** Gestionnaires communs avec la vue en liste (clic, clic droit, glisser…). */
  rowProps: React.HTMLAttributes<HTMLDivElement> & { "data-id": number };
}

/** Carte d'un projet : mini forme d'onde, statut, BPM, temps passé, dernière ouverture. */
export default function ProjectCard({ project: p, selected, playing, nested, onPlay, rowProps }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const seen = useSeen(ref);
  const [preview, setPreview] = useState<PreviewInfo | null>(null);

  useEffect(() => {
    setPreview(null);
    if (!seen || !p.preview_hash) return;
    let alive = true;
    loadPreview(p.preview_hash).then((info) => alive && setPreview(info));
    return () => {
      alive = false;
    };
  }, [seen, p.preview_hash]);

  return (
    <div
      ref={ref}
      {...rowProps}
      className={`ecard ${selected ? "selected" : ""} ${nested ? "nested" : ""} ${p.hidden ? "hidden-item" : ""} ${rowProps.className ?? ""}`}
    >
      <div className="ecard-head">
        <span className={`sdot ${p.status}`} title={STATUS_LABELS[p.status]} />
        <span className="ecard-name" title={p.name}>
          {p.favorite && <span className="fav">★ </span>}
          {p.name}
        </span>
      </div>
      <div className="ecard-wave">
        <button
          className={`icon listen ${playing ? "active" : ""}`}
          disabled={!p.preview_hash}
          title={p.preview_hash ? "Écouter l'aperçu" : "Pas encore d'aperçu"}
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            onPlay();
          }}
        >
          ▶
        </button>
        {preview ? (
          <Waveform
            peaks={preview.peaks}
            duration={preview.duration}
            position={playing ? preview.duration : 0}
            height={26}
          />
        ) : (
          <div className={`ecard-nowave ${p.preview_hash ? "loading" : ""}`}>
            {p.preview_hash ? "" : "pas d'aperçu"}
          </div>
        )}
      </div>
      <small className="ecard-meta">
        <span>{STATUS_LABELS[p.status]}</span>
        {p.tempo ? <span>{p.tempo} BPM</span> : null}
        {p.time_spent > 0 && <span title="Temps passé dans Live">⏱ {formatDuration(p.time_spent)}</span>}
      </small>
      <small className="ecard-meta" title="Dernière session dans Live">
        {p.last_worked_at ? `Ouvert ${formatAgo(p.last_worked_at)}` : "Jamais ouvert"}
      </small>
    </div>
  );
}
