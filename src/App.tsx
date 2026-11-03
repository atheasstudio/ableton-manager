import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, CaptureStatus, Filter, ProjectSummary, Root, ScanProgress, VaultInfo } from "./api";
import Explorer from "./components/Explorer";
import CommandPalette, { PaletteAction, ShortcutsHelp } from "./components/CommandPalette";
import PreviewPlayer, { PlayerHandle } from "./components/PreviewPlayer";
import SettingsDialog, { DEFAULT_ROOT } from "./components/SettingsDialog";
import Onboarding, { ONBOARDED_KEY } from "./components/Onboarding";
import Toasts, { friendlyError, toast, toastError, undoLast } from "./components/Toasts";
import Workspace from "./components/Workspace";
import { emit, useShortcuts, withKey } from "./shortcuts";
import "./ui.css";

const SELECTED_KEY = "app.selected";
const FILTER_KEY = "app.filter";
const LAYOUT_KEY = "app.layout";
const LAST_OPENED_KEY = "app.lastOpened";

/** Disposition gardée entre les sessions. */
interface Layout {
  sidebar: boolean;
  focus: boolean;
  /** Taille du texte en px (densité). */
  fontSize: number;
}

function load<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return v ? { ...fallback, ...JSON.parse(v) } : fallback;
  } catch {
    return fallback;
  }
}

function save(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Non gardé : sans gravité.
  }
}

/** Dernier projet ouvert dans Live depuis l'app. */
function lastOpened(): number | null {
  try {
    const v = Number(localStorage.getItem(LAST_OPENED_KEY));
    return v > 0 ? v : null;
  } catch {
    return null;
  }
}

/** Dossier qui contient un fichier. */
function parentDir(path: string): string {
  return path.replace(/[\\/][^\\/]+$/, "");
}

const AUDIO_EXT = ["wav", "aif", "aiff", "mp3", "flac", "ogg", "m4a"];

/** Extension (minuscules) du dernier élément d'un chemin, "" s'il n'en a pas. */
function extension(path: string): string {
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? "";
  const m = /\.([^.\s]{1,5})$/.exec(name);
  // « v1.2 » ou « Song Project » : des dossiers, pas des extensions.
  return m && !/^\d+$/.test(m[1]) ? m[1].toLowerCase() : "";
}

/** Fichiers et dossiers glissés sur la fenêtre, triés. */
function sortDrop(paths: string[]) {
  const audio = paths.filter((p) => AUDIO_EXT.includes(extension(p)));
  const folders = paths.filter((p) => extension(p) === "");
  const sets = paths.filter((p) => extension(p) === "als");
  // Un .als déposé : on ajoute le dossier du projet qui le contient.
  for (const s of sets) {
    const dir = parentDir(s);
    if (!folders.includes(dir)) folders.push(dir);
  }
  return { audio, folders, sets };
}

/** Chemin Windows comparable : sans casse, séparateurs unifiés, sans séparateur final. */
function normPath(p: string): string {
  return p.replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
}

/** Le dossier est-il déjà une racine, ou dans une racine ? */
function covered(path: string, root: string): boolean {
  const a = normPath(path);
  const b = normPath(root);
  return a === b || a.startsWith(b + "\\");
}

interface DropHint {
  text: string;
  ok: boolean;
}

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
  const [filter, setFilter] = useState<Filter>(() => load<Filter>(FILTER_KEY, {}));
  const [layout, setLayout] = useState<Layout>(() => load<Layout>(LAYOUT_KEY, { sidebar: true, focus: false, fontSize: 14 }));
  const [selected, setSelectedState] = useState<number | null>(savedSelection);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [scanning, setScanning] = useState(false);
  const [vault, setVault] = useState<VaultInfo | null>(null);
  const [capture, setCapture] = useState<CaptureStatus | null>(null);
  const [playing, setPlaying] = useState<ProjectSummary | null>(null);
  const [settings, setSettings] = useState(false);
  const [onboarding, setOnboarding] = useState(() => {
    try {
      return !localStorage.getItem(ONBOARDED_KEY);
    } catch {
      return false;
    }
  });
  /** Change pour recharger la fiche du projet (aperçu importé par glisser-déposer). */
  const [workspaceKey, setWorkspaceKey] = useState(0);
  const [palette, setPalette] = useState(false);
  const [help, setHelp] = useState(false);

  // Écoute enchaînée : quand la sélection change pendant une lecture, le lecteur
  // passe au projet suivant et reprend au même endroit relatif.
  /** Fraction de la durée où en est la lecture. */
  const playRatio = useRef(0);
  /** Le lecteur joue (ou va rejouer : changement de projet en cours). */
  const audioActive = useRef(false);
  const switching = useRef(false);
  const [playStart, setPlayStart] = useState<number | undefined>(undefined);
  const player = useRef<PlayerHandle | null>(null);
  const [dropHint, setDropHint] = useState<DropHint | null>(null);

  function play(p: ProjectSummary | null, ratio?: number) {
    switching.current = p !== null && ratio !== undefined;
    if (p === null) audioActive.current = false;
    setPlayStart(ratio);
    setPlaying(p);
  }

  function setSelected(id: number) {
    setSelectedState(id);
    try {
      localStorage.setItem(SELECTED_KEY, String(id));
    } catch {
      // Sélection non gardée : sans gravité.
    }
  }

  useEffect(() => save(FILTER_KEY, filter), [filter]);
  useEffect(() => {
    save(LAYOUT_KEY, layout);
    document.documentElement.style.fontSize = `${layout.fontSize}px`;
  }, [layout]);

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

  useEffect(() => {
    if (!audioActive.current || !playing || selected === null || playing.id === selected) return;
    const next = projects.find((p) => p.id === selected);
    // Sans aperçu, on ne coupe pas : le projet précédent continue.
    if (next?.preview_hash && next.preview_hash !== playing.preview_hash) play(next, playRatio.current);
  }, [selected]);

  /** Projet sélectionné, même s'il n'est pas dans la liste filtrée. */
  async function selectedProject(): Promise<ProjectSummary | null> {
    if (selected === null) return null;
    return projects.find((p) => p.id === selected) ?? (await api.getProject(selected));
  }

  async function togglePlay() {
    if (playing && playing.id === selected && player.current) {
      player.current.toggle();
      return;
    }
    const p = await selectedProject();
    if (!p) return;
    if (!p.preview_hash) {
      toast(`« ${p.name} » n'a pas encore d'aperçu`);
      return;
    }
    play(p);
  }

  function openInLive(id = selected) {
    if (id === null) return;
    save(LAST_OPENED_KEY, id);
    api.openInAbleton(id).catch(toastError("Ouverture impossible"));
  }

  function openFolder() {
    if (selected !== null) api.openFolder(selected).catch(toastError("Dossier introuvable"));
  }

  // Glisser-déposer depuis l'explorateur Windows (évènements natifs de Tauri :
  // le glisser-déposer HTML de la page est désactivé tant que dragDropEnabled est vrai).
  /** Projet visé : la ligne de l'explorateur sous le curseur, sinon le projet ouvert. */
  function dropTarget(x: number, y: number): number | null {
    const el = document.elementFromPoint(x / devicePixelRatio, y / devicePixelRatio);
    const row = el?.closest<HTMLElement>(".explorer [data-id]");
    const id = row ? Number(row.dataset.id) : NaN;
    return id > 0 ? id : selected;
  }

  function projectName(id: number | null): string {
    return projects.find((p) => p.id === id)?.name ?? "le projet ouvert";
  }

  function describeDrop(paths: string[], target: number | null): DropHint {
    const { audio, folders } = sortDrop(paths);
    if (folders.length) {
      return {
        ok: true,
        text:
          folders.length > 1
            ? `Ajouter ces ${folders.length} dossiers à la bibliothèque et les scanner`
            : "Ajouter ce dossier à la bibliothèque et le scanner",
      };
    }
    if (audio.length) {
      return target === null
        ? { ok: false, text: "Choisis d'abord un projet : le fichier deviendra son aperçu" }
        : { ok: true, text: `Faire de ce fichier l'aperçu de « ${projectName(target)} »` };
    }
    return { ok: false, text: "Dépose un dossier de projets ou un fichier audio" };
  }

  async function onDrop(paths: string[], target: number | null) {
    const { audio, folders } = sortDrop(paths);
    if (folders.length) {
      let added = 0;
      for (const f of folders) {
        if (roots.some((r) => covered(f, r.path))) continue;
        try {
          await api.addRoot(f);
          added++;
        } catch (e) {
          toast(`Impossible d'ajouter ${f} : ${friendlyError(e)}`);
        }
      }
      refreshRoots();
      if (added) toast(added > 1 ? `${added} dossiers ajoutés, scan en cours…` : "Dossier ajouté, scan en cours…");
      if (scanning) toast("Un scan est déjà en cours : relance-le ensuite");
      else await scan();
      return;
    }
    if (audio.length && target !== null) {
      const name = projectName(target);
      try {
        await api.importPreview(target, audio[0]);
        toast(`Nouvel aperçu pour « ${name} »`);
        // L'aperçu du lecteur et de la liste change : on recharge.
        if (playing?.id === target) play(null);
        await refresh();
        setWorkspaceKey((k) => k + 1);
      } catch (e) {
        toast(`Import de l'aperçu impossible : ${friendlyError(e)}`);
      }
    }
  }

  const drop = useRef({ describeDrop, onDrop, dropTarget });
  drop.current = { describeDrop, onDrop, dropTarget };

  useEffect(() => {
    let paths: string[] = [];
    let target: number | null = null;
    let alive = true;
    let unlisten: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent(({ payload: p }) => {
        const d = drop.current;
        if (p.type === "enter") {
          paths = p.paths;
          target = d.dropTarget(p.position.x, p.position.y);
          setDropHint(d.describeDrop(paths, target));
        } else if (p.type === "over") {
          const t = d.dropTarget(p.position.x, p.position.y);
          if (t !== target) {
            target = t;
            setDropHint(d.describeDrop(paths, target));
          }
        } else if (p.type === "drop") {
          setDropHint(null);
          d.onDrop(p.paths, d.dropTarget(p.position.x, p.position.y));
        } else {
          setDropHint(null);
        }
      })
      .then(
        (f) => (alive ? (unlisten = f) : f()),
        () => {},
      );
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  useShortcuts({
    togglePlay,
    palette: () => setPalette((v) => !v),
    openInLive,
    help: () => setHelp(true),
    openFolder,
    undo: () => {
      if (!undoLast()) toast("Rien à annuler");
    },
    toggleSidebar: () => setLayout((l) => ({ ...l, sidebar: !l.sidebar })),
    toggleFocus: () => setLayout((l) => ({ ...l, focus: !l.focus })),
    zoom: (d) => setLayout((l) => ({ ...l, fontSize: Math.min(18, Math.max(12, l.fontSize + d)) })),
  });

  // Icône de la barre des tâches : « Ouvrir le dernier projet ».
  useEffect(() => {
    const un = listen("tray-open-last", () => {
      const id = lastOpened();
      if (id) openRef.current(id);
      else toast("Aucun projet ouvert récemment");
    });
    return () => {
      un.then((f) => f());
    };
  }, []);
  const openRef = useRef(openInLive);
  openRef.current = openInLive;

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
      toast(`Erreur de scan : ${friendlyError(e)}`);
    } finally {
      setScanning(false);
      setProgress(null);
    }
  }

  function paletteActions(): PaletteAction[] {
    const current = projects.find((p) => p.id === selected);
    const hint = current?.name;
    const none = selected === null;
    return [
      { id: "live", label: "Ouvrir dans Live", hint, disabled: none, keywords: "ableton ctrl+o", run: () => openInLive() },
      { id: "folder", label: "Ouvrir le dossier du projet", hint, disabled: none, keywords: "explorateur windows ctrl+e", run: openFolder },
      {
        id: "last",
        label: "Ouvrir le dernier projet dans Live",
        keywords: "récent reprendre",
        run: () => {
          const id = lastOpened();
          if (id) openInLive(id);
          else toast("Aucun projet ouvert récemment");
        },
      },
      { id: "undo", label: "Annuler la dernière action", keywords: "ctrl+z défaire", run: () => undoLast() || toast("Rien à annuler") },
      {
        id: "sidebar",
        label: layout.sidebar ? "Replier la barre latérale" : "Déplier la barre latérale",
        keywords: "ctrl+b panneau",
        run: () => setLayout((l) => ({ ...l, sidebar: !l.sidebar })),
      },
      {
        id: "focus",
        label: layout.focus ? "Quitter le mode focus" : "Mode focus",
        keywords: "plein écran ctrl+shift+f",
        run: () => setLayout((l) => ({ ...l, focus: !l.focus })),
      },
      { id: "bigger", label: "Agrandir le texte", keywords: "police densité zoom ctrl++", run: () => setLayout((l) => ({ ...l, fontSize: Math.min(18, l.fontSize + 1) })) },
      { id: "smaller", label: "Réduire le texte", keywords: "police densité compact ctrl+-", run: () => setLayout((l) => ({ ...l, fontSize: Math.max(12, l.fontSize - 1) })) },
      { id: "resetfilter", label: "Effacer les filtres", keywords: "réinitialiser recherche", run: () => setFilter({}) },
      {
        id: "play",
        label: "Écouter / arrêter l'aperçu",
        hint,
        disabled: none,
        keywords: "lecture espace",
        run: togglePlay,
      },
      {
        id: "render",
        label: "Rendu complet de l'aperçu",
        hint,
        disabled: none,
        keywords: "enregistrer bounce",
        run: () =>
          api.renderPreview(selected!).then(
            () => toast("Rendu complet lancé dans Live"),
            toastError("Rendu impossible"),
          ),
      },
      { id: "scan", label: "Scanner les dossiers", disabled: scanning || roots.length === 0, keywords: "rechercher nouveaux", run: scan },
      { id: "settings", label: "Réglages", keywords: "dossiers coffre capture", run: () => setSettings(true) },
      {
        id: "doctor",
        label: "Santé de la bibliothèque",
        keywords: "docteur samples manquants plugins",
        run: () => emit("am:open-doctor"),
      },
      { id: "search", label: "Rechercher dans la bibliothèque", keywords: "filtre /", run: () => emit("am:focus-search") },
      { id: "help", label: "Raccourcis clavier", keywords: "aide ?", run: () => setHelp(true) },
    ];
  }

  const captureState = !capture?.listening
    ? "off"
    : capture.device
      ? capture.recording
        ? "rec"
        : "on"
      : "off";

  return (
    <div className={`app ${layout.sidebar && !layout.focus ? "" : "sidebar-hidden"} ${layout.focus ? "focus" : ""}`}>
      {(!layout.sidebar || layout.focus) && (
        <button
          className="icon sidebar-toggle"
          onClick={() => setLayout((l) => ({ ...l, sidebar: true, focus: false }))}
          title={withKey("Afficher la barre latérale", "Ctrl+B")}
          aria-label="Afficher la barre latérale"
        >
          ☰
        </button>
      )}
      <aside className="explorer">
        <header className="explorer-head">
          <button
            className="icon"
            onClick={() => setLayout((l) => ({ ...l, sidebar: false }))}
            title={withKey("Replier la barre latérale", "Ctrl+B")}
            aria-label="Replier la barre latérale"
          >
            ‹
          </button>
          <h1>Ableton Manager</h1>
          <button
            onClick={scan}
            disabled={scanning || roots.length === 0}
            title="Rechercher les projets nouveaux ou modifiés (tu peux continuer à travailler pendant le scan)"
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
            className="icon doctor-btn"
            onClick={() => emit("am:open-doctor")}
            title={withKey("Santé de la bibliothèque : samples introuvables, plugins manquants…", "Ctrl+K")}
            aria-label="Santé de la bibliothèque"
          >
            🩺
          </button>
          <button
            className="icon settings-btn"
            onClick={() => setSettings(true)}
            title={withKey("Réglages : dossiers, coffre, pré-écoute", "Ctrl+K")}
            aria-label="Réglages"
          >
            ⚙<span className={`dot ${captureState}`} />
          </button>
        </header>
        {progress && (
          <div className="progress" title={`${progress.done} / ${progress.total}`}>
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
          onPlay={(p) => play(playing?.id === p.id ? null : p)}
          onChanged={refresh}
        />

        {playing?.preview_hash && (
          <div className="now-playing">
            <div className="now-playing-head">
              <strong title={playing.folder}>{playing.name}</strong>
              <button
                className="icon"
                onClick={() => play(null)}
                title="Fermer le lecteur"
                aria-label="Fermer le lecteur"
              >
                ×
              </button>
            </div>
            <PreviewPlayer
              key={playing.preview_hash}
              hash={playing.preview_hash}
              tempo={playing.tempo}
              startRatio={playStart}
              handle={player}
              onTime={(t, d) => {
                if (d > 0) playRatio.current = t / d;
              }}
              onPlayingChange={(on) => {
                if (on) {
                  switching.current = false;
                  audioActive.current = true;
                } else if (!switching.current) {
                  audioActive.current = false;
                }
              }}
              autoPlay
              compact
            />
          </div>
        )}
      </aside>

      {selected !== null ? (
        <Workspace key={workspaceKey} id={selected} onChange={refresh} onVaultChange={refreshVault} />
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

      {dropHint && (
        <div className={`dropzone ${dropHint.ok ? "" : "refused"}`}>
          <p>{dropHint.text}</p>
        </div>
      )}

      <Toasts />

      {palette && (
        <CommandPalette
          actions={paletteActions()}
          onSelectProject={setSelected}
          onClose={() => setPalette(false)}
        />
      )}
      {help && <ShortcutsHelp onClose={() => setHelp(false)} />}

      {onboarding && (
        <Onboarding
          roots={roots}
          onRootsChange={refreshRoots}
          scanning={scanning}
          onScan={scan}
          capture={capture}
          onClose={() => setOnboarding(false)}
        />
      )}

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
