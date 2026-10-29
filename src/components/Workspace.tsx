import { useEffect, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { api, formatDate, ProjectDetail, STATUS_LABELS, Status } from "../api";
import { baseName } from "./Explorer";
import PreviewSection from "./PreviewSection";
import TimelineTab from "./TimelineTab";
import VersionsTab from "./VersionsTab";

interface Props {
  id: number;
  /** Appelé après une modification, pour rafraîchir l'explorateur. */
  onChange: () => void;
  /** Appelé quand le coffre change (versions créées ou supprimées). */
  onVaultChange: () => void;
}

type DockTab = "timeline" | "preview" | "infos";

const KIND_LABELS = { audio: "Audio", midi: "MIDI", group: "Groupe", return: "Retour" };
const DOCK_KEY = "workspace.dock";

function savedDock(): DockTab | null {
  try {
    const v = localStorage.getItem(DOCK_KEY);
    return v === "" ? null : ((v as DockTab | null) ?? "preview");
  } catch {
    return "preview";
  }
}

/**
 * Écran central d'un projet : en-tête, arbre des versions au milieu, et en
 * bas la timeline, l'aperçu et les infos.
 */
export default function Workspace({ id, onChange, onVaultChange }: Props) {
  const [project, setProject] = useState<ProjectDetail | null | undefined>(undefined);
  const [dock, setDockState] = useState<DockTab | null>(savedDock);
  const [note, setNote] = useState("");
  const [newTag, setNewTag] = useState("");
  const [message, setMessage] = useState<string | null>(null);

  function setDock(tab: DockTab | null) {
    setDockState(tab);
    try {
      localStorage.setItem(DOCK_KEY, tab ?? "");
    } catch {
      // Préférence non gardée : sans gravité.
    }
  }

  async function load() {
    const p = await api.getProject(id);
    setProject(p);
    setNote(p?.note ?? "");
  }

  useEffect(() => {
    setMessage(null);
    load();
  }, [id]);

  if (project === null) {
    return (
      <main className="workspace welcome">
        <p>Ce projet n'est plus dans la bibliothèque. Choisis-en un autre à gauche.</p>
      </main>
    );
  }
  if (!project || project.id !== id) return <main className="workspace" />;

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

  async function moveToOwnFolder() {
    const target = `${project!.als_path
      .split(/[\\/]/)
      .pop()!
      .replace(/\.als$/i, "")} Project`;
    const ok = await ask(
      `« ${project!.name} » partage son dossier avec ${project!.other_sets.length} autre(s) set(s).\n\n` +
        `Il va être rangé dans un nouveau dossier « ${target} » à côté, avec une copie des audios ` +
        "du dossier qu'il utilise et ses sauvegardes Backup. Les autres sets ne bougent pas.\n\n" +
        "Ferme-le dans Ableton avant.",
      { title: "Ranger dans son propre dossier", okLabel: "Ranger", cancelLabel: "Annuler" },
    );
    if (!ok) return;
    try {
      const r = await api.moveToOwnFolder(id);
      setMessage(`Rangé dans ${r.folder} (${r.copied} audio(s) copié(s)).`);
      await load();
      onChange();
    } catch (e) {
      setMessage(`Erreur : ${e}`);
    }
  }

  const missing = project.samples.filter((s) => s.missing).length;
  const shared = project.other_sets.length > 0;

  return (
    <main className="workspace">
      <header className="ws-head">
        <button
          className="icon star"
          onClick={() => update(api.setFavorite(id, !project.favorite))}
          aria-label="Favori"
        >
          {project.favorite ? "★" : "☆"}
        </button>
        <div className="ws-title">
          <h2>{project.name}</h2>
          <small title={project.als_path}>
            {shared && `📁 ${baseName(project.folder)} · `}
            {[
              project.tempo && `${project.tempo} BPM`,
              project.time_sig,
              project.live_version && `Live ${project.live_version}`,
              `modifié ${formatDate(project.modified_at)}`,
            ]
              .filter(Boolean)
              .join(" · ")}
          </small>
        </div>
        <select
          className={`status ${project.status}`}
          value={project.status}
          onChange={(e) => update(api.setStatus(id, e.target.value as Status))}
          title="Statut"
        >
          {Object.entries(STATUS_LABELS).map(([k, label]) => (
            <option key={k} value={k}>
              {label}
            </option>
          ))}
        </select>
        <div className="actions">
          <button className="primary" onClick={() => api.openInAbleton(id)}>
            Ouvrir dans Live
          </button>
          <button onClick={() => api.openFolder(id)}>Dossier</button>
          {shared && (
            <button
              onClick={moveToOwnFolder}
              title="Ce set partage son dossier avec d'autres morceaux : lui donner le sien"
            >
              Ranger dans son dossier…
            </button>
          )}
        </div>
      </header>
      {message && <p className="message ws-message">{message}</p>}

      <section className="ws-tree">
        <VersionsTab
          key={id}
          projectId={id}
          timeSig={project.time_sig}
          onVaultChange={onVaultChange}
        />
      </section>

      <section className={`dock ${dock ? "open" : ""}`}>
        <nav className="tabs">
          {(
            [
              ["timeline", "Timeline"],
              ["preview", "Aperçu"],
              ["infos", "Infos"],
            ] as const
          ).map(([k, label]) => (
            <button
              key={k}
              className={dock === k ? "active" : ""}
              onClick={() => setDock(dock === k ? null : k)}
            >
              {label}
            </button>
          ))}
          <span className="spacer" />
          <button onClick={() => setDock(dock ? null : "preview")} title="Replier / déplier">
            {dock ? "▾" : "▴"}
          </button>
        </nav>

        {dock && (
          <div className="dock-body">
            {dock === "timeline" && <TimelineTab projectId={id} />}
            {dock === "preview" && (
              <PreviewSection
                project={project}
                onChange={() => {
                  load();
                  onChange();
                }}
              />
            )}
            {dock === "infos" && (
              <div className="infos">
                <div className="infos-col">
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
                      <input
                        placeholder="+ tag"
                        value={newTag}
                        onChange={(e) => setNewTag(e.target.value)}
                      />
                    </form>
                  </div>
                  <p className="path" title={project.als_path}>
                    {project.als_path}
                  </p>
                </div>
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
                <details open>
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
                {shared && (
                  <details open>
                    <summary>Autres sets du dossier ({project.other_sets.length})</summary>
                    <ul>
                      {project.other_sets.map((s) => (
                        <li key={s.path ?? s.name}>{s.name}</li>
                      ))}
                    </ul>
                  </details>
                )}
              </div>
            )}
          </div>
        )}
      </section>
    </main>
  );
}
