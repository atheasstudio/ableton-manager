import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ask, open } from "@tauri-apps/plugin-dialog";
import {
  api,
  CaptureStatus,
  Filter,
  NO_USER_LIBRARY,
  formatSize,
  ProjectSummary,
  Root,
  ScanProgress,
  STATUS_LABELS,
  Status,
  VaultInfo,
} from "./api";
import ProjectList from "./components/ProjectList";
import PreviewPlayer from "./components/PreviewPlayer";
import ProjectPanel from "./components/ProjectPanel";

/** Dossier proposé au premier lancement. */
const DEFAULT_ROOT = "F:\\musique\\Projets";

export default function App() {
  const [roots, setRoots] = useState<Root[]>([]);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [tags, setTags] = useState<string[]>([]);
  const [filter, setFilter] = useState<Filter>({});
  const [selected, setSelected] = useState<number | null>(null);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [scanning, setScanning] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [vault, setVault] = useState<VaultInfo | null>(null);
  const [movingVault, setMovingVault] = useState(false);
  const [capture, setCapture] = useState<CaptureStatus | null>(null);
  const [deviceMessage, setDeviceMessage] = useState<string | null>(null);
  const [playing, setPlaying] = useState<ProjectSummary | null>(null);

  const refreshVault = useCallback(() => {
    api.vaultInfo().then(setVault, () => setVault(null));
  }, []);

  const refresh = useCallback(async () => {
    setProjects(await api.searchProjects(filter));
    setTags(await api.allTags());
  }, [filter]);

  useEffect(() => {
    api.listRoots().then(setRoots);
    refreshVault();
    const unlisten = listen<ScanProgress>("scan-progress", (e) => setProgress(e.payload));
    const pollCapture = () => api.captureStatus().then(setCapture, () => setCapture(null));
    pollCapture();
    const timer = setInterval(pollCapture, 5000);
    // Un nouvel aperçu change l'icône d'écoute de la liste.
    const unlistenPreview = listen("preview-changed", () => refreshRef.current());
    return () => {
      clearInterval(timer);
      unlisten.then((f) => f());
      unlistenPreview.then((f) => f());
    };
  }, []);

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function installDevice() {
    setDeviceMessage(null);
    try {
      let path: string;
      try {
        path = await api.installDevice();
      } catch (e) {
        if (String(e) !== NO_USER_LIBRARY) throw e;
        const dir = await open({
          directory: true,
          title: "Dossier « User Library » de Live (Réglages > Bibliothèque)",
        });
        if (typeof dir !== "string") return;
        path = await api.installDevice(dir);
      }
      setDeviceMessage(
        `Installé : ${path}. Dans Live, glisse « Ableton Manager Capture » (User Library > Presets > ` +
          "Audio Effects > Max Audio Effect) sur le Master, puis enregistre-le dans ton set par défaut.",
      );
    } catch (e) {
      setDeviceMessage(`Erreur : ${e}`);
    }
  }

  async function scan() {
    setScanning(true);
    setMessage(null);
    try {
      const r = await api.scan();
      const parts = [`${r.found} projets trouvés`, `${r.updated} mis à jour`];
      if (r.removed) parts.push(`${r.removed} disparus`);
      if (r.errors.length) parts.push(`${r.errors.length} illisibles`);
      setMessage(parts.join(" · "));
      if (r.errors.length) console.warn("Fichiers illisibles", r.errors);
      await refresh();
    } catch (e) {
      setMessage(`Erreur : ${e}`);
    } finally {
      setScanning(false);
      setProgress(null);
    }
  }

  async function addRoot() {
    const dir = await open({ directory: true, title: "Dossier de projets Ableton" });
    if (typeof dir !== "string") return;
    await api.addRoot(dir);
    setRoots(await api.listRoots());
  }

  async function addDefaultRoot() {
    await api.addRoot(DEFAULT_ROOT);
    setRoots(await api.listRoots());
  }

  async function moveVault() {
    const dir = await open({ directory: true, title: "Dossier du coffre (idéalement un disque dédié)" });
    if (typeof dir !== "string") return;
    const ok = await ask(`Déplacer le coffre vers ${dir} ? Les fichiers déjà rangés y sont déplacés.`, {
      title: "Déplacer le coffre",
      okLabel: "Déplacer",
      cancelLabel: "Annuler",
    });
    if (!ok) return;
    setMovingVault(true);
    try {
      await api.setVaultDir(dir);
    } catch (e) {
      setMessage(`Erreur : ${e}`);
    } finally {
      setMovingVault(false);
      refreshVault();
    }
  }

  async function removeRoot(id: number) {
    await api.removeRoot(id);
    setRoots(await api.listRoots());
  }

  return (
    <div className="app">
      <aside className="sidebar">
        <h1>Ableton Manager</h1>

        <section>
          <h2>Dossiers scannés</h2>
          {roots.length === 0 && (
            <button className="link" onClick={addDefaultRoot}>
              + {DEFAULT_ROOT}
            </button>
          )}
          <ul className="roots">
            {roots.map((r) => (
              <li key={r.id} title={r.path}>
                <span>{r.path}</span>
                <button className="icon" onClick={() => removeRoot(r.id)} aria-label="Retirer">
                  ×
                </button>
              </li>
            ))}
          </ul>
          <button onClick={addRoot}>Ajouter un dossier ou un disque…</button>
          <button className="primary" onClick={scan} disabled={scanning || roots.length === 0}>
            {scanning ? "Scan en cours…" : "Scanner"}
          </button>
          {progress && (
            <div className="progress">
              <progress value={progress.done} max={progress.total} />
              <small>{progress.current}</small>
            </div>
          )}
          {message && <p className="message">{message}</p>}
        </section>

        <section>
          <h2>Statut</h2>
          <select
            value={filter.status ?? ""}
            onChange={(e) =>
              setFilter({ ...filter, status: (e.target.value || undefined) as Status | undefined })
            }
          >
            <option value="">Tous</option>
            {Object.entries(STATUS_LABELS).map(([k, label]) => (
              <option key={k} value={k}>
                {label}
              </option>
            ))}
          </select>
          <label className="check">
            <input
              type="checkbox"
              checked={!!filter.favorites_only}
              onChange={(e) => setFilter({ ...filter, favorites_only: e.target.checked })}
            />
            Favoris seulement
          </label>
        </section>

        <section>
          <h2>Coffre des versions</h2>
          {vault && (
            <>
              <p className="path" title={vault.path}>
                {vault.path}
              </p>
              <small className="message">
                {vault.files} fichiers · {formatSize(vault.bytes)}
              </small>
            </>
          )}
          <button onClick={moveVault} disabled={movingVault}>
            {movingVault ? "Déplacement…" : "Changer de dossier…"}
          </button>
        </section>

        <section>
          <h2>Pré-écoute</h2>
          <small className="message">
            {!capture?.listening
              ? "Port de capture occupé : une autre instance de l'appli tourne ?"
              : capture.device
                ? capture.recording
                  ? "● Écoute en cours d'enregistrement"
                  : "Device de capture connecté"
                : "Device de capture non détecté dans Live"}
          </small>
          {capture?.open_set && (
            <small className="message" title="Set ouvert dans Live">
              Live : {capture.open_set}
            </small>
          )}
          <button onClick={installDevice}>Installer le device…</button>
          {deviceMessage && <p className="message">{deviceMessage}</p>}
        </section>

        {tags.length > 0 && (
          <section>
            <h2>Tags</h2>
            <div className="tags">
              {tags.map((t) => (
                <button
                  key={t}
                  className={`tag ${filter.tag === t ? "active" : ""}`}
                  onClick={() => setFilter({ ...filter, tag: filter.tag === t ? undefined : t })}
                >
                  {t}
                </button>
              ))}
            </div>
          </section>
        )}
      </aside>

      <main className="main">
        <input
          className="search"
          type="search"
          placeholder="Chercher un projet, un plugin, un sample, un tag…"
          value={filter.query ?? ""}
          onChange={(e) => setFilter({ ...filter, query: e.target.value })}
          autoFocus
        />
        <ProjectList
          projects={projects}
          selected={selected}
          onSelect={setSelected}
          playing={playing?.id ?? null}
          onPlay={(p) => setPlaying(p)}
        />
        {playing?.preview_hash && (
          <div className="now-playing">
            <strong title={playing.folder}>{playing.name}</strong>
            <PreviewPlayer
              key={playing.preview_hash}
              hash={playing.preview_hash}
              tempo={playing.tempo}
              autoPlay
              compact
            />
            <button className="icon" onClick={() => setPlaying(null)} aria-label="Fermer le lecteur">
              ×
            </button>
          </div>
        )}
      </main>

      {selected !== null && (
        <ProjectPanel
          id={selected}
          onClose={() => setSelected(null)}
          onChange={refresh}
          onVaultChange={refreshVault}
        />
      )}
    </div>
  );
}
