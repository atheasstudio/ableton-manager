import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ask, open } from "@tauri-apps/plugin-dialog";
import {
  api,
  Filter,
  formatSize,
  ProjectSummary,
  Root,
  ScanProgress,
  STATUS_LABELS,
  Status,
  VaultInfo,
} from "./api";
import ProjectList from "./components/ProjectList";
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
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

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
        <ProjectList projects={projects} selected={selected} onSelect={setSelected} />
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
