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

## Phase 3 : pré-écoute

- Chaque projet a un aperçu audio (MP3 dans le coffre) avec sa forme d'onde : lecteur dans la fiche du projet, dans chaque version, dans la comparaison de deux versions (les deux lecteurs reprennent au même endroit) et depuis la liste (bouton ▶).
- **Écoutes dans Live** (option A) : le device Max for Live « Ableton Manager Capture », posé sur le master, enregistre ce que tu joues avec la position dans le morceau. L'appli place chaque passage à sa mesure, le plus récent l'emporte, et garde les plus anciens là où rien de plus récent ne les recouvre. L'aperçu est aligné sur la mesure 1 (seconde = beats × 60 / tempo), pour y poser les annotations.
- À chaque sauvegarde, la nouvelle version reçoit les écoutes faites depuis la version précédente. Une écoute du set sauvegardé (sans « * » dans le titre de Live) met aussi à jour l'aperçu de la dernière version.
- **Rendu complet** (option B) : bouton dans la fiche du projet. L'appli ouvre le set dans Live au besoin, puis le device le joue du début à la fin de l'arrangement (boucle coupée, 4 temps de queue) et l'enregistre.
- **Vieux projets** : « Reprendre un bounce » prend le fichier audio du dossier du projet qui porte le nom du set (sinon le plus récent), « Choisir un fichier… » n'importe lequel (WAV, AIFF, MP3, FLAC, OGG, M4A).
- Le set ouvert est lu dans le titre de la fenêtre de Live (« Set [Dossier Project] - Ableton Live 12 Suite »). Les écoutes d'un set absent de la bibliothèque sont jetées. Les écoutes en attente (WAV dans `captures/`) sont supprimées dès qu'elles sont recouvertes, après 30 jours, ou au-delà de 2 Go.

### Installer le device

Réglages de gauche, « Pré-écoute » → « Installer le device… ». Il est copié dans `Documents\Ableton\User Library\Presets\Audio Effects\Max Audio Effect` (ou dans la User Library choisie). Dans Live, glisse « Ableton Manager Capture » sur le Master, puis enregistre-le dans le set par défaut (Réglages de Live > Fichiers/Dossiers > « Enregistrer le set actuel comme set par défaut ») pour l'avoir partout. Il faut Live Suite ou Max for Live.

Le device n'enregistre que si l'appli tourne : il parle à l'appli en OSC sur `127.0.0.1:47811` (appli) et `47812` (device). Ses sources sont dans `device/` (`am-capture.maxpat`, `am-capture.js`) ; l'appli fabrique le `.amxd` à l'installation. Si Live refuse le `.amxd`, crée un « Max Audio Effect » vide, ouvre-le dans Max, colle le contenu de `am-capture.maxpat`, place `am-capture.js` à côté et enregistre.

## Phases 4 et 5 : coffre, versions et branches

- Onglet « Versions » d'un projet → « Activer les versions ». L'historique du dossier `Backup/` de Live peut être repris.
- Chaque sauvegarde dans Ableton crée une version (surveillance du dossier, plus un rattrapage au lancement). Elle garde le `.als` et tous les audios qu'il utilise, où qu'ils soient. Seuls les samples intégrés à Live sont exclus.
- Coffre : fichiers rangés par empreinte BLAKE3 (`objects/ab/cdef…`), donc stockés une seule fois. Il est placé par défaut dans les données de l'appli et peut être déplacé vers un disque dédié.
- Chaque version reçoit un nom automatique qui résume les changements (pistes, plugins, tempo, audios). On peut la renommer.
- Restaurer : l'état actuel est sauvegardé d'abord, puis le set revient à la version choisie, ce qui crée une nouvelle version. Les audios effacés ou modifiés sont remis en place.
- Nouvelle branche : recrée un dossier complet `<projet> - <branche> Project` à côté de l'original. Ses sauvegardes alimentent la branche.
- Arbre en 2D : les versions sont des nœuds, avec une ligne par branche ; on glisse pour se déplacer, on zoome à la molette, et il y a un mode plein écran. Notes libres posées dans l'espace (double-clic) ou accrochées à une version (lâcher la note dessus). Maj + clic sur deux versions pour les comparer.
- Comparaison de deux versions, à l'écoute aussi.
- Supprimer une version rattache ses enfants à sa parente et libère les fichiers que plus rien n'utilise.

## Interface

### Onglet Temps

- Dock du projet → « Temps » : total, 7 et 30 derniers jours, date de la première session, et une grille des 12 dernières semaines (une case par jour, plus elle est orangée, plus tu y as passé de temps ; survol pour le détail).
- Sessions récentes, avec leur branche si ce n'est pas la principale. La croix supprime une session (elle ne compte plus nulle part).
- L'en-tête du projet affiche « ⏱ 12 h 30 · dernière session hier » ; un clic ouvre l'onglet. Tout se recharge dès qu'une session se termine (évènement `worktime-changed`).

### Notifications

- Les messages (résultat d'un scan, d'une restauration, erreurs…) s'affichent en notifications en bas à droite, empilées ; le survol suspend le décompte.
- Actions réversibles (favori, statut, retrait d'un tag) : appliquées tout de suite, « Annuler » (4 s) fait l'inverse.
- Actions destructrices (supprimer une version, une branche, une session de travail) : rien ne part tout de suite, la notification laisse 6 s pour « Annuler » ; la suppression a lieu à son expiration (ou à la fermeture de la fenêtre). La suppression d'une branche garde sa confirmation, puisque son dossier part à la corbeille.
- N'importe quelle partie de l'interface peut en afficher une avec l'évènement DOM `am:toast` (`{ message, undo?, commit?, duration? }`).

## Suivi du temps

- Toutes les 20 s, l'appli lit le set ouvert dans Live (titre de sa fenêtre) et prolonge la session de travail du projet (table `work_sessions`, branche comprise). Une pause de plus de 3 min ouvre une nouvelle session.
- Le temps ne compte que si quelqu'un est là : clavier ou souris utilisés dans les 5 dernières minutes, ou Live en train de jouer (Remote Script).
- L'évènement Tauri `worktime-changed` (`{ project_id }`) est émis quand une session commence, se termine (set fermé, autre set ouvert, absence) ou est supprimée, pas à chaque relevé.
- Réglages → « Temps passé » : case « Suivre le temps passé » (réglage `worktime_enabled`) et total de la semaine avec les projets les plus travaillés.
- Les projets qui ont du temps noté ne sont pas retirés de la bibliothèque si leur disque est débranché.

## Collections

- Le filtre de l'explorateur accepte aussi `worked_since`, `not_worked_since`, `modified_since` (durées en secondes comptées depuis maintenant ; « pas travaillé depuis » inclut les projets jamais travaillés) et `plugin` (nom exact).
- Une collection est un filtre enregistré sous un nom : `{ id, name, filter }`. Elles sont stockées en JSON dans la table `settings` (clé `collections`), via les commandes `list_collections` et `save_collections`.
- Tant que rien n'est enregistré, trois collections sont proposées : « Touchés cette semaine », « Oubliés depuis 3 mois », « En cours ».

## Santé de la bibliothèque

Fenêtre ouverte par le bouton « Santé de la bibliothèque » de la barre de gauche (évènement DOM `am:open-doctor`).

- **Samples manquants** : liste, par projet, des samples introuvables (même test que la fiche du projet : chemin absolu ou relatif au dossier). « Chercher les fichiers » parcourt les dossiers scannés à la recherche de fichiers du même nom ; on choisit le bon candidat (proposé d'office s'il est seul), puis « Réparer » réécrit les chemins dans le `.als` (`relocate.rs`).
- Avant la réécriture : une version est créée si les versions sont actives, sinon le `.als` est copié en `.als.bak` (puis `.als.bak2`…). Le set doit être fermé dans Live. Le projet est ensuite relu pour la bibliothèque.
- **Plugins** : inventaire de tous les plugins avec le nombre de projets qui les utilisent. Un clic sur un nom filtre l'explorateur (évènement `am:filter-plugin`) ; le nombre de projets déplie la liste « Quels projets utilisent X ? ».
- Les travaux longs tournent hors du fil de l'interface et envoient leur progression par l'évènement Tauri `doctor-progress`.

## Icône dans la zone de notification

- L'appli place une icône dans la zone de notification de Windows (construite en Rust, `tray.rs`) : clic pour afficher la fenêtre, menu Ouvrir / Scanner / Quitter. « Scanner » scanne en tâche de fond et annonce le résultat dans une notification.
- La croix de la fenêtre la réduit dans la zone de notification au lieu de quitter (Réglages → Fenêtre ; réglage `close_to_tray`). Pour quitter : menu de l'icône → Quitter.
- « Lancer au démarrage de Windows » (Réglages → Fenêtre) utilise `tauri-plugin-autostart` : l'appli est lancée avec `--hidden` et reste cachée dans la zone de notification (le suivi du temps et les versions tournent).
- Une seule instance : relancer l'appli réaffiche la fenêtre de celle qui tourne déjà.

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
| `src-tauri/src` | Appli Rust : base SQLite (`db.rs`), scan (`scanner.rs`), coffre (`vault.rs`), versions et branches (`versions.rs`), détection des sauvegardes (`watcher.rs`), annotations (`annotations.rs`), pont OSC avec Live (`live.rs`, `live_window.rs`), aperçus (`previews.rs`, `audio.rs`), liaison avec le device de capture (`capture.rs`), commandes (`commands.rs`) |
| `device` | Device Max for Live de capture |
| `src` | Interface React |

La base est stockée dans `%APPDATA%\studio.atheas.ableton-manager\library.db`.
