import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { api, CaptureStatus, ProjectDetail, RenderDone, RenderState } from "../api";
import PreviewPlayer from "./PreviewPlayer";

interface Props {
  project: ProjectDetail;
  /** Appelé quand l'aperçu du projet change. */
  onChange: () => void;
}

/** Aperçu du projet (onglet Infos) : lecteur, rendu complet, import d'un bounce. */
export default function PreviewSection({ project, onChange }: Props) {
  const [status, setStatus] = useState<CaptureStatus | null>(null);
  const [render, setRender] = useState<RenderState | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    setMessage(null);
    const poll = () => api.captureStatus().then(setStatus, () => setStatus(null));
    poll();
    const timer = setInterval(poll, 3000);
    const unlisten = [
      listen<{ project_id: number }>("preview-changed", (e) => {
        if (e.payload.project_id === project.id) onChange();
      }),
      listen<RenderState>("render-progress", (e) => {
        if (e.payload.project_id === project.id) setRender(e.payload);
      }),
      listen<RenderDone>("render-done", (e) => {
        if (e.payload.project_id !== project.id) return;
        setRender(null);
        setBusy(null);
        setMessage(e.payload.error ?? "Rendu terminé : l'aperçu couvre tout le morceau.");
      }),
    ];
    return () => {
      clearInterval(timer);
      unlisten.forEach((u) => u.then((f) => f()));
    };
  }, [project.id]);

  async function renderNow() {
    setMessage(null);
    setBusy("Ouverture du set dans Live…");
    setRender({ project_id: project.id, beats: 0, end: 0 });
    try {
      await api.renderPreview(project.id);
      setBusy("Rendu en cours : Live joue le morceau du début à la fin.");
    } catch (e) {
      setRender(null);
      setBusy(null);
      setMessage(String(e));
    }
  }

  async function importFile(path: string) {
    setBusy("Import…");
    setMessage(null);
    try {
      await api.importPreview(project.id, path);
      onChange();
      setMessage(`Aperçu repris de ${path.split(/[\\/]/).pop()}`);
    } catch (e) {
      setMessage(`Erreur : ${e}`);
    } finally {
      setBusy(null);
    }
  }

  async function findBounce() {
    const path = await api.findBounce(project.id);
    if (!path) {
      setMessage("Aucun fichier audio dans le dossier du projet. Choisis-en un.");
      return;
    }
    const ok = await ask(`Prendre ${path} comme aperçu du projet ?`, {
      title: "Bounce trouvé",
      okLabel: "Prendre",
      cancelLabel: "Annuler",
    });
    if (ok) importFile(path);
  }

  async function chooseFile() {
    const path = await open({
      title: "Aperçu du projet",
      defaultPath: project.folder,
      filters: [{ name: "Audio", extensions: ["wav", "aif", "aiff", "mp3", "flac", "ogg", "m4a"] }],
    });
    if (typeof path === "string") importFile(path);
  }

  const rendering = render !== null;
  return (
    <section className="preview">
      {project.preview_hash ? (
        <PreviewPlayer
          hash={project.preview_hash}
          tempo={project.tempo}
          timeSig={project.time_sig}
        />
      ) : (
        <p className="empty">
          Pas encore d'aperçu. Joue le morceau dans Live avec le device de capture sur le master,
          lance un rendu complet ou reprends un bounce.
        </p>
      )}

      {rendering && render.end > 0 && (
        <div className="progress">
          <progress value={render.beats} max={render.end} />
        </div>
      )}

      <div className="actions wrap">
        {rendering ? (
          <button onClick={() => api.cancelRender()}>Arrêter le rendu</button>
        ) : (
          <button
            disabled={!!busy || !status?.device}
            onClick={renderNow}
            title={
              status?.device
                ? "Live joue le morceau du début à la fin et l'enregistre (prend la durée du morceau)"
                : "Le device de capture n'est pas détecté dans Live"
            }
          >
            Rendu complet
          </button>
        )}
        <button disabled={!!busy} onClick={findBounce}>
          Reprendre un bounce
        </button>
        <button disabled={!!busy} onClick={chooseFile}>
          Choisir un fichier…
        </button>
      </div>
      {busy && <p className="message">{busy}</p>}
      {message && <p className="message">{message}</p>}
    </section>
  );
}
