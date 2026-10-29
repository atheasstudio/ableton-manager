import { useEffect, useState } from "react";
import {
  cleanFilter,
  Collection,
  ExplorerFilter,
  listCollections,
  newId,
  sameFilter,
  saveCollections,
  toast,
} from "../api-explorer";
import ContextMenu, { MenuState } from "./ContextMenu";

interface Props {
  filter: ExplorerFilter;
  onFilter: (f: ExplorerFilter) => void;
}

/**
 * Collections intelligentes, en haut de l'explorateur : un clic applique le
 * filtre enregistré (un second clic le retire), clic droit pour renommer ou
 * supprimer, « + » enregistre le filtre courant.
 */
export default function Collections({ filter, onFilter }: Props) {
  const [collections, setCollections] = useState<Collection[]>([]);
  const [menu, setMenu] = useState<MenuState | null>(null);

  useEffect(() => {
    listCollections()
      .then(setCollections)
      .catch((e) => toast(`Collections indisponibles : ${e}`));
  }, []);

  /** Met à jour la liste tout de suite, puis l'enregistre. */
  async function store(next: Collection[]): Promise<boolean> {
    const before = collections;
    setCollections(next);
    try {
      await saveCollections(next);
      return true;
    } catch (e) {
      setCollections(before);
      toast(`Erreur : ${e}`);
      return false;
    }
  }

  const current = cleanFilter(filter);
  const empty = Object.keys(current).length === 0;

  function add() {
    const name = window.prompt("Nom de la collection (filtre courant) :")?.trim();
    if (!name) return;
    store([...collections, { id: newId(), name, filter: current }]);
  }

  function openMenu(e: React.MouseEvent, c: Collection) {
    e.preventDefault();
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: [
        {
          label: "Renommer…",
          action: () => {
            const name = window.prompt(`Nouveau nom pour « ${c.name} » :`, c.name)?.trim();
            if (name && name !== c.name) store(collections.map((x) => (x.id === c.id ? { ...x, name } : x)));
          },
        },
        {
          label: "Remplacer par le filtre courant",
          disabled: empty || sameFilter(c.filter, current),
          action: () => store(collections.map((x) => (x.id === c.id ? { ...x, filter: current } : x))),
        },
        {
          label: "Supprimer",
          danger: true,
          action: async () => {
            const before = collections;
            if (await store(collections.filter((x) => x.id !== c.id)))
              toast({
                message: `Collection « ${c.name} » supprimée`,
                undo: async () => {
                  setCollections(before);
                  await saveCollections(before);
                },
              });
          },
        },
      ],
    });
  }

  return (
    <div className="collections">
      {collections.map((c) => {
        const active = sameFilter(c.filter, filter);
        return (
          <button
            key={c.id}
            className={`collection ${active ? "active" : ""}`}
            onClick={() => onFilter(active ? {} : c.filter)}
            onContextMenu={(e) => openMenu(e, c)}
            title={active ? "Retirer ce filtre" : "Appliquer ce filtre · clic droit : renommer, supprimer"}
          >
            {c.name}
          </button>
        );
      })}
      <button
        className="collection add"
        disabled={empty || collections.some((c) => sameFilter(c.filter, current))}
        onClick={add}
        title={empty ? "Choisis d'abord un filtre à enregistrer" : "Enregistrer le filtre courant comme collection"}
      >
        +
      </button>
      {menu && <ContextMenu menu={menu} onClose={() => setMenu(null)} />}
    </div>
  );
}
