import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import {
  api,
  Branch,
  formatDateTime,
  formatSize,
  TreeNote,
  Version,
  VersionsView,
  versionLabel,
} from "../api";
import ContextMenu, { MenuState } from "./ContextMenu";
import CompareDialog from "./CompareDialog";
import PreviewPlayer from "./PreviewPlayer";
import VersionGraph, { laneColor } from "./VersionGraph";

interface Props {
  projectId: number;
  /** Signature du projet, pour afficher les mesures des aperçus. */
  timeSig: string | null;
  /** Appelé quand le coffre change de taille. */
  onVaultChange: () => void;
}

export default function VersionsTab({ projectId, timeSig, onVaultChange }: Props) {
  const [view, setView] = useState<VersionsView | null>(null);
  const [notes, setNotes] = useState<TreeNote[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [checked, setChecked] = useState<number[]>([]);
  const [comparing, setComparing] = useState(false);
  const [importBackups, setImportBackups] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [rename, setRename] = useState("");
  const [branchName, setBranchName] = useState<string | null>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const autoEnabled = useRef<number | null>(null);

  async function load() {
    setView(await api.getVersions(projectId));
    setNotes(await api.listTreeNotes(projectId));
  }

  useEffect(() => {
    setSelected(null);
    setChecked([]);
    setMessage(null);
    load();
    const unlisten = listen<{ project_id: number }>("versions-changed", (e) => {
      if (e.payload.project_id === projectId) {
        load();
        onVaultChange();
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [projectId]);

  const version = view?.versions.find((v) => v.id === selected) ?? null;
  useEffect(() => {
    setRename(version?.name ?? "");
    setBranchName(null);
  }, [version?.id]);

  /** Lance une action longue, affiche son résultat ou son erreur. */
  async function run(label: string, action: () => Promise<string | null>) {
    setBusy(label);
    setMessage(null);
    try {
      setMessage(await action());
      await load();
      onVaultChange();
    } catch (e) {
      setMessage(`Erreur : ${e}`);
    } finally {
      setBusy(null);
    }
  }

  // Les versions s'activent toutes seules à l'ouverture du projet.
  useEffect(() => {
    if (!view || view.branches.length > 0 || autoEnabled.current === projectId) return;
    autoEnabled.current = projectId;
    run("Activation des versions…", async () => {
      const n = await api.enableVersions(projectId, true);
      return `Versions activées : ${n} version${n > 1 ? "s" : ""} reprise${n > 1 ? "s" : ""}`;
    });
  }, [view, projectId]);

  if (!view) return null;

  if (view.branches.length === 0) {
    return (
      <div className="versions-off">
        <p>
          Chaque sauvegarde dans Ableton crée une version : le .als et tous les audios qu'il utilise
          sont copiés dans le coffre, une seule fois chacun. Tu peux ensuite revenir à n'importe
          quelle version ou en partir dans une nouvelle branche.
        </p>
        <label className="check">
          <input
            type="checkbox"
            checked={importBackups}
            onChange={(e) => setImportBackups(e.target.checked)}
          />
          Reprendre l'historique du dossier Backup de Live
        </label>
        <button
          className="primary"
          disabled={!!busy}
          onClick={() =>
            run("Activation…", async () => {
              const n = await api.enableVersions(projectId, importBackups);
              return `${n} version${n > 1 ? "s" : ""} créée${n > 1 ? "s" : ""}`;
            })
          }
        >
          {busy ?? "Activer les versions"}
        </button>
        {message && <p className="message">{message}</p>}
      </div>
    );
  }

  const branchOf = (id: number) => view.branches.find((b) => b.id === id);
  const toggleCheck = (id: number) =>
    setChecked((c) => (c.includes(id) ? c.filter((x) => x !== id) : [...c.slice(-1), id]));
  const pair = checked
    .map((id) => view.versions.find((v) => v.id === id)!)
    .filter(Boolean)
    .sort((x, y) => x.created_at - y.created_at || x.id - y.id);

  async function restore(target: Version | null = version) {
    const version = target;
    if (!version) return;
    const b = branchOf(version.branch_id)!;
    const ok = await ask(
      `Le set de la branche « ${b.name} » va revenir à « ${versionLabel(version)} ».\n\n` +
        "Ferme-le dans Ableton avant. L'état actuel est d'abord sauvegardé comme version, rien n'est perdu.",
      {
        title: "Restaurer cette version",
        kind: "warning",
        okLabel: "Restaurer",
        cancelLabel: "Annuler",
      },
    );
    if (!ok) return;
    run("Restauration…", async () => {
      const r = await api.restoreVersion(version.id);
      if (r.version_id) setSelected(r.version_id);
      return r.lost.length
        ? `Restauré, mais ${r.lost.length} audio(s) manquaient déjà à la sauvegarde : ${r.lost.join(", ")}`
        : "Version restaurée. Tu peux rouvrir le set dans Ableton.";
    });
  }

  function createBranch(e: React.FormEvent) {
    e.preventDefault();
    if (!version || !branchName?.trim()) return;
    const name = branchName;
    run("Création de la branche…", async () => {
      const r = await api.createBranch(version.id, name);
      setBranchName(null);
      const lost = r.lost.length ? ` (${r.lost.length} audio(s) introuvables)` : "";
      return `Branche « ${r.branch.name} » créée dans ${r.branch.folder}${lost}`;
    });
  }

  async function deleteBranch(b: Branch) {
    const ok = await ask(
      `Supprimer la branche « ${b.name} » et ses versions ? Son dossier part à la corbeille :\n${b.folder}`,
      { title: "Supprimer la branche", kind: "warning", okLabel: "Supprimer", cancelLabel: "Annuler" },
    );
    if (!ok) return;
    run("Suppression de la branche…", async () => {
      const freed = await api.deleteBranch(b.id);
      setSelected(null);
      setChecked([]);
      return `Branche supprimée, ${formatSize(freed)} libérés`;
    });
  }

  function branchFrom(v: Version) {
    const name = window.prompt(`Nom de la nouvelle branche (depuis « ${versionLabel(v)} ») :`);
    if (!name?.trim()) return;
    run("Création de la branche…", async () => {
      const r = await api.createBranch(v.id, name);
      return `Branche « ${r.branch.name} » créée dans ${r.branch.folder}`;
    });
  }

  function branchMenu(b: Branch, e: React.MouseEvent) {
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: "▶ Ouvrir dans Live", action: () => api.openBranch(b.id) },
        { label: "Ouvrir le dossier", action: () => api.openBranchFolder(b.id) },
        {
          label: "Créer une version maintenant",
          action: () =>
            run("Sauvegarde…", async () =>
              (await api.snapshotNow(b.id)) ? "Version créée" : "Rien n'a changé depuis la dernière version",
            ),
        },
        { label: "Supprimer la branche", danger: true, disabled: b.is_main, action: () => deleteBranch(b) },
      ],
    });
  }

  function versionMenu(v: Version, e: React.MouseEvent) {
    const b = branchOf(v.branch_id);
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: "Nouvelle branche depuis ici…", action: () => branchFrom(v) },
        { label: "Restaurer cette version", action: () => restore(v) },
        { label: "Cocher pour comparer", action: () => toggleCheck(v.id) },
        ...(b ? [{ label: `Ouvrir « ${b.name} » dans Live`, action: () => api.openBranch(b.id) }] : []),
        { label: "Supprimer la version", danger: true, action: () => remove(v) },
        ...(b && !b.is_main
          ? [{ label: `Supprimer la branche « ${b.name} »`, danger: true, action: () => deleteBranch(b) }]
          : []),
      ],
    });
  }

  async function remove(target: Version | null = version) {
    const version = target;
    if (!version) return;
    const ok = await ask(
      `Supprimer « ${versionLabel(version)} » ? Les audios que plus aucune version n'utilise quittent le coffre.`,
      {
        title: "Supprimer la version",
        kind: "warning",
        okLabel: "Supprimer",
        cancelLabel: "Annuler",
      },
    );
    if (!ok) return;
    run("Suppression…", async () => {
      const freed = await api.deleteVersion(version.id);
      setSelected(null);
      setChecked((c) => c.filter((x) => x !== version.id));
      return `Version supprimée, ${formatSize(freed)} libérés`;
    });
  }

  return (
    <div className="versions">
      {menu && <ContextMenu menu={menu} onClose={() => setMenu(null)} />}
      <ul className="branches">
        {view.branches.map((b) => (
          <li key={b.id} title={b.als_path}>
            <span className="dot" style={{ background: laneColor(view.branches, b.id) }} />
            <span className="branch-name">{b.name}</span>
            <button
              onClick={() => api.openBranch(b.id)}
              title="Ouvrir le set de cette branche dans Live"
            >
              ▶ Live
            </button>
            <button
              onClick={() => api.openBranchFolder(b.id)}
              title="Ouvrir le dossier de la branche"
            >
              Dossier
            </button>
            <button
              disabled={!!busy}
              title="Créer une version sans attendre une sauvegarde"
              onClick={() =>
                run("Sauvegarde…", async () =>
                  (await api.snapshotNow(b.id))
                    ? "Version créée"
                    : "Rien n'a changé depuis la dernière version",
                )
              }
            >
              + Version
            </button>
            {!b.is_main && (
              <button
                className="danger"
                disabled={!!busy}
                title="Supprimer cette branche"
                onClick={() => deleteBranch(b)}
              >
                ✕
              </button>
            )}
          </li>
        ))}
      </ul>

      <VersionGraph
        branches={view.branches}
        versions={view.versions}
        notes={notes}
        selected={selected}
        onSelect={setSelected}
        checked={checked}
        onToggleCheck={toggleCheck}
        onOpenBranch={(id) => api.openBranch(id)}
        onVersionMenu={versionMenu}
        onBranchMenu={branchMenu}
        onAddNote={async (versionId, x, y) => {
          const note = await api.addTreeNote(projectId, versionId, x, y);
          setNotes((all) => [...all, note]);
          return note;
        }}
        onUpdateNote={(note) => {
          setNotes((all) => all.map((n) => (n.id === note.id ? note : n)));
          api.updateTreeNote(note);
        }}
        onDeleteNote={(id) => {
          setNotes((all) => all.filter((n) => n.id !== id));
          api.deleteTreeNote(id);
        }}
        overlay={
          <>
            {busy && <p className="message">{busy}</p>}
            {message && <p className="message">{message}</p>}

            {checked.length === 2 && (
              <button className="primary" onClick={() => setComparing(true)}>
                Comparer les 2 versions cochées
              </button>
            )}

            {version && (
              <div className="version-detail">
                <form
                  onSubmit={(e) => {
                    e.preventDefault();
                    api.renameVersion(version.id, rename).then(load);
                  }}
                >
                  <input
                    value={rename}
                    placeholder={version.auto_name}
                    onChange={(e) => setRename(e.target.value)}
                    onBlur={() =>
                      rename !== (version.name ?? "") &&
                      api.renameVersion(version.id, rename).then(load)
                    }
                    title="Renommer la version"
                  />
                </form>
                <small>
                  {formatDateTime(version.created_at)} · {branchOf(version.branch_id)?.name}
                  {version.tempo && ` · ${version.tempo} BPM`} · {version.files} audio(s)
                </small>
                {version.name && <small>{version.auto_name}</small>}
                {version.preview_hash ? (
                  <PreviewPlayer
                    hash={version.preview_hash}
                    tempo={version.tempo}
                    timeSig={timeSig}
                  />
                ) : (
                  <small>
                    Pas d'aperçu : le set n'a pas été écouté depuis la version précédente.
                  </small>
                )}
                <div className="actions">
                  {branchOf(version.branch_id)?.head_id === version.id && (
                    <button
                      className="primary"
                      onClick={() => api.openBranch(version.branch_id)}
                      title="C'est l'état actuel de cette branche"
                    >
                      Ouvrir dans Live
                    </button>
                  )}
                  <button disabled={!!busy} onClick={() => restore()}>
                    Restaurer
                  </button>
                  <button
                    disabled={!!busy}
                    onClick={() => setBranchName(branchName === null ? "" : null)}
                  >
                    Nouvelle branche…
                  </button>
                  {version.parent_id !== null && (
                    <button
                      onClick={() => {
                        setChecked([version.parent_id!, version.id]);
                        setComparing(true);
                      }}
                    >
                      Comparer à la précédente
                    </button>
                  )}
                  <button className="danger" disabled={!!busy} onClick={() => remove()}>
                    Supprimer
                  </button>
                </div>
                {branchName !== null && (
                  <form className="branch-form" onSubmit={createBranch}>
                    <input
                      autoFocus
                      placeholder="Nom : version club, radio edit…"
                      value={branchName}
                      onChange={(e) => setBranchName(e.target.value)}
                    />
                    <button className="primary" disabled={!branchName.trim() || !!busy}>
                      Créer
                    </button>
                  </form>
                )}
              </div>
            )}
          </>
        }
      />

      {comparing && pair.length === 2 && (
        <CompareDialog
          a={pair[0]}
          b={pair[1]}
          branches={view.branches}
          timeSig={timeSig}
          onClose={() => setComparing(false)}
        />
      )}
    </div>
  );
}
