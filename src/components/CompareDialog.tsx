import { useEffect, useRef, useState } from "react";
import { api, Branch, Diff, formatDateTime, Version, versionLabel } from "../api";
import PreviewPlayer from "./PreviewPlayer";
import { laneColor } from "./VersionGraph";

interface Props {
  /** Les deux versions, la plus ancienne en premier. */
  a: Version;
  b: Version;
  branches: Branch[];
  timeSig: string | null;
  onClose: () => void;
}

/**
 * Aperçu d'une version. Les deux lecteurs partagent la position : passer de
 * l'un à l'autre reprend au même endroit du morceau, pour comparer à l'écoute.
 */
function PreviewSlot({
  version,
  timeSig,
  shared,
}: {
  version: Version;
  timeSig: string | null;
  shared: { current: number };
}) {
  if (!version.preview_hash) {
    return <div className="preview-slot">Pas d'aperçu pour cette version</div>;
  }
  return (
    <PreviewPlayer
      hash={version.preview_hash}
      tempo={version.tempo}
      timeSig={timeSig}
      startAt={() => shared.current}
      onTime={(t) => (shared.current = t)}
      compact
    />
  );
}

function List({ title, added, removed }: { title: string; added: string[]; removed: string[] }) {
  if (added.length === 0 && removed.length === 0) return null;
  return (
    <section>
      <h3>{title}</h3>
      <ul className="diff-list">
        {added.map((x, i) => (
          <li key={`+${i}`} className="added">
            + {x}
          </li>
        ))}
        {removed.map((x, i) => (
          <li key={`-${i}`} className="removed">
            − {x}
          </li>
        ))}
      </ul>
    </section>
  );
}

function Changed<T>({
  title,
  change,
  unit = "",
}: {
  title: string;
  change: [T, T] | null;
  unit?: string;
}) {
  if (!change) return null;
  const show = (v: T) => (v === null ? "–" : `${v}${unit}`);
  return (
    <section>
      <h3>{title}</h3>
      <p>
        {show(change[0])} → {show(change[1])}
      </p>
    </section>
  );
}

export default function CompareDialog({ a, b, branches, timeSig, onClose }: Props) {
  const shared = useRef(0);
  const [diff, setDiff] = useState<Diff | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.compareVersions(a.id, b.id).then(setDiff, (e) => setError(String(e)));
  }, [a.id, b.id]);

  const branchName = (v: Version) => branches.find((x) => x.id === v.branch_id)?.name ?? "";
  const track = (t: { name: string; kind: string }) => `${t.name} (${t.kind})`;
  const empty =
    diff &&
    !diff.tempo &&
    !diff.time_sig &&
    !diff.live_version &&
    [
      diff.tracks_added,
      diff.tracks_removed,
      diff.plugins_added,
      diff.plugins_removed,
      diff.samples_added,
      diff.samples_removed,
      diff.locators_added,
      diff.locators_removed,
    ].every((l) => l.length === 0);

  return (
    <div className="overlay" onClick={onClose}>
      <div className="dialog compare" onClick={(e) => e.stopPropagation()}>
        <header>
          <h2>Comparer deux versions</h2>
          <button className="icon" onClick={onClose} aria-label="Fermer">
            ×
          </button>
        </header>

        <div className="compare-heads">
          {[a, b].map((v, i) => (
            <div key={v.id}>
              <small>{i === 0 ? "Avant" : "Après"}</small>
              <strong>{versionLabel(v)}</strong>
              <small>
                <span style={{ color: laneColor(branches, v.branch_id) }}>{branchName(v)}</span> ·{" "}
                {formatDateTime(v.created_at)}
              </small>
              <PreviewSlot version={v} timeSig={timeSig} shared={shared} />
            </div>
          ))}
        </div>

        {error && <p className="missing">{error}</p>}
        {empty && (
          <p className="empty">
            Même structure : seuls les clips, réglages ou automations diffèrent.
          </p>
        )}
        {diff && (
          <div className="diff">
            <Changed title="Tempo" change={diff.tempo} unit=" BPM" />
            <Changed title="Signature" change={diff.time_sig} />
            <Changed title="Version de Live" change={diff.live_version} />
            <List
              title="Pistes"
              added={diff.tracks_added.map(track)}
              removed={diff.tracks_removed.map(track)}
            />
            <List title="Plugins" added={diff.plugins_added} removed={diff.plugins_removed} />
            <List title="Audios" added={diff.samples_added} removed={diff.samples_removed} />
            <List title="Repères" added={diff.locators_added} removed={diff.locators_removed} />
          </div>
        )}
      </div>
    </div>
  );
}
