# Ableton Manager

Appli desktop Windows pour retrouver, écouter, annoter et versionner ses projets Ableton Live sans les ouvrir.

Stack : Tauri 2 (Rust) + React, SQLite.

## Phase 1 : bibliothèque

- Ajout de dossiers (ou de disques entiers) à scanner. Les `Backup/` de Live et les dossiers système sont ignorés.
- Lecture des `.als` : tempo, signature, version de Live, pistes, plugins, samples.
- Recherche instantanée sur le nom, la note, les plugins, les samples et les tags (sans tenir compte des accents).
- Tags, statut, favoris, note libre ; boutons « Ouvrir dans Ableton » et « Ouvrir le dossier ».

## Développer

Prérequis : [Rust](https://rustup.rs), Node 22 et les [prérequis Tauri](https://tauri.app/start/prerequisites/) (WebView2 est déjà présent sur Windows 10/11).

```sh
npm install
npm run tauri dev      # lance l'appli
npm run tauri build    # installeur Windows dans src-tauri/target/release/bundle/
cargo test --workspace # tests
```

Pour tester le parseur sur de vrais projets (non versionnés) :

```sh
ALS_FIXTURES="F:/musique/Projets" cargo test --workspace -- --nocapture
```

## Organisation

| Dossier | Contenu |
| --- | --- |
| `crates/als-parser` | Lecture des `.als` (XML gzip), testable seule |
| `src-tauri/src` | Appli Rust : base SQLite (`db.rs`), scan (`scanner.rs`), commandes (`commands.rs`) |
| `src` | Interface React |

La base est stockée dans `%APPDATA%\studio.atheas.ableton-manager\library.db`.
