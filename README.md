# Ableton Manager

Appli desktop Windows pour retrouver, écouter, annoter et versionner ses projets Ableton Live sans les ouvrir.

Stack : Tauri 2 (Rust) + React, SQLite.

## Phase 1 : bibliothèque

- Ajout de dossiers (ou de disques entiers) à scanner. Les `Backup/` de Live et les dossiers système sont ignorés.
- Lecture des `.als` : tempo, signature, version de Live, pistes, plugins, samples.
- Recherche instantanée sur le nom, la note, les plugins, les samples et les tags (sans tenir compte des accents).
- Tags, statut, favoris, note libre ; boutons « Ouvrir dans Ableton » et « Ouvrir le dossier ».

## Phase 2 : pont Ableton

- Onglet « Timeline » d'un projet : une règle en mesures (signature et durée lues dans le `.als`), les locators du set et tes annotations.
- Un clic sur la timeline pose une annotation sur la mesure la plus proche ; on la nomme et on peut la déplacer (saisie `41`, `41.3` ou `41.3.2`, comme dans Live). La durée en mm:ss est affichée au tempo du set.
- Un clic sur une annotation envoie la tête de lecture de Live à cet endroit (et lance la lecture si la case est cochée). Si un autre set est ouvert, ou si Live est fermé, l'appli ouvre le projet et attend qu'il soit chargé.
- « + Position de Live » crée une annotation là où est la tête de lecture ; « Importer les locators » reprend ceux du set.
- Les positions sont stockées en beats : une annotation reste juste si le tempo change.

### Installer le Remote Script

1. Onglet Timeline → « Installer le script ». Il est copié dans `Documents\Ableton\User Library\Remote Scripts\AbletonManager` (l'appli demande le dossier « User Library » s'il est ailleurs).
2. Redémarre Live, puis Préférences > Link, Tempo & MIDI > Control Surface : choisis `AbletonManager` (entrée et sortie : aucune).

Le script (`remote-script/AbletonManager`) écoute en OSC/UDP sur `127.0.0.1:11040`, avec les adresses d'[AbletonOSC](https://github.com/ideoforms/AbletonOSC) (`/live/song/set/current_song_time`, …) plus `/manager/status` et `/manager/jump`. L'API de Live ne donne pas le fichier du set ouvert : l'appli le lit dans le titre de la fenêtre de Live.

## Phases 4 et 5 : coffre, versions et branches

- Onglet « Versions » d'un projet → « Activer les versions ». L'historique du dossier `Backup/` de Live peut être repris.
- Chaque sauvegarde dans Ableton crée une version (surveillance du dossier, plus un rattrapage au lancement). Elle garde le `.als` et tous les audios qu'il utilise, où qu'ils soient. Seuls les samples intégrés à Live sont exclus.
- Coffre : fichiers rangés par empreinte BLAKE3 (`objects/ab/cdef…`), donc stockés une seule fois. Il est placé par défaut dans les données de l'appli et peut être déplacé vers un disque dédié.
- Chaque version reçoit un nom automatique qui résume les changements (pistes, plugins, tempo, audios). On peut la renommer.
- Restaurer : l'état actuel est sauvegardé d'abord, puis le set revient à la version choisie, ce qui crée une nouvelle version. Les audios effacés ou modifiés sont remis en place.
- Nouvelle branche : recrée un dossier complet `<projet> - <branche> Project` à côté de l'original. Ses sauvegardes alimentent la branche.
- Arbre (une colonne par branche) et comparaison de deux versions. L'emplacement des aperçus audio est prévu (`preview_hash`) pour la pré-écoute.
- Supprimer une version rattache ses enfants à sa parente et libère les fichiers que plus rien n'utilise.

## Développer

Prérequis : [Rust](https://rustup.rs), Node 22 et les [prérequis Tauri](https://tauri.app/start/prerequisites/) (WebView2 est déjà présent sur Windows 10/11).

```sh
npm install
npm run tauri dev      # lance l'appli
npm run tauri build    # installeur Windows dans src-tauri/target/release/bundle/
cargo test --workspace # tests
python remote-script/tests/test_remote_script.py  # tests du Remote Script, sans Live
```

Pour tester le parseur sur de vrais projets (non versionnés) :

```sh
ALS_FIXTURES="F:/musique/Projets" cargo test --workspace -- --nocapture
```

## Organisation

| Dossier | Contenu |
| --- | --- |
| `crates/als-parser` | Lecture des `.als` (XML gzip), testable seule |
| `remote-script/AbletonManager` | Remote Script Python installé dans Live |
| `src-tauri/src` | Appli Rust : base SQLite (`db.rs`), scan (`scanner.rs`), coffre (`vault.rs`), versions et branches (`versions.rs`), détection des sauvegardes (`watcher.rs`), annotations (`annotations.rs`), pont OSC avec Live (`live.rs`, `live_window.rs`), commandes (`commands.rs`) |
| `src` | Interface React |

La base est stockée dans `%APPDATA%\studio.atheas.ableton-manager\library.db`.
