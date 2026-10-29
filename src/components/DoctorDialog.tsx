import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  api,
  DoctorProgress,
  MissingProject,
  PluginUse,
  ProjectSummary,
  Relink,
} from "../api";
import "../doctor.css";

/** Notification affichée par le composant Toasts (lot interface). */
function toast(message: string) {
  window.dispatchEvent(new CustomEvent("am:toast", { detail: { message } }));
}

type Tab = "samples" | "plugins";

/**
 * Santé de la bibliothèque : samples introuvables (avec réparation) et
 * inventaire des plugins. Montée à côté de l'appli, s'ouvre sur
 * l'évènement DOM `am:open-doctor`.
 */
export default function DoctorDialog() {
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<Tab>("samples");

  useEffect(() => {
    const onOpen = () => setOpen(true);
    window.addEventListener("am:open-doctor", onOpen);
    return () => window.removeEventListener("am:open-doctor", onOpen);
  }, []);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  if (!open) return null;
  const close = () => setOpen(false);
  return (
    <div className="overlay" onClick={close}>
      <div className="dialog doctor" onClick={(e) => e.stopPropagation()}>
        <header>
          <h2>Santé de la bibliothèque</h2>
          <button className="icon" onClick={close} aria-label="Fermer">
            ×
          </button>
        </header>
        <nav className="tabs">
          <button className={tab === "samples" ? "active" : ""} onClick={() => setTab("samples")}>
            Samples manquants
          </button>
          <button className={tab === "plugins" ? "active" : ""} onClick={() => setTab("plugins")}>
            Plugins
          </button>
        </nav>
        {tab === "samples" ? <MissingSamples /> : <Plugins onClose={close} />}
      </div>
    </div>
  );
}

function progressText(p: DoctorProgress | null): string | null {
  if (!p) return null;
  if (p.step === "samples") return `Vérification des projets… ${p.done} / ${p.total}`;
  return `Recherche dans les dossiers scannés… ${p.done.toLocaleString("fr-FR")} fichiers`;
}

/** Clé d'un sample dans les choix : projet + chemin noté dans le set. */
const key = (projectId: number, path: string) => `${projectId}\u0000${path}`;

function MissingSamples() {
  const [projects, setProjects] = useState<MissingProject[] | null>(null);
  const [busy, setBusy] = useState<"analyse" | "search" | number | null>(null);
  const [progress, setProgress] = useState<DoctorProgress | null>(null);
  const [candidates, setCandidates] = useState<Record<string, string[]> | null>(null);
  /** Fichier choisi pour chaque sample (clé : `key`). */
  const [choices, setChoices] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const unlisten = listen<DoctorProgress>("doctor-progress", (e) => setProgress(e.payload));
    analyse();
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  async function analyse() {
    setBusy("analyse");
    setError(null);
    try {
      setProjects(await api.doctorMissingSamples());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
      setProgress(null);
    }
  }

  const names = useMemo(() => {
    const set = new Set<string>();
    projects?.forEach((p) => p.samples.forEach((s) => s.relinkable && set.add(s.name)));
    return [...set];
  }, [projects]);

  async function search() {
    setBusy("search");
    setError(null);
    try {
      const found = await api.doctorFindCandidates(names);
      setCandidates(found);
      // Un seul fichier du même nom : on le propose d'office.
      const pre: Record<string, string> = {};
      projects?.forEach((p) =>
        p.samples.forEach((s) => {
          const list = found[s.name];
          if (s.path && list?.length === 1) pre[key(p.project_id, s.path)] = list[0];
        }),
      );
      setChoices((c) => ({ ...pre, ...c }));
      const total = Object.values(found).filter((l) => l.length > 0).length;
      toast(
        total === 0
          ? "Aucun fichier du même nom dans les dossiers scannés"
          : `${total} sample${total > 1 ? "s" : ""} retrouvé${total > 1 ? "s" : ""} (au moins un candidat)`,
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
      setProgress(null);
    }
  }

  async function repair(p: MissingProject) {
    const relinks: Relink[] = p.samples.flatMap((s) => {
      const chosen = s.path && choices[key(p.project_id, s.path)];
      return s.path && chosen ? [{ old_path: s.path, new_path: chosen }] : [];
    });
    if (relinks.length === 0) return;
    setBusy(p.project_id);
    try {
      const r = await api.doctorRelink(p.project_id, relinks);
      const saved = r.backup
        ? `copie de sécurité : ${r.backup}`
        : r.version_id
          ? "version créée avant la modification"
          : "état précédent déjà dans les versions";
      toast(`${p.name} : ${r.relinked} sample${r.relinked > 1 ? "s" : ""} réparé${r.relinked > 1 ? "s" : ""} (${saved})`);
      const fresh = await api.doctorMissingSamples();
      setProjects(fresh);
    } catch (e) {
      toast(`${p.name} : ${e}`);
    } finally {
      setBusy(null);
      setProgress(null);
    }
  }

  const status = progressText(progress);
  const totalMissing = projects?.reduce((n, p) => n + p.samples.length, 0) ?? 0;

  return (
    <section className="doctor-body">
      <div className="actions">
        <button onClick={analyse} disabled={busy !== null}>
          {busy === "analyse" ? "Analyse…" : "Analyser à nouveau"}
        </button>
        <button className="primary" onClick={search} disabled={busy !== null || names.length === 0}>
          {busy === "search" ? "Recherche…" : "Chercher les fichiers"}
        </button>
      </div>
      {status && <p className="message">{status}</p>}
      {error && <p className="message doctor-error">Erreur : {error}</p>}
      {projects && projects.length === 0 && (
        <p className="message">Aucun sample manquant : tous les fichiers audio sont à leur place.</p>
      )}
      {projects && projects.length > 0 && (
        <p className="message">
          {totalMissing} sample{totalMissing > 1 ? "s" : ""} introuvable{totalMissing > 1 ? "s" : ""} dans{" "}
          {projects.length} projet{projects.length > 1 ? "s" : ""}. Ferme le set dans Live avant de le
          réparer.
        </p>
      )}
      <ul className="doctor-projects">
        {projects?.map((p) => {
          const chosen = p.samples.filter((s) => s.path && choices[key(p.project_id, s.path)]).length;
          return (
            <li key={p.project_id}>
              <details open={projects.length <= 3}>
                <summary>
                  <span className="doctor-name">{p.name}</span>
                  <span className="doctor-count">{p.samples.length}</span>
                </summary>
                <ul className="doctor-samples">
                  {p.samples.map((s) => {
                    const k = s.path ? key(p.project_id, s.path) : null;
                    const list = candidates?.[s.name] ?? [];
                    return (
                      <li key={(s.path ?? s.rel_path ?? "") + s.name}>
                        <div className="doctor-sample" title={s.path ?? s.rel_path ?? ""}>
                          <span>{s.name}</span>
                          <small>{s.path ?? s.rel_path}</small>
                        </div>
                        {!s.relinkable ? (
                          <small className="doctor-muted">Chemin absent du set : réparation impossible ici</small>
                        ) : candidates === null ? null : list.length === 0 ? (
                          <small className="doctor-muted">Aucun fichier du même nom</small>
                        ) : (
                          <select
                            value={(k && choices[k]) || ""}
                            onChange={(e) =>
                              k &&
                              setChoices((c) => {
                                const next = { ...c };
                                if (e.target.value) next[k] = e.target.value;
                                else delete next[k];
                                return next;
                              })
                            }
                          >
                            <option value="">— Ne pas changer —</option>
                            {list.map((c) => (
                              <option key={c} value={c}>
                                {c}
                              </option>
                            ))}
                          </select>
                        )}
                      </li>
                    );
                  })}
                </ul>
                {candidates !== null && (
                  <div className="actions">
                    <button
                      className="primary"
                      disabled={chosen === 0 || busy !== null}
                      onClick={() => repair(p)}
                    >
                      {busy === p.project_id ? "Réparation…" : `Réparer (${chosen})`}
                    </button>
                  </div>
                )}
              </details>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function Plugins({ onClose }: { onClose: () => void }) {
  const [plugins, setPlugins] = useState<PluginUse[] | null>(null);
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<string | null>(null);
  const [users, setUsers] = useState<ProjectSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.doctorPlugins().then(setPlugins, (e) => setError(String(e)));
  }, []);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (plugins ?? []).filter((p) => !q || p.name.toLowerCase().includes(q));
  }, [plugins, query]);

  function filterExplorer(plugin: string) {
    window.dispatchEvent(new CustomEvent("am:filter-plugin", { detail: { plugin } }));
    onClose();
  }

  async function toggleUsers(plugin: string) {
    if (expanded === plugin) {
      setExpanded(null);
      return;
    }
    setExpanded(plugin);
    setUsers(null);
    try {
      setUsers(await api.searchProjects({ plugin, show_hidden: true }));
    } catch (e) {
      toast(`Erreur : ${e}`);
    }
  }

  return (
    <section className="doctor-body">
      <input
        type="search"
        placeholder="Quels projets utilisent… ?"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        autoFocus
      />
      {error && <p className="message doctor-error">Erreur : {error}</p>}
      {plugins && (
        <p className="message">
          {plugins.length} plugin{plugins.length > 1 ? "s" : ""} dans la bibliothèque. Clique sur un nom
          pour filtrer l'explorateur.
        </p>
      )}
      <ul className="doctor-plugins">
        {shown.map((p) => (
          <li key={p.name}>
            <div className="doctor-plugin">
              <button className="link" onClick={() => filterExplorer(p.name)} title="Filtrer l'explorateur">
                {p.name}
              </button>
              <button className="doctor-count" onClick={() => toggleUsers(p.name)} title="Quels projets ?">
                {p.projects} projet{p.projects > 1 ? "s" : ""}
              </button>
            </div>
            {expanded === p.name && (
              <ul className="doctor-users">
                {users === null ? (
                  <li className="doctor-muted">Chargement…</li>
                ) : (
                  users.map((u) => (
                    <li key={u.id} title={u.folder}>
                      {u.name}
                      {u.hidden && <small className="doctor-muted"> (masqué)</small>}
                    </li>
                  ))
                )}
              </ul>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
