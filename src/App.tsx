import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, CaptureStatus, Filter, ProjectSummary, Root, ScanProgress, VaultInfo } from "./api";
import Explorer from "./components/Explorer";
import PreviewPlayer from "./components/PreviewPlayer";
import SettingsDialog, { DEFAULT_ROOT } from "./components/SettingsDialog";
import Toasts, { toast } from "./components/Toasts";
import Workspace from "./components/Workspace";
import "./ui.css";

const SELECTED_KEY = "app.selected";

function savedSelection(): number | null {
  try {
    const v = Number(localStorage.getItem(SELECTED_KEY));
    return v > 0 ? v : null;
  } catch {
    return null;
  }
}

export default function App() {
  const [roots, setRoots] = useState<Root[]>([]);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [tags, setTags] = useState<string[]>([]);
  const [filter, setFilter] = useState<Filter>({});
  const [selected, setSelectedState] = useState<number | null>(savedSelection);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [scanning, setScanning] = useState(false);
  const [vault, setVault] = useState<VaultInfo | null>(null);
  const [capture, setCapture] = useState<CaptureStatus | null>(null);
  const [playing, setPlaying] = useState<ProjectSummary | null>(null);
  const [settings, setSettings] = useState(false);

  function setSelected(id: number) {
    setSelectedState(id);
    try {
      localStorage.setItem(SELECTED_KEY, String(id));
    } catch {
      // Sélection non gardée : sans gravité.
    }
  }

  const refreshVault = useCallback(() => {
    api.vaultInfo().then(setVault, () => setVault(null));
  }, []);
  const refreshRoots = useCallback(() => {
    api.listRoots().then(setRoots);
  }, []);

  const refresh = useCallback(async () => {
    setProjects(await api.searchProjects(filter));
    setTags(await api.allTags());
  }, [filter]);

  useEffect(() => {
    refreshRoots();
    refreshVault();
    const unlisten = listen<ScanProgress>("scan-progress", (e) => setProgress(e.payload));
    // Suit le set ouvert dans Live : quand il change, on le sélectionne.
    let lastOpen: number | null = null;
    const pollCapture = () => {
      api.captureStatus().then(setCapture, () => setCapture(null));
      api.openProjectId().then((id) => {
        if (id !== null && id !== lastOpen) setSelected(id);
        lastOpen = id;
      }, () => {});
    };
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

  // Rien de choisi (premier lancement) ou projet disparu : le plus récent.
  useEffect(() => {
    const unfiltered = !filter.query && !filter.status && !filter.tag && !filter.favorites_only && !filter.show_hidden;
    const gone = unfiltered && !projects.some((p) => p.id === selected);
    if (projects.length > 0 && (selected === null || gone)) setSelected(projects[0].id);
  }, [projects]);

  async function scan() {
    setScanning(true);
    try {
      const r = await api.scan();
      const parts = [`${r.found} projets trouvés`, `${r.updated} mis à jour`];
      if (r.removed) parts.push(`${r.removed} disparus`);
      if (r.errors.length) parts.push(`${r.errors.length} illisibles`);
      toast(parts.join(" · "));
      if (r.errors.length) console.warn("Fichiers illisibles", r.errors);
      await refresh();
    } catch (e) {
      toast(`Erreur de scan : ${e}`);
    } finally {
      setScanning(false);
      setProgress(null);
    }
  }

  const captureState = !capture?.listening
    ? "off"
    : capture.device
      ? capture.recording
        ? "rec"
        : "on"
      : "off";

  return (
    <div className="app">
      <aside className="explorer">
        <header className="explorer-head">
          <h1>Ableton Manager</h1>
          <button
            onClick={scan}
            disabled={scanning || roots.length === 0}
            title="Rechercher les projets nouveaux ou modifiés"
          >
            {scanning ? "Scan…" : "Scanner"}
          </button>
          <button
            onClick={() =>
              api.liveAddCapture().then(
                () => toast("Device de capture ajouté au Master du set ouvert"),
                (e) => toast(`Capture impossible : ${e}`),
              )
            }
            title="Ajouter « Ableton Manager Capture » au set ouvert dans Live"
          >
            + Capture
          </button>
          <button
            className="icon settings-btn"
            onClick={() => setSettings(true)}
            title="Réglages : dossiers, coffre, pré-écoute"
            aria-label="Réglages"
          >
            ⚙<span className={`dot ${captureState}`} />
          </button>
        </header>
        {progress && (
          <div className="progress">
            <progress value={progress.done} max={progress.total} />
            <small>{progress.current}</small>
          </div>
        )}

        <Explorer
          projects={projects}
          tags={tags}
          filter={filter}
          onFilter={setFilter}
          selected={selected}
          onSelect={setSelected}
          playing={playing?.id ?? null}
          onPlay={(p) => setPlaying(playing?.id === p.id ? null : p)}
          onChanged={refresh}
          onMessage={(m) => toast(m)}
        />

        {playing?.preview_hash && (
          <div className="now-playing">
            <div className="now-playing-head">
              <strong title={playing.folder}>{playing.name}</strong>
              <button
                className="icon"
                onClick={() => setPlaying(null)}
                aria-label="Fermer le lecteur"
              >
                ×
              </button>
            </div>
            <PreviewPlayer
              key={playing.preview_hash}
              hash={playing.preview_hash}
              tempo={playing.tempo}
              autoPlay
              compact
            />
          </div>
        )}
      </aside>

      {selected !== null ? (
        <Workspace id={selected} onChange={refresh} onVaultChange={refreshVault} />
      ) : (
        <main className="workspace welcome">
          <h2>Bienvenue</h2>
          {roots.length === 0 ? (
            <>
              <p>Indique où sont tes projets Ableton, puis lance un scan.</p>
              <div className="actions">
                <button
                  className="primary"
                  onClick={() => api.addRoot(DEFAULT_ROOT).then(refreshRoots)}
                >
                  Utiliser {DEFAULT_ROOT}
                </button>
                <button onClick={() => setSettings(true)}>Choisir un autre dossier…</button>
              </div>
            </>
          ) : (
            <>
              <p>Choisis un projet à gauche, ou lance un scan pour trouver tes projets.</p>
              <div className="actions">
                <button className="primary" onClick={scan} disabled={scanning}>
                  {scanning ? "Scan en cours…" : "Scanner"}
                </button>
              </div>
            </>
          )}
        </main>
      )}

      <Toasts />

      {settings && (
        <SettingsDialog
          roots={roots}
          onRootsChange={refreshRoots}
          scanning={scanning}
          onScan={scan}
          vault={vault}
          onVaultChange={refreshVault}
          capture={capture}
          onClose={() => setSettings(false)}
        />
      )}
    </div>
  );
}
