import { useEffect, useState } from "react";
import { ask, open } from "@tauri-apps/plugin-dialog";
import {
  api,
  CaptureStatus,
  formatDuration,
  formatSize,
  NO_USER_LIBRARY,
  Root,
  VaultInfo,
  WorktimeOverview,
} from "../api";

/** Dossier proposé au premier lancement. */
export const DEFAULT_ROOT = "F:\\musique\\Projets";

interface Props {
  roots: Root[];
  onRootsChange: () => void;
  scanning: boolean;
  onScan: () => void;
  vault: VaultInfo | null;
  onVaultChange: () => void;
  capture: CaptureStatus | null;
  onClose: () => void;
}

/** Réglages : dossiers scannés, coffre des versions, device de pré-écoute. */
export default function SettingsDialog(props: Props) {
  const { roots, vault, capture } = props;
  const [movingVault, setMovingVault] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [deviceMessage, setDeviceMessage] = useState<string | null>(null);
  const [worktimeOn, setWorktimeOn] = useState<boolean | null>(null);
  const [overview, setOverview] = useState<WorktimeOverview | null>(null);

  useEffect(() => {
    api.worktimeEnabled().then(setWorktimeOn, () => setWorktimeOn(null));
    api.worktimeOverview().then(setOverview, () => setOverview(null));
  }, []);

  async function toggleWorktime(on: boolean) {
    setWorktimeOn(on);
    try {
      await api.setWorktimeEnabled(on);
    } catch (e) {
      setWorktimeOn(!on);
      setMessage(`Erreur : ${e}`);
    }
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && props.onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  async function addRoot() {
    const dir = await open({ directory: true, title: "Dossier de projets Ableton" });
    if (typeof dir !== "string") return;
    await api.addRoot(dir);
    props.onRootsChange();
  }

  async function moveVault() {
    const dir = await open({
      directory: true,
      title: "Dossier du coffre (idéalement un disque dédié)",
    });
    if (typeof dir !== "string") return;
    const ok = await ask(
      `Déplacer le coffre vers ${dir} ? Les fichiers déjà rangés y sont déplacés.`,
      {
        title: "Déplacer le coffre",
        okLabel: "Déplacer",
        cancelLabel: "Annuler",
      },
    );
    if (!ok) return;
    setMovingVault(true);
    try {
      await api.setVaultDir(dir);
    } catch (e) {
      setMessage(`Erreur : ${e}`);
    } finally {
      setMovingVault(false);
      props.onVaultChange();
    }
  }

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

  return (
    <div className="overlay" onClick={props.onClose}>
      <div className="dialog settings" onClick={(e) => e.stopPropagation()}>
        <header>
          <h2>Réglages</h2>
          <button className="icon" onClick={props.onClose} aria-label="Fermer">
            ×
          </button>
        </header>

        <section>
          <h3>Dossiers scannés</h3>
          <ul className="roots">
            {roots.map((r) => (
              <li key={r.id} title={r.path}>
                <span>{r.path}</span>
                <button
                  className="icon"
                  onClick={() => api.removeRoot(r.id).then(props.onRootsChange)}
                  aria-label="Retirer"
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
          <div className="actions">
            {roots.length === 0 && (
              <button onClick={() => api.addRoot(DEFAULT_ROOT).then(props.onRootsChange)}>
                + {DEFAULT_ROOT}
              </button>
            )}
            <button onClick={addRoot}>Ajouter un dossier ou un disque…</button>
            <button
              className="primary"
              onClick={props.onScan}
              disabled={props.scanning || roots.length === 0}
            >
              {props.scanning ? "Scan en cours…" : "Scanner"}
            </button>
          </div>
        </section>

        <section>
          <h3>Coffre des versions</h3>
          {vault && (
            <p className="message" title={vault.path}>
              {vault.path} · {vault.files} fichiers · {formatSize(vault.bytes)}
            </p>
          )}
          <div className="actions">
            <button onClick={moveVault} disabled={movingVault}>
              {movingVault ? "Déplacement…" : "Changer de dossier…"}
            </button>
          </div>
          {message && <p className="message">{message}</p>}
        </section>

        <section>
          <h3>Pré-écoute</h3>
          <p className="message">
            {!capture?.listening
              ? "Port de capture occupé : une autre instance de l'appli tourne ?"
              : capture.device
                ? capture.recording
                  ? "● Écoute en cours d'enregistrement"
                  : "Device de capture connecté"
                : "Device de capture non détecté dans Live"}
            {capture?.open_set && ` · Live : ${capture.open_set}`}
          </p>
          <div className="actions">
            <button onClick={installDevice}>Installer le device…</button>
          </div>
          {deviceMessage && <p className="message">{deviceMessage}</p>}
        </section>

        <section>
          <h3>Temps passé</h3>
          <label className="check">
            <input
              type="checkbox"
              checked={worktimeOn ?? true}
              disabled={worktimeOn === null}
              onChange={(e) => toggleWorktime(e.target.checked)}
            />{" "}
            Suivre le temps passé
          </label>
          <p className="message">
            Le temps compte quand le set est ouvert dans Live et que tu es là (clavier, souris ou
            lecture en cours).
          </p>
          {overview && (
            <p className="message">
              Cette semaine : {formatDuration(overview.week)}
              {overview.top.length > 0 &&
                " · " +
                  overview.top
                    .slice(0, 3)
                    .map((p) => `${p.name} (${formatDuration(p.seconds)})`)
                    .join(", ")}
            </p>
          )}
        </section>
      </div>
    </div>
  );
}
