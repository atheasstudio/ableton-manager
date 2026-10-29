import { useEffect, useRef, useState } from "react";
import { api, barAt, formatDateTime, formatTime, PREVIEW_SOURCES, PreviewInfo } from "../api";
import Waveform, { Marker } from "./Waveform";

/** MP3 déjà chargés depuis le coffre, par empreinte. */
const urls = new Map<string, Promise<string>>();

function audioUrl(hash: string): Promise<string> {
  let url = urls.get(hash);
  if (!url) {
    url = api
      .previewAudio(hash)
      .then((bytes) => URL.createObjectURL(new Blob([bytes], { type: "audio/mpeg" })));
    url.catch(() => urls.delete(hash));
    urls.set(hash, url);
  }
  return url;
}

/** Un seul aperçu joue à la fois. */
let current: HTMLAudioElement | null = null;

interface Props {
  hash: string;
  /** Signature du projet, pour afficher les mesures. */
  timeSig?: string | null;
  /** Tempo de repli si l'aperçu n'en a pas. */
  tempo?: number | null;
  /** Position où reprendre la lecture (comparaison : au même endroit que l'autre lecteur). */
  startAt?: () => number;
  onTime?: (seconds: number) => void;
  autoPlay?: boolean;
  markers?: Marker[];
  /** Cache la ligne d'informations. */
  compact?: boolean;
}

export default function PreviewPlayer({
  hash,
  timeSig = null,
  tempo = null,
  startAt,
  onTime,
  autoPlay = false,
  markers,
  compact = false,
}: Props) {
  const audio = useRef<HTMLAudioElement>(null);
  const [info, setInfo] = useState<PreviewInfo | null>(null);
  const [url, setUrl] = useState<string | null>(null);
  const [position, setPosition] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const onTimeRef = useRef(onTime);
  onTimeRef.current = onTime;

  useEffect(() => {
    setInfo(null);
    setUrl(null);
    setPosition(0);
    setError(null);
    let alive = true;
    api.getPreview(hash).then((i) => alive && setInfo(i), (e) => alive && setError(String(e)));
    audioUrl(hash).then((u) => alive && setUrl(u), (e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [hash]);

  // Suit la lecture image par image (timeupdate est trop lent pour la forme d'onde).
  useEffect(() => {
    if (!playing) return;
    let frame = 0;
    const tick = () => {
      const t = audio.current?.currentTime ?? 0;
      setPosition(t);
      onTimeRef.current?.(t);
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [playing]);

  useEffect(() => {
    if (autoPlay && url) play();
  }, [autoPlay, url]);

  useEffect(
    () => () => {
      if (current === audio.current) current = null;
    },
    [],
  );

  function play() {
    const el = audio.current;
    if (!el) return;
    if (current && current !== el) current.pause();
    current = el;
    const at = startAt?.();
    if (at !== undefined && Math.abs(el.currentTime - at) > 0.25) el.currentTime = at;
    el.play().catch((e) => setError(String(e)));
  }

  function seek(seconds: number) {
    const el = audio.current;
    if (!el) return;
    el.currentTime = seconds;
    setPosition(seconds);
    onTime?.(seconds);
  }

  const duration = info?.duration ?? 0;
  const bpm = info?.tempo ?? tempo;
  return (
    <div className="player">
      {url && (
        <audio
          ref={audio}
          src={url}
          preload="auto"
          onPlay={() => setPlaying(true)}
          onPause={() => setPlaying(false)}
          onEnded={() => setPlaying(false)}
        />
      )}
      <div className="player-row">
        <button
          className="play"
          disabled={!url}
          onClick={() => (playing ? audio.current?.pause() : play())}
          aria-label={playing ? "Pause" : "Écouter"}
        >
          {playing ? "❚❚" : "▶"}
        </button>
        {info ? (
          <Waveform
            peaks={info.peaks}
            duration={duration}
            position={position}
            onSeek={seek}
            markers={markers}
          />
        ) : (
          <div className="waveform loading" />
        )}
      </div>
      <div className="player-info">
        <span className="num">
          {formatTime(position)} / {formatTime(duration)}
          {bpm ? ` · mes. ${barAt(position, bpm, timeSig)}` : ""}
        </span>
        {!compact && info && (
          <span title={formatDateTime(info.created_at)}>{PREVIEW_SOURCES[info.source]}</span>
        )}
      </div>
      {error && <small className="missing">Lecture impossible : {error}</small>}
    </div>
  );
}
