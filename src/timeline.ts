/** Conversions entre beats (noires, comme dans Live), mesures et secondes. */

export interface Meter {
  num: number;
  den: number;
}

/** Durée d'une mesure, en beats. */
export function barLength(m: Meter): number {
  return (m.num * 4) / m.den;
}

/** Durée d'un temps de la mesure, en beats. */
function beatLength(m: Meter): number {
  return 4 / m.den;
}

const EPSILON = 1e-6;

/** Position façon Live (mesure.temps.double-croche), réduite : « 41 », « 41.3 », « 41.3.2 ». */
export function formatPosition(beat: number, m: Meter): string {
  const bar = Math.floor(beat / barLength(m) + EPSILON);
  const inBar = Math.max(0, beat - bar * barLength(m));
  const b = Math.floor(inBar / beatLength(m) + EPSILON);
  const sixteenth = Math.floor((inBar - b * beatLength(m)) / 0.25 + EPSILON);
  if (sixteenth > 0) return `${bar + 1}.${b + 1}.${sixteenth + 1}`;
  if (b > 0) return `${bar + 1}.${b + 1}`;
  return `${bar + 1}`;
}

/** Inverse de formatPosition ; null si la saisie n'est pas une position. */
export function parsePosition(text: string, m: Meter): number | null {
  const parts = text
    .trim()
    .split(/[.,:\s]+/)
    .map(Number);
  if (parts.length === 0 || parts.length > 3 || parts.some((n) => !Number.isInteger(n) || n < 1)) {
    return null;
  }
  const [bar, b = 1, sixteenth = 1] = parts;
  return (bar - 1) * barLength(m) + (b - 1) * beatLength(m) + (sixteenth - 1) * 0.25;
}

/** « 1:23 » au tempo du set (sans tenir compte d'une automation de tempo). */
export function formatTime(beat: number, tempo: number | null): string {
  if (!tempo) return "";
  const seconds = Math.round((beat * 60) / tempo);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}
