import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, CaptureStatus, Root } from "../api";
import { DEFAULT_ROOT } from "./SettingsDialog";
import { installLiveTools } from "../liveSetup";

export const ONBOARDED_KEY = "onboarded";

interface Props {
  roots: Root[];
  onRootsChange: () => void;
  scanning: boolean;
  onScan: () => void;
  capture: CaptureStatus | null;
  onClose: () => void;
}

const STEPS = ["Tes dossiers", "Lien avec Live", "Premier scan", "C'est parti"];

/** Guide en 4 étapes affiché au premier lancement. */
export default function Onboarding(props: Props) {
  const [step, setStep] = useState(0);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function finish() {
    try {
      localStorage.setItem(ONBOARDED_KEY, "1");
    } catch {}
    props.onClose();
  }

  async function addRoot(dir?: string) {
    const d = dir ?? (await open({ directory: true, title: "Dossier de projets Ableton" }));
    if (typeof d !== "string") return;
    await api.addRoot(d);
    props.onRootsChange();
  }

  async function install() {
    setBusy(true);
    setMessage(null);
    try {
      setMessage(await installLiveTools());
    } catch (e) {
      setMessage(`Erreur : ${e}`);
    } finally {
      setBusy(false);
    }
  }

  const { roots, capture } = props;
  return (
    <div className="overlay">
      <div className="dialog settings onboarding">
        <header>
          <h2>
            {step + 1}/4 · {STEPS[step]}
          </h2>
          <button className="icon" onClick={finish} aria-label="Passer">
            ×
          </button>
        </header>

        {step === 0 && (
          <section>
            <p>Où sont tes projets Ableton ? Tu peux ajouter plusieurs dossiers ou disques entiers.</p>
            <ul className="roots">
              {roots.map((r) => (
                <li key={r.id} title={r.path}>
                  <span>{r.path}</span>
                </li>
              ))}
            </ul>
            <div className="actions">
              {!roots.some((r) => r.path === DEFAULT_ROOT) && (
                <button onClick={() => addRoot(DEFAULT_ROOT)}>+ {DEFAULT_ROOT}</button>
              )}
              <button onClick={() => addRoot()}>Ajouter un dossier…</button>
            </div>
          </section>
        )}

        {step === 1 && (
          <section>
            <p>
              Installe le Remote Script (sauts dans la timeline) et le device de capture (pré-écoutes)
              en un clic.
            </p>
            <p className="message">
              {capture?.device ? "✓ Device détecté dans Live" : "Device pas encore détecté dans Live"}
            </p>
            <div className="actions">
              <button className="primary" onClick={install} disabled={busy}>
                {busy ? "Installation…" : "Installer pour Live"}
              </button>
            </div>
            {message && <p className="message">{message}</p>}
          </section>
        )}

        {step === 2 && (
          <section>
            <p>Lance un scan pour trouver tous tes sets dans les dossiers choisis.</p>
            <div className="actions">
              <button
                className="primary"
                onClick={props.onScan}
                disabled={props.scanning || roots.length === 0}
              >
                {props.scanning ? "Scan en cours…" : "Scanner maintenant"}
              </button>
            </div>
            {roots.length === 0 && <p className="message">Ajoute d'abord un dossier (étape 1).</p>}
          </section>
        )}

        {step === 3 && (
          <section>
            <p>Quelques raccourcis utiles :</p>
            <ul>
              <li>Ctrl+K : palette de commandes et recherche de projets</li>
              <li>? : tous les raccourcis</li>
              <li>⚙ : réglages (dossiers, coffre, mises à jour)</li>
            </ul>
          </section>
        )}

        <div className="actions">
          {step > 0 && <button onClick={() => setStep(step - 1)}>Retour</button>}
          {step < 3 ? (
            <button className="primary" onClick={() => setStep(step + 1)}>
              Suivant
            </button>
          ) : (
            <button className="primary" onClick={finish}>
              Terminer
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
