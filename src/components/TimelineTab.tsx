import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Annotation, api, LiveStatus, Timeline } from "../api";
import { barLength, formatPosition, formatTime, Meter, parsePosition } from "../timeline";

interface Props {
  projectId: number;
}

const PLAY_KEY = "timeline.play";
/** Intervalle de rafraîchissement de l'état de Live. */
const POLL_MS = 1000;

export default function TimelineTab({ projectId }: Props) {
  const [timeline, setTimeline] = useState<Timeline | null>(null);
  const [status, setStatus] = useState<LiveStatus | null>(null);
  const [play, setPlay] = useState(() => localStorage.getItem(PLAY_KEY) === "1");
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  /** Annotation qui vient d'être créée : on place le curseur sur son nom. */
  const [focus, setFocus] = useState<number | null>(null);

  async function load() {
    setTimeline(await api.getTimeline(projectId));
  }

  useEffect(() => {
    setTimeline(null);
    setMessage(null);
    load().catch((e) => setMessage(`Erreur : ${e}`));
  }, [projectId]);

  useEffect(() => {
    let stopped = false;
    let timer: number | undefined;
    async function poll() {
      try {
        const s = await api.liveStatus(projectId);
        if (!stopped) setStatus(s);
      } catch {
        if (!stopped) setStatus(null);
      }
      if (!stopped) timer = window.setTimeout(poll, POLL_MS);
    }
    poll();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, [projectId]);

  if (!timeline) return message ? <p className="message">{message}</p> : null;

  const meter: Meter = { num: timeline.time_sig[0], den: timeline.time_sig[1] };
  const bar = barLength(meter);
  const { annotations, locators, tempo } = timeline;

  const connected = status?.script != null;
  const outdated = connected && status!.script! < status!.expected_script;
  const here = connected && status!.is_open !== false;
  const playhead = here ? (status!.transport?.beat ?? null) : null;

  // Au moins 16 mesures, et une mesure libre après le dernier élément.
  const last = Math.max(
    timeline.length ?? 0,
    16 * bar,
    playhead ?? 0,
    ...annotations.map((a) => a.beat),
    ...locators.map((l) => l.time),
  );
  const total = (Math.ceil(last / bar) + 1) * bar;
  const bars = total / bar;
  const step = [1, 2, 4, 8, 16, 32, 64, 128].find((s) => bars / s <= 16) ?? 256;
  const pct = (beat: number) => `${(beat / total) * 100}%`;

  const notImported = locators.filter(
    (l) => !annotations.some((a) => Math.abs(a.beat - l.time) < 1e-6 && a.label === l.name.trim()),
  );

  async function action(work: () => Promise<string | null | void>, label: string | null = null) {
    setBusy(label);
    setMessage(null);
    try {
      setMessage((await work()) ?? null);
    } catch (e) {
      setMessage(`Erreur : ${e}`);
    } finally {
      setBusy(null);
    }
  }

  function jump(beat: number) {
    const opening = here
      ? null
      : "Ouverture du projet dans Live… (Live peut demander d'enregistrer le set en cours)";
    action(async () => {
      const opened = await api.liveJump(projectId, beat, play);
      return opened ? "Projet ouvert dans Live" : null;
    }, opening);
  }

  async function add(beat: number, label = "") {
    const a = await api.addAnnotation(projectId, beat, label);
    await load();
    setFocus(a.id);
  }

  function addAt(e: React.MouseEvent<HTMLDivElement>) {
    const rect = e.currentTarget.getBoundingClientRect();
    const beat = ((e.clientX - rect.left) / rect.width) * total;
    // Calée sur le début de mesure le plus proche.
    action(() => add(Math.round(beat / bar) * bar));
  }

  function addAtLive() {
    action(async () => {
      const s = await api.liveStatus(projectId);
      if (!s.transport) return "Live ne répond pas";
      await add(Math.round(s.transport.beat * 4) / 4);
    });
  }

  function install() {
    action(async () => {
      let dir = await api.installRemoteScript(null);
      if (!dir) {
        const chosen = await open({ directory: true, title: "Dossier « User Library » d'Ableton" });
        if (typeof chosen !== "string") return;
        dir = await api.installRemoteScript(chosen);
      }
      return (
        `Script installé dans ${dir}. Redémarre Live, puis choisis « AbletonManager » dans ` +
        "Préférences > Link, Tempo & MIDI > Control Surface (entrée et sortie : aucune)."
      );
    });
  }

  function togglePlay(value: boolean) {
    setPlay(value);
    localStorage.setItem(PLAY_KEY, value ? "1" : "0");
  }

  return (
    <div className="timeline-tab">
      <div className="live-status">
        <span className={`dot ${connected ? (here ? "on" : "other") : "off"}`} />
        <span className="live-text">
          {!status && "Recherche de Live…"}
          {status &&
            !connected &&
            "Live ne répond pas : un clic sur une annotation ouvrira le projet."}
          {connected &&
            status!.is_open === false &&
            `« ${status!.open_set} » est ouvert dans Live : un clic ouvrira ce projet.`}
          {here && (status!.transport?.playing ? "Connecté à Live · lecture" : "Connecté à Live")}
        </span>
        {(!connected || outdated) && status && (
          <button onClick={install} title="Copie le Remote Script dans la User Library de Live">
            {outdated ? "Mettre à jour le script" : "Installer le script"}
          </button>
        )}
      </div>

      <div className="timeline" onClick={addAt} title="Cliquer pour ajouter une annotation">
        <div className="ruler">
          {Array.from({ length: Math.ceil(bars / step) }, (_, i) => i * step)
            // Pas de numéro coupé contre le bord droit.
            .filter((b) => b === 0 || b / bars < 0.95)
            .map((b) => (
              <span key={b} style={{ left: pct(b * bar) }}>
                {b + 1}
              </span>
            ))}
        </div>
        {timeline.length !== null && (
          <div className="arrangement" style={{ width: pct(timeline.length) }} />
        )}
        {locators.map((l, i) => (
          <span
            key={i}
            className="locator"
            style={{ left: pct(l.time) }}
            title={`Locator : ${l.name}`}
          />
        ))}
        {annotations.map((a) => (
          <button
            key={a.id}
            className="marker"
            style={{ left: pct(a.beat) }}
            title={`${a.label || "Sans nom"} · mesure ${formatPosition(a.beat, meter)} : envoyer Live ici`}
            onClick={(e) => {
              e.stopPropagation();
              jump(a.beat);
            }}
          >
            <span>{a.label || formatPosition(a.beat, meter)}</span>
          </button>
        ))}
        {playhead !== null && <span className="playhead" style={{ left: pct(playhead) }} />}
      </div>

      <div className="actions">
        <label className="check">
          <input type="checkbox" checked={play} onChange={(e) => togglePlay(e.target.checked)} />
          Lancer la lecture
        </label>
        <button
          disabled={!here || !!busy}
          onClick={addAtLive}
          title="Ajoute une annotation là où est la tête de lecture de Live"
        >
          + Position de Live
        </button>
        {notImported.length > 0 && (
          <button
            disabled={!!busy}
            onClick={() =>
              action(async () => {
                const n = await api.importLocators(projectId);
                await load();
                return `${n} locator${n > 1 ? "s" : ""} importé${n > 1 ? "s" : ""}`;
              })
            }
          >
            Importer les locators ({notImported.length})
          </button>
        )}
      </div>

      {busy && <p className="message">{busy}</p>}
      {message && <p className="message">{message}</p>}

      {annotations.length === 0 ? (
        <p className="message">
          Clique sur la timeline pour poser une annotation, puis clique dessus pour envoyer Live à
          cet endroit.
        </p>
      ) : (
        <ul className="annotations">
          {annotations.map((a) => (
            <AnnotationRow
              key={a.id}
              annotation={a}
              meter={meter}
              tempo={tempo}
              autoFocus={focus === a.id}
              onJump={() => jump(a.beat)}
              onSave={(beat, label) =>
                action(async () => {
                  await api.updateAnnotation(a.id, beat, label);
                  await load();
                })
              }
              onDelete={() =>
                action(async () => {
                  await api.deleteAnnotation(a.id);
                  await load();
                })
              }
            />
          ))}
        </ul>
      )}
    </div>
  );
}

interface RowProps {
  annotation: Annotation;
  meter: Meter;
  tempo: number | null;
  autoFocus: boolean;
  onJump: () => void;
  onSave: (beat: number, label: string) => void;
  onDelete: () => void;
}

function AnnotationRow({
  annotation: a,
  meter,
  tempo,
  autoFocus,
  onJump,
  onSave,
  onDelete,
}: RowProps) {
  const [position, setPosition] = useState(formatPosition(a.beat, meter));
  const [label, setLabel] = useState(a.label);
  const labelRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setPosition(formatPosition(a.beat, meter));
    setLabel(a.label);
  }, [a.beat, a.label, meter.num, meter.den]);

  useEffect(() => {
    if (autoFocus) labelRef.current?.focus();
  }, [autoFocus]);

  function save() {
    const beat = parsePosition(position, meter);
    if (beat === null) {
      setPosition(formatPosition(a.beat, meter));
      return;
    }
    if (beat !== a.beat || label.trim() !== a.label) onSave(beat, label);
  }

  const blurOnEnter = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") e.currentTarget.blur();
  };

  return (
    <li>
      <button className="icon jump" onClick={onJump} title="Envoyer Live ici">
        ▶
      </button>
      <input
        className="position num"
        value={position}
        onChange={(e) => setPosition(e.target.value)}
        onBlur={save}
        onKeyDown={blurOnEnter}
        title="Mesure (ou mesure.temps)"
      />
      <small className="num">{formatTime(a.beat, tempo)}</small>
      <input
        ref={labelRef}
        className="label"
        value={label}
        placeholder="Nom : drop, pont, à refaire…"
        onChange={(e) => setLabel(e.target.value)}
        onBlur={save}
        onKeyDown={blurOnEnter}
      />
      <button className="icon" onClick={onDelete} aria-label="Supprimer">
        ×
      </button>
    </li>
  );
}
