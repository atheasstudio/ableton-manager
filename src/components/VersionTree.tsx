import { Branch, formatDateTime, Version, versionLabel } from "../api";

interface Props {
  branches: Branch[];
  versions: Version[];
  selected: number | null;
  onSelect: (id: number) => void;
  /** Versions cochées pour la comparaison. */
  checked: number[];
  onCheck: (id: number) => void;
}

const ROW_H = 46;
const LANE_W = 16;
/** Une couleur par branche, la principale en orange. */
export const LANE_COLORS = ["#ff7a2f", "#6cb6ff", "#57ab5a", "#d2a8ff", "#e3b341", "#f47067"];

export function laneColor(branches: Branch[], branchId: number): string {
  const i = branches.findIndex((b) => b.id === branchId);
  return LANE_COLORS[Math.max(i, 0) % LANE_COLORS.length];
}

const SOURCE_LABELS: Record<Version["source"], string> = {
  save: "",
  manual: "manuelle",
  backup: "Backup de Live",
  restore: "restauration",
};

/**
 * Arbre des versions, de la plus récente à la plus ancienne : une colonne
 * par branche, et un trait de chaque version vers sa parente.
 */
export default function VersionTree({
  branches,
  versions,
  selected,
  onSelect,
  checked,
  onCheck,
}: Props) {
  const lane = new Map(branches.map((b, i) => [b.id, i]));
  const row = new Map(versions.map((v, i) => [v.id, i]));
  const x = (v: Version) => (lane.get(v.branch_id) ?? 0) * LANE_W + 10;
  const y = (i: number) => i * ROW_H + ROW_H / 2;
  const width = branches.length * LANE_W + 6;

  const edges = versions.flatMap((v, i) => {
    const j = v.parent_id === null ? undefined : row.get(v.parent_id);
    if (j === undefined) return [];
    const parent = versions[j];
    const [x1, y1, x2, y2] = [x(v), y(i), x(parent), y(j)];
    // Une branche part de sa version d'origine par une courbe.
    const d =
      x1 === x2
        ? `M ${x1} ${y1} V ${y2}`
        : `M ${x1} ${y1} V ${y2 - ROW_H * 0.7} C ${x1} ${y2 - ROW_H * 0.2}, ${x2} ${y2 - ROW_H * 0.5}, ${x2} ${y2}`;
    return [<path key={v.id} d={d} stroke={laneColor(branches, v.branch_id)} />];
  });

  return (
    <div className="tree" style={{ ["--lanes-width" as string]: `${width}px` }}>
      <svg className="tree-lines" width={width} height={versions.length * ROW_H}>
        <g fill="none" strokeWidth={2}>
          {edges}
        </g>
        {versions.map((v, i) => (
          <circle
            key={v.id}
            cx={x(v)}
            cy={y(i)}
            r={v.id === selected ? 6 : 5}
            fill={v.source === "backup" ? "var(--surface)" : laneColor(branches, v.branch_id)}
            stroke={laneColor(branches, v.branch_id)}
            strokeWidth={2}
          />
        ))}
      </svg>
      <ul>
        {versions.map((v) => {
          const heads = branches.filter((b) => b.head_id === v.id);
          const starts = branches.filter(
            (b) => !b.is_main && b.base_id === v.id && b.head_id === v.id,
          );
          return (
            <li
              key={v.id}
              className={v.id === selected ? "selected" : ""}
              style={{ height: ROW_H }}
              onClick={() => onSelect(v.id)}
            >
              <input
                type="checkbox"
                title="Comparer"
                checked={checked.includes(v.id)}
                onClick={(e) => e.stopPropagation()}
                onChange={() => onCheck(v.id)}
              />
              <div className="version-text">
                <span className="version-name" title={v.auto_name}>
                  {versionLabel(v)}
                </span>
                <small>
                  {formatDateTime(v.created_at)}
                  {SOURCE_LABELS[v.source] && ` · ${SOURCE_LABELS[v.source]}`}
                  {v.missing > 0 && (
                    <span className="missing"> · {v.missing} audio(s) manquant(s)</span>
                  )}
                </small>
              </div>
              {heads.map((b) => (
                <span
                  key={b.id}
                  className="branch-chip"
                  style={{
                    borderColor: laneColor(branches, b.id),
                    color: laneColor(branches, b.id),
                  }}
                  title={
                    starts.includes(b)
                      ? "Branche créée ici, pas encore de sauvegarde"
                      : "État actuel"
                  }
                >
                  {b.name}
                </span>
              ))}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
