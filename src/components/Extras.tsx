import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { api, BIBLIO_CHANGEE, ExportFile, formatAgo, Todo } from "../api";
import { toast, toastError } from "./Toasts";
import "./pastille-ia.css";

/**
 * Ouvre le projet dans Live, après avoir prévenu s'il manque des samples.
 * Propose alors de les rassembler dans le projet avant d'ouvrir.
 */
export async function openInLive(id: number): Promise<void> {
  try {
    const missing = await api.projectMissingSamples(id).catch(() => [] as string[]);
    if (missing.length > 0) {
      const list = missing.slice(0, 8).join("\n");
      const more = missing.length > 8 ? `\n… et ${missing.length - 8} autres` : "";
      const go = await ask(
        `${missing.length} sample${missing.length > 1 ? "s" : ""} introuvable${missing.length > 1 ? "s" : ""} :\n${list}${more}\n\nLive les affichera en rouge. Ouvrir quand même ?\n(Le Docteur peut les retrouver.)`,
        { title: "Samples manquants", kind: "warning", okLabel: "Ouvrir quand même", cancelLabel: "Annuler" },
      );
      if (!go) return;
    }
    await api.openInAbleton(id);
  } catch (e) {
    toastError("Ouverture impossible")(e);
  }
}

/** Demande le dossier cloud (OneDrive, Google Drive, Dropbox, 2e disque…). */
export async function chooseCloudDir(): Promise<string | null> {
  const dir = await open({
    directory: true,
    title: "Dossier synchronisé (OneDrive, Google Drive, Dropbox…) où copier les projets",
  });
  if (typeof dir !== "string") return null;
  await api.setCloudDir(dir);
  return dir;
}

function formatSize(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} Go`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(0)} Mo`;
  return `${Math.max(1, Math.round(bytes / 1e3))} Ko`;
}

/** Bouton « Cloud » de l'en-tête du projet. */
export function CloudButton({ projectId }: { projectId: number }) {
  const [last, setLast] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.cloudInfo(projectId).then((i) => setLast(i.last_backup), () => setLast(null));
  }, [projectId]);

  async function backup() {
    try {
      let { dir } = await api.cloudInfo();
      if (!dir) dir = await chooseCloudDir();
      if (!dir) return;
      setBusy(true);
      const r = await api.cloudBackup(projectId);
      setLast(Date.now() / 1000);
      toast(
        r.copied === 0
          ? "Cloud : déjà à jour"
          : `Cloud : ${r.copied} fichier${r.copied > 1 ? "s" : ""} envoyé${r.copied > 1 ? "s" : ""} (${formatSize(r.bytes)})`,
      );
    } catch (e) {
      toastError("Sauvegarde cloud impossible")(e);
    } finally {
      setBusy(false);
    }
  }

  return (
    <button
      onClick={backup}
      disabled={busy}
      title={
        (last ? `Dernière sauvegarde ${formatAgo(last)}. ` : "Jamais sauvegardé. ") +
        "Copie le projet et ses samples dans ton dossier cloud (seulement ce qui a changé)."
      }
    >
      {busy ? "Envoi…" : last ? "☁ Cloud ✓" : "☁ Cloud"}
    </button>
  );
}

/** Onglet « Exports » : fichiers audio exportés dans le dossier du projet. */
export function ExportsTab({ projectId, onPreview }: { projectId: number; onPreview: () => void }) {
  const [files, setFiles] = useState<ExportFile[] | null>(null);

  useEffect(() => {
    setFiles(null);
    api.listExports(projectId).then(setFiles, () => setFiles([]));
  }, [projectId]);

  if (!files) return <p className="muted">Recherche des exports…</p>;
  if (files.length === 0)
    return <p className="muted">Aucun export audio dans le dossier du projet. Exporte depuis Live (Ctrl+Maj+R) et il apparaîtra ici.</p>;

  return (
    <ul className="exports-list">
      {files.map((f) => (
        <li key={f.path}>
          <span className="name" title={f.path}>
            {f.name}
          </span>
          <span className="muted">
            {formatSize(f.size)} · {formatAgo(f.modified)}
          </span>
          <button onClick={() => api.openExport(projectId, f.path).catch(toastError("Lecture impossible"))} title="Écouter dans le lecteur de Windows">
            ▶
          </button>
          <button
            onClick={() =>
              api.importPreview(projectId, f.path).then(() => {
                toast("Aperçu mis à jour");
                onPreview();
              }, toastError("Import impossible"))
            }
            title="Utiliser ce fichier comme aperçu du projet"
          >
            Aperçu
          </button>
        </li>
      ))}
    </ul>
  );
}

/** Onglet « Tâches » : à faire du projet, ou de toute la bibliothèque. */
export function TodosTab({ projectId, onOpenProject }: { projectId: number; onOpenProject?: (id: number) => void }) {
  const [all, setAll] = useState(false);
  const [todos, setTodos] = useState<Todo[]>([]);
  const [text, setText] = useState("");

  function load() {
    api.listTodos(all ? undefined : projectId).then(setTodos, toastError("Tâches illisibles"));
  }
  useEffect(load, [projectId, all]);
  // Tâche ajoutée ou cochée par l'IA : la liste suit.
  useEffect(() => {
    const un = listen(BIBLIO_CHANGEE, load);
    return () => {
      un.then((f) => f());
    };
  }, [projectId, all]);

  async function add() {
    if (!text.trim()) return;
    await api.addTodo(projectId, text).catch(toastError("Ajout impossible"));
    setText("");
    load();
  }

  return (
    <div className="todos">
      <div className="todos-bar">
        <input
          value={text}
          placeholder="Nouvelle tâche (ex. refaire le mix du kick)"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
        />
        <button onClick={add}>Ajouter</button>
        <label className="muted">
          <input type="checkbox" checked={all} onChange={(e) => setAll(e.target.checked)} /> Tous les projets
        </label>
      </div>
      {todos.length === 0 && <p className="muted">{all ? "Rien à faire nulle part 🎉" : "Aucune tâche pour ce projet."}</p>}
      <ul>
        {todos.map((t) => (
          <li key={t.id} className={t.done ? "done" : ""}>
            <label>
              <input
                type="checkbox"
                checked={t.done}
                onChange={(e) => api.setTodoDone(t.id, e.target.checked).then(load, toastError("Erreur"))}
              />{" "}
              {t.text}
            </label>
            {t.source === "ia" && (
              <span className="at-pastille-ia" title="Tâche ajoutée par l'IA">
                par l'IA
              </span>
            )}
            {all && t.project_id !== projectId && (
              <button className="link" onClick={() => onOpenProject?.(t.project_id)}>
                {t.project_name}
              </button>
            )}
            <button onClick={() => api.deleteTodo(t.id).then(load, toastError("Erreur"))} title="Supprimer">
              ✕
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
