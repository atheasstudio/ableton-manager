import { formatDate, ProjectSummary, STATUS_LABELS } from "../api";

interface Props {
  projects: ProjectSummary[];
  selected: number | null;
  onSelect: (id: number) => void;
}

export default function ProjectList({ projects, selected, onSelect }: Props) {
  if (projects.length === 0) {
    return <p className="empty">Aucun projet. Ajoute un dossier puis lance un scan.</p>;
  }
  return (
    <table className="projects">
      <thead>
        <tr>
          <th />
          <th>Projet</th>
          <th>BPM</th>
          <th>Statut</th>
          <th>Tags</th>
          <th>Modifié</th>
        </tr>
      </thead>
      <tbody>
        {projects.map((p) => (
          <tr
            key={p.id}
            className={p.id === selected ? "selected" : ""}
            onClick={() => onSelect(p.id)}
            title={p.folder}
          >
            <td className="fav">{p.favorite ? "★" : ""}</td>
            <td>{p.name}</td>
            <td className="num">{p.tempo ?? "–"}</td>
            <td>
              <span className={`status ${p.status}`}>{STATUS_LABELS[p.status]}</span>
            </td>
            <td className="tag-cell">{p.tags.join(", ")}</td>
            <td>{formatDate(p.modified_at)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
