import { useEffect, useRef, useState } from "react";

export interface Marker {
  /** Position en secondes. */
  at: number;
  label: string;
}

interface Props {
  /** Niveau crête de chaque tranche, de 0 à 255. */
  peaks: number[];
  duration: number;
  /** Position de lecture, en secondes. */
  position: number;
  /** Absent : forme d'onde décorative, sans clic ni survol (cartes de l'explorateur). */
  onSeek?: (seconds: number) => void;
  /** Repères posés sur la forme d'onde (annotations). */
  markers?: Marker[];
  height?: number;
}

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/** Forme d'onde cliquable : un clic (ou un glissé) déplace la lecture. Sans onSeek, simple image. */
export default function Waveform({ peaks, duration, position, onSeek, markers = [], height = 56 }: Props) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [width, setWidth] = useState(0);
  const [hover, setHover] = useState<number | null>(null);

  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const el = canvas.current;
    if (!el || width === 0) return;
    const ratio = window.devicePixelRatio || 1;
    el.width = Math.round(width * ratio);
    el.height = Math.round(height * ratio);
    const ctx = el.getContext("2d")!;
    ctx.scale(ratio, ratio);
    ctx.clearRect(0, 0, width, height);

    const played = duration > 0 ? (position / duration) * width : 0;
    const [accent, muted, text] = [cssVar("--accent"), cssVar("--border"), cssVar("--muted")];
    const bar = 2;
    const mid = height / 2;
    for (let x = 0; x < width; x += bar + 1) {
      // Crête maximale des tranches couvertes par cette barre.
      const from = Math.floor((x / width) * peaks.length);
      const to = Math.max(from + 1, Math.floor(((x + bar) / width) * peaks.length));
      let peak = 0;
      for (let i = from; i < to && i < peaks.length; i++) peak = Math.max(peak, peaks[i]);
      const h = Math.max(1, (peak / 255) * (height - 4));
      ctx.fillStyle = x < played ? accent : muted;
      ctx.fillRect(x, mid - h / 2, bar, h);
    }

    ctx.fillStyle = text;
    for (const m of markers) {
      if (duration <= 0) break;
      const x = Math.round((m.at / duration) * width);
      ctx.fillRect(x, 0, 1, height);
    }
    if (hover !== null) {
      ctx.fillStyle = text;
      ctx.fillRect(hover, 0, 1, height);
    }
  }, [peaks, duration, position, markers, width, height, hover]);

  function seconds(e: React.PointerEvent) {
    const rect = e.currentTarget.getBoundingClientRect();
    const fraction = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    return fraction * duration;
  }

  if (!onSeek)
    return (
      <canvas ref={canvas} className="waveform static" style={{ height }}>
        {markers.map((m) => m.label).join(", ")}
      </canvas>
    );

  return (
    <canvas
      ref={canvas}
      className="waveform"
      style={{ height }}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        onSeek(seconds(e));
      }}
      onPointerMove={(e) => {
        const rect = e.currentTarget.getBoundingClientRect();
        setHover(e.clientX - rect.left);
        if (e.buttons === 1) onSeek(seconds(e));
      }}
      onPointerLeave={() => setHover(null)}
    >
      {markers.map((m) => m.label).join(", ")}
    </canvas>
  );
}
