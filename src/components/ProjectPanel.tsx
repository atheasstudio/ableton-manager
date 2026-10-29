import { useEffect, useState } from "react";
import { api, formatDate, ProjectDetail, STATUS_LABELS, Status } from "../api";

interface Props {
  id: number;
  onClose: () => void;
  /** Appelé après une modification, pour rafraîchir la liste. */
  onChange: () => void;
}

const KIND_LABELS = { audio: "Audio", midi: "MIDI", group: "Groupe", return: "Retour" };

export default function ProjectPanel({ id, onClose, onChange }: Props) {
  const [project, setProject] = useState<ProjectDetail | null>(null);
  const [note, setNote] = useState("");
  const [newTag, setNewTag] = useState("");

  async function load() {
    const p = await api.getProject(id);
    setProject(p);
    setNote(p?.note ?? "");
  }

  useEffect(() => {
    load();
  }, [id]);

  if (!project) return null;

  async function update(action: Promise<void>) {
    await action;
    await load();
    onChange();
  }

  function addTag(e: React.FormEvent) {
    e.preventDefault();
    if (!newTag.trim()) return;
    update(api.addTag(id, newTag));
    setNewTag("");
  }

  const missing = project.samples.filter((s) => s.missing).length;

  return (
    <aside className="panel">
      <header>
        <button
          className="icon star"
          onClick={() => update(api.setFavorite(id, !project.favorite))}
          aria-label="Favori"
        >
          {project.favorite ? "★" : "☆"}
        </button>
        <h2>{project.name}</h2>
        <button className="icon" onClick={onClose} aria-label="Fermer">
          ×
        </button>
      </header>

      <div className="actions">
        <button className="primary" onClick={() => api.openInAbleton(id)}>
          Ouvrir dans Ableton
        </button>
        <button onClick={() => api.openFolder(id)}>Ouvrir le dossier</button>
      </div>

      <dl className="facts">
        <dt>Tempo</dt>
        <dd>{project.tempo ? `${project.tempo} BPM` : "–"}</dd>
        <dt>Signature</dt>
        <dd>{project.time_sig ?? "–"}</dd>
        <dt>Live</dt>
        <dd>{project.live_version ?? "–"}</dd>
        <dt>Modifié</dt>
        <dd>{formatDate(project.modified_at)}</dd>
      </dl>
      <p className="path" title={project.als_path}>
        {project.als_path}
      </p>

      <label>
        Statut
        <select
          value={project.status}
          onChange={(e) => update(api.setStatus(id, e.target.value as Status))}
        >
          {Object.entries(STATUS_LABELS).map(([k, label]) => (
            <option key={k} value={k}>
              {label}
            </option>
          ))}
        </select>
      </label>

      <div className="tags">
        {project.tags.map((t) => (
          <span key={t} className="tag">
            {t}
            <button className="icon" onClick={() => update(api.removeTag(id, t))}>
              ×
            </button>
          </span>
        ))}
        <form onSubmit={addTag}>
          <input placeholder="+ tag" value={newTag} onChange={(e) => setNewTag(e.target.value)} />
        </form>
      </div>

      <label>
        Note
        <textarea
          rows={4}
          value={note}
          onChange={(e) => setNote(e.target.value)}
          onBlur={() => note !== project.note && update(api.setNote(id, note))}
          placeholder="Idées, choses à finir…"
        />
      </label>

      <details open>
        <summary>Pistes ({project.tracks.length})</summary>
        <ul>
          {project.tracks.map((t, i) => (
            <li key={i}>
              <span className={`kind ${t.kind}`}>{KIND_LABELS[t.kind]}</span> {t.name}
            </li>
          ))}
        </ul>
      </details>

      <details open>
        <summary>Plugins ({project.plugins.length})</summary>
        <ul>
          {project.plugins.map((p) => (
            <li key={p.name}>{p.name}</li>
          ))}
        </ul>
      </details>

      <details>
        <summary>
          Samples ({project.samples.length}
          {missing > 0 && <span className="missing"> · {missing} introuvables</span>})
        </summary>
        <ul>
          {project.samples.map((s, i) => (
            <li key={i} className={s.missing ? "missing" : ""} title={s.path ?? ""}>
              {s.name}
            </li>
          ))}
        </ul>
      </details>

      {project.other_sets.length > 0 && (
        <details>
          <summary>Autres sets du dossier ({project.other_sets.length})</summary>
          <ul>
            {project.other_sets.map((s) => (
              <li key={s.path ?? s.name}>{s.name}</li>
            ))}
          </ul>
        </details>
      )}
    </aside>
  );
}
