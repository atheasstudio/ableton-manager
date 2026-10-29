import type { Filter } from "./api";

/** Champs vides retirés : deux filtres équivalents ont la même forme. */
export function cleanFilter(f: Filter): Filter {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(f)) {
    if (v === undefined || v === null || v === "" || v === false) continue;
    out[k] = typeof v === "string" ? v.trim() : v;
    if (out[k] === "") delete out[k];
  }
  return out as Filter;
}

export function sameFilter(a: Filter, b: Filter): boolean {
  const x = cleanFilter(a);
  const y = cleanFilter(b);
  const kx = Object.keys(x);
  return (
    kx.length === Object.keys(y).length &&
    kx.every((k) => (x as Record<string, unknown>)[k] === (y as Record<string, unknown>)[k])
  );
}

export function newId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  }
}

/** « 7 jours », « 3 mois », « 1 an ». */
export function formatSpan(seconds: number): string {
  const days = Math.round(seconds / 86_400);
  if (days >= 365 && days % 365 === 0) return days === 365 ? "1 an" : `${days / 365} ans`;
  if (days >= 30 && days % 30 === 0) return `${days / 30} mois`;
  if (days === 1) return "24 h";
  return `${days} jours`;
}
