# Revue d'architecture globale — driver clavier Apple Bluetooth

> Revue du 2026-10-01, branche `audit/debug-complet` (commit de base `dd7d376`).
> Périmètre fixé par le gérant : **le projet ne concerne que le driver du clavier
> Bluetooth Apple**. Écran / DDC / moniteur / MQTT-écran sont **hors périmètre** :
> ils sont à **supprimer ou isoler**, pas à migrer.
> Tous les chiffres ci-dessous sont mesurés (`wc -l`, `cargo tree`, build chronométré)
> à cette date ; aucun n'est repris d'un document antérieur.

## 1. Verdict

**Pas de réécriture. Une amputation, puis une séparation en deux.**

1. Le cœur « clavier » existe déjà et il est sain : `keyboard.rs`, `bluez.rs`,
   `rssi.rs`, `history.rs` = **1 317 lignes** sur 5 883 lignes Rust, peu couplées
   (aucune ne dépend d'egui, chacune a ses tests unitaires sauf `history.rs`).
   Rien ne justifie de le réécrire.
2. Ce qui est faux, c'est **l'enveloppe** : un binaire GUI de 11 Mo (egui + winit,
   216 crates sur 256) qui porte aussi l'acquisition, le fournisseur de batterie BlueZ,
   le tray, le MQTT et l'historique, **plus** un démon Python de 2 455 lignes qui fait
   la même acquisition en parallèle (unité systemd `apple-kb-monitor.service`),
   **plus** un widget Plasma qui relance ce Python toutes les 30 s.
   Résultat : jusqu'à 3 lecteurs concurrents du même `/dev/hidrawN`, 2 écrivains
   d'historique (#17), 2 fournisseurs de batterie BlueZ, et une acquisition qui
   ne tourne que si une session graphique a lancé `apihub-app` (#21).
3. **Plus de la moitié du code est hors périmètre** (écran) : à sortir d'abord,
   c'est la plus grosse simplification disponible et elle rend sans objet
   une quinzaine d'issues ouvertes.
4. Cible : **un démon utilisateur headless unique** (seul propriétaire du matériel,
   de l'état, du fournisseur BlueZ, de l'historique, du tray) qui expose son état
   sur D-Bus ; **des clients minces** (fenêtre egui, widget Plasma, CLI `--json`).
   Migration en 5 étapes, chacune livrable seule, sans casser l'existant.

## 2. Mesures

### 2.1 Taille et répartition clavier / écran

| Élément | Lignes | Périmètre |
|---|---:|---|
| `apihub-app/src/keyboard.rs` | 641 | clavier |
| `apihub-app/src/bluez.rs` | 352 | clavier |
| `apihub-app/src/rssi.rs` | 227 | clavier |
| `apihub-app/src/history.rs` | 97 | clavier |
| `apihub-app/src/tray.rs` | 585 | mixte (≈ 20 % écran : menus luminosité, 10 modes d'image, molette = luminosité) |
| `apihub-app/src/mqtt.rs` | 458 | mixte (5 entités clavier, 10 entités LG + 4 commandes DDC) |
| `apihub-app/src/main.rs` | 2 582 | mixte, voir ci-dessous |
| `apihub-app/src/ddc.rs` | 550 | **écran** |
| `apihub-app/src/brightness.rs` | 298 | **écran** |
| `apihub-app/tests/test_ddc.rs` | 381 | **écran** |
| `ddc-tool/src/main.rs` | 300 | **écran** |
| `mqtt-bridge.py` | 193 | **écran** (100 % DDC via `ddc-tool`) |
| `apihub-settings` (PySide6) | 504 | mixte, **non installé par le PKGBUILD** |
| `apple-kb-monitor` (CLI Python) | 2 455 | clavier (26 lignes DDC : `--auto-brightness`) |
| `tests/test_apple_kb.py` | 828 | clavier (92 tests du CLI Python) |
| `kde/shortcuts/apple-brightness-*` | — | **écran** |
| `docs/LG_34GN850_RE.md` | 318 | **écran** |

`main.rs`, plages de fonctions spécifiques écran (mesurées par `grep -n "fn "`) :
profils DDC + presets d'application + détection de fenêtre KWin (l. 20-207),
écrivain DDC (256-280), poll DDC/presets (488-564), helpers VCP (916-948),
onglets Display + Advanced (1335-1749), sliders VCP (2376-2478), soit **≈ 840 lignes
purement écran**, plus ≈ 300 lignes écran mêlées dans les onglets System, MQTT et
Diagnostics. **≈ 45 % de `main.rs` sort avec l'écran.**

Bilan : le code Rust passe d'environ **5 900 à environ 2 700 lignes** après l'étape 1,
dont 1 317 déjà propres et testées.

### 2.2 Dépendances et build

| Mesure | Valeur |
|---|---|
| Crates uniques (`cargo tree -e normal`) | 256 |
| dont sous-arbre `eframe` | 216 |
| dont `notify-rust` | 67 (tire **zbus 5.14** alors que l'app utilise **zbus 4.4** : 2 zbus dans `Cargo.lock`) |
| dont `rumqttc` | 40 (tire tokio + rustls/ring) |
| Build release propre | **253 s** (20 cœurs, machine partagée avec d'autres builds) |
| Rebuild release après `touch history.rs` | **144 s** (`lto = true` + `opt-level = "s"` : toute modification refait la LTO complète) |
| Binaire `apihub-app` strippé | 11 Mo |

Conséquence directe : **toute modification du code clavier coûte ≈ 2,5 min de build**
parce qu'elle relinke egui en LTO. Un démon sans `eframe` retombe à ≈ 40 crates
(zbus + serde + libc + toml), build propre de l'ordre de la dizaine de secondes ;
c'est le premier argument chiffré pour séparer le démon de l'UI.

### 2.3 Tests

| Où | Tests |
|---|---:|
| `keyboard.rs` | 8 |
| `bluez.rs` | 5 |
| `rssi.rs` | 5 |
| `history.rs` | **0** |
| `tray.rs` | **0** |
| `main.rs` | 2 |
| `mqtt.rs` | 5 |
| écran (`ddc.rs`, `brightness.rs`, `tests/test_ddc.rs`) | 39 |
| Python `tests/test_apple_kb.py` | 92 |

Les tests clavier Rust ne testent que des fonctions pures ; `read_keyboard()` ouvre
directement `/dev/hidrawN` par `libc::open` + `ioctl(HIDIOCGFEATURE)`, **aucun test
ne peut exercer le décodage des rapports sans clavier** (#24, #11). Les 92 tests
Python, eux, contiennent des trames réelles : ce sont les fixtures à récupérer.

## 3. Ce que l'architecture actuelle fait mal (clavier uniquement)

| # | Constat | Preuve | Effet |
|---|---|---|---|
| A1 | Acquisition liée à la GUI | `main()` d'`apihub-app` lance tray + poll ; aucun mode headless | Pas de batterie BlueZ / historique / alerte tant qu'aucune session graphique n'a lancé l'app (#21) |
| A2 | Deux acquisitions concurrentes | `systemd/apple-kb-monitor.service` lance le Python ; `apihub-app` lit le même hidraw | 2 historiques (#17), 2 `RegisterBatteryProvider` pour le même périphérique |
| A3 | Widget Plasma = 3ᵉ lecteur | `main.qml` : `apple-kb-monitor --json` toutes les 30 s | Un processus Python + 21 ioctl HID toutes les 30 s, en plus du poll de l'app |
| A4 | État partagé = un gros `Arc<Mutex<SharedState>>` cloné à chaque frame | `main.rs` l. 239, `update()` l. 814 | Couplage UI ↔ acquisition ; impossible à exposer hors processus |
| A5 | Config artisanale | `MqttConfig::from_config_file` l. 626-688 (+ copie dans `ddc.rs`) | Échappements, sections pointées, types : #12, #30 |
| A6 | RSSI jamais lu par le paquet | `rssi.rs` exige `CAP_NET_ADMIN` ; le PKGBUILD ne fait aucun `setcap` ; seul l'ancien `rssi-helper` C (installé à la main) a `cap_net_admin=ep` | RSSI toujours `None` avec une installation propre |
| A7 | Fournisseur BlueZ figé | `bluez.rs` : `/org/bluez/hci0/...` en dur, créé une seule fois pour la 1ʳᵉ MAC vue | Adaptateur ≠ `hci0` ou changement de clavier = batterie absente de KDE |
| A8 | Reconnexion par sondage seulement | `read_keyboard()` rouvre le hidraw si la sonde 0xEA échoue ; cycle de 10 s, lecture HID 1 cycle sur 4 | Jusqu'à ≈ 40 s avant de voir une reconnexion ; aucun abonnement à `org.bluez.Device1.Connected` ni aux uevents hidraw |
| A9 | Observabilité = `eprintln!` | 0 usage de `log`/`tracing` | Pas de niveaux, pas de filtrage journald, pas d'état d'erreur consultable |
| A10 | Historique non borné | `history.rs` : `read_to_string` du fichier entier, append toutes les 300 s | Croît sans limite, relu en entier à chaque ouverture de fenêtre |
| A11 | Packaging qui écrase un fichier d'un autre paquet | `apple-kb-monitor.install` copie `DeviceItem.qml` dans le plasmoïde Bluetooth de KDE | Écrasé à chaque mise à jour de `bluedevil`, ou inversement ; hors contrôle de pacman |
| A12 | Commentaires de cadence faux | `spawn_poll_thread` : « every 2nd cycle (~10s) » pour `cycle % 4` avec un sommeil de 10 s, « every 30th cycle = ~15s » pour 300 s | Lecture du code trompeuse |

Ce qui est **bon** et à garder tel quel : décodage HID des 21 rapports et table des
10 modèles (`keyboard.rs`), fournisseur `BatteryProvider1` avec réenregistrement
quand BlueZ redémarre (`bluez.rs`), tray en zbus pur sans busy-poll (`tray.rs`),
lecture MGMT appariée (`rssi.rs`, après #56), écritures atomiques des fichiers JSON.

## 4. Architecture cible

```
                       Matériel / noyau
  /dev/hidrawN (HID feature 0xEA,0x47,0xF5,0x5A…)   org.bluez (bus système)   AF_BLUETOOTH MGMT
           │  uevent add/remove                         │ Device1.Connected          │
           ▼                                            ▼                            ▼
 ┌───────────────────────────────────────────────────────────────────┐   ┌──────────────────┐
 │ apple-kb-monitord   (systemd --user, headless, aucun egui)        │   │ akm-rssi (helper │
 │                                                                   │◄──┤ cap_net_admin=ep,│
 │  thread acquisition ── HidSource (trait) ──► Snapshot ──┐         │   │ 1 requête, JSON) │
 │    · sondage adaptatif (10 s connecté / backoff absent) │         │   └──────────────────┘
 │    · réveil immédiat sur Device1.Connected / uevent     ▼         │
 │                                          watch (Arc<RwLock> + n°) │
 │                         ┌──────────────┬──────────┬──────────┐    │
 │                         ▼              ▼          ▼          ▼    │
 │              BatteryProvider1     Historique   Tray SNI   MQTT    │
 │              (bus système,        JSONL borné  (bus       (feature│
 │               adaptateur résolu,  écrivain     session)   cargo,  │
 │               suit la MAC)        unique                  clavier)│
 │                         └──────► com.agenceapi.AppleKbMonitor1 ◄──┘
 │                                  (bus session : propriétés + PropertiesChanged,
 │                                   Refresh(), History(), LastError)
 └──────────────────────────────────────────┬────────────────────────┘
                                            │ D-Bus session (lecture seule)
            ┌───────────────────────────────┼───────────────────────────┐
            ▼                               ▼                           ▼
  apihub-app (egui, client)     apple-kb-monitord --json/--once    Widget Plasma
  lancé depuis le tray,         (client D-Bus ; repli lecture       (exécute la CLI,
  aucun accès hidraw            directe si le démon est absent)     plus de Python)

  crate akm-core (lib, sans E/S système dans la logique) :
    report.rs   décodage des 21 rapports, modèles, calibration
    battery.rs  pourcentage/tension/type de pile
    history.rs  modèle + estimation (horloge injectée)
    config.rs   serde + toml
```

### 4.1 Découpage en crates (workspace Cargo)

| Crate | Contenu | Dépendances |
|---|---|---|
| `akm-core` (lib) | décodage HID, modèle batterie, estimation d'historique, config, trait `HidSource` | serde, toml |
| `apple-kb-monitord` (bin) | acquisition, BlueZ, tray, D-Bus, historique, CLI client | akm-core, zbus, libc, tracing ; `rumqttc` derrière `--features mqtt` |
| `akm-rssi` (bin) | une requête MGMT `Get Connection Info`, sortie JSON | akm-core, libc |
| `apihub-app` (bin) | fenêtre egui, client D-Bus uniquement | akm-core, zbus, eframe |

Un seul `zbus` (aligner `notify-rust` ou envoyer la notification directement via
`org.freedesktop.Notifications` en zbus, ce qui supprime 67 crates).

### 4.2 Modèle de threads et état

- **Un seul thread** fait des E/S HID (ioctl bloquants, ≈ 21 lectures par cycle) ;
  il produit un `Snapshot` immuable et incrémente un numéro de version.
- L'état publié est un `Arc<RwLock<Snapshot>>` + numéro (motif *watch*) : les
  consommateurs (D-Bus, tray, historique, MQTT) lisent la dernière valeur, aucun
  ne tient de verrou pendant une E/S.
- Les commandes (Refresh, flash LED) passent par un `std::sync::mpsc` vers le thread
  d'acquisition : un seul endroit touche le matériel.
- zbus en API bloquante (son exécuteur interne suffit) ; pas de tokio dans le démon
  hors feature `mqtt`.

### 4.3 Robustesse à la perte de matériel

- Clavier absent → état `Connected=false`, pas d'erreur fatale, backoff 10 s → 60 s.
- Réveil immédiat sur `PropertiesChanged(Connected=true)` de `org.bluez.Device1`
  (le démon écoute déjà le bus système pour le fournisseur) ; repli sur le sondage.
- BlueZ redémarré → réenregistrement (déjà présent dans `bluez.rs`) ; adaptateur
  résolu par `ObjectManager` au lieu de `hci0` en dur ; fournisseur recréé si la MAC
  change.
- Bus session absent ou `StatusNotifierWatcher` absent → le démon continue
  (BlueZ + historique) et retente le tray (#38).
- `Restart=on-failure` + `WatchdogSec` côté systemd.

### 4.4 Démarrage sans session graphique

Unité `systemd --user` `WantedBy=default.target` : le bus session utilisateur existe
sans Wayland ; la batterie apparaît dans KDE/UPower dès l'ouverture de session, ou
même en SSH avec `loginctl enable-linger`. La fenêtre egui ne se lance qu'à la
demande (clic tray → `apihub-app`), ce qui règle #21 sans `.desktop` autostart.

### 4.5 Config, persistance, observabilité, versioning

- **Config** : `serde` + `toml` (#12), struct unique `Config { keyboard, history, mqtt: Option<_> }`,
  clés inconnues signalées, fichier `0600`. Les sections `[ddc]`, `[monitor]`,
  `[brightness]` disparaissent de `config.toml.example`.
- **Persistance** : historique JSONL sous `$XDG_STATE_HOME/apple-kb-monitor/`
  (migration une fois depuis `~/.local/share`), un seul écrivain, rotation à 90 jours ;
  exposé par `History(since)` sur D-Bus pour que l'UI ne relise plus le fichier.
- **Observabilité** : `tracing` + `tracing-subscriber` (`RUST_LOG`), sortie vers
  journald via stderr ; propriétés D-Bus `LastUpdate`, `LastError`.
- **Versioning** : version unique dans `[workspace.package]`, reprise par le PKGBUILD,
  le `metadata.json` du widget et `--version` (#10) ; interface D-Bus suffixée `1`.

### 4.6 Tests sans matériel et CI

- `trait HidSource { fn feature(&self, id: u8) -> io::Result<Vec<u8>> }` ; impl réelle
  `Hidraw`, impl de test `Fixture` alimentée par des trames réelles (celles des
  92 tests Python + un `--dump` d'un clavier réel). Couvre #24 côté clavier.
- `Clock` injectable pour `history` (0 test aujourd'hui).
- Test d'intégration D-Bus sous `dbus-run-session` (fournisseur + interface session).
- CI (#9) : `cargo fmt --check`, `clippy -D warnings`, `cargo test -p akm-core -p apple-kb-monitord`
  sans `eframe` (rapide) ; `apihub-app` compilé dans un job séparé.
  Profil `release` : garder `lto` pour le paquet, mais un profil `dev-release`
  sans LTO pour itérer.

## 5. Plan de migration

Chaque étape est livrable seule, garde l'existant fonctionnel et a un critère de
sortie vérifiable.

### Étape 1 — Retirer le périmètre écran (supprimer / isoler)

- Taguer l'état actuel (`v3.0-avec-ecran`) : c'est l'archive, rien n'est migré.
- Supprimer : `ddc.rs`, `brightness.rs`, `tests/test_ddc.rs`, `ddc-tool/`,
  `mqtt-bridge.py`, `kde/shortcuts/apple-brightness-*`, `docs/LG_34GN850_RE.md`,
  onglets Display/Advanced, profils DDC, presets d'application + détection KWin,
  menus luminosité/mode d'image du tray, entités MQTT LG, source moniteur du widget
  Plasma, `--auto-brightness` du CLI Python, sections `[ddc]/[monitor]/[brightness]`,
  groupe `i2c` dans `.install`, mots-clés DDC du `.desktop`.
- Les issues écran deviennent sans objet (#18, #19, #20 en partie, #25, #26, #27,
  #33, #34, #54, #55 en partie) : à fermer par le commit, avec la mention du tag.
- **Sortie** : `grep -rniE "ddc|vcp|i2c|brightness" apihub-app/src` = 0 ; `cargo build`
  et `cargo test` verts ; `apihub-app/src` ≤ 3 000 lignes ; PKGBUILD sans `ddc-tool`.

### Étape 2 — Workspace + `akm-core` testable sans matériel

- Workspace Cargo ; extraire décodage HID, modèle batterie, historique, config dans
  `akm-core` ; trait `HidSource` + fixtures ; config `serde`/`toml` (absorbe #12).
- `apihub-app` continue de fonctionner à l'identique en dépendant de `akm-core`.
- **Sortie** : `cargo test -p akm-core` vert sans clavier ni bus D-Bus ;
  `cargo tree -p akm-core` sans `eframe`/`zbus` ; `read_keyboard` testé sur fixture ;
  `history` ≥ 5 tests.

### Étape 3 — Démon headless `apple-kb-monitord`

- Nouveau binaire : acquisition, `BatteryProvider1` (adaptateur résolu, suivi de MAC),
  historique écrivain unique, notification batterie faible, tray, interface
  `com.agenceapi.AppleKbMonitor1`, `tracing`, MQTT clavier en feature.
- Unité `systemd --user` du démon ; l'ancienne unité Python reste installée mais
  désactivée par défaut (bascule réversible).
- Helper `akm-rssi` avec `setcap cap_net_admin=ep` dans `.install` (corrige A6).
- **Sortie** : le démon tourne sans `WAYLAND_DISPLAY` ; batterie visible dans KDE
  sans `apihub-app` lancé ; survit à `systemctl restart bluetooth` et à un
  éteint/rallumé du clavier (reconnexion < 15 s, mesurée dans le journal) ;
  `busctl --user introspect com.agenceapi.AppleKbMonitor1` répond.

### Étape 4 — Clients minces

- `apihub-app` ne lit plus le matériel : il lit D-Bus (propriétés + `History`) ;
  tray retiré de l'app (il est dans le démon) ; plus de boucle `sleep(2)` dans `main()`.
- `apple-kb-monitord --json` = client D-Bus (repli lecture directe si démon absent) ;
  le widget Plasma l'appelle au lieu du Python.
- **Sortie** : `fuser /dev/hidraw*` ne montre que le démon ; `cargo tree -p apple-kb-monitord`
  sans `eframe` ; `main.rs` d'`apihub-app` < 800 lignes (rend #15 sans objet).

### Étape 5 — Retrait du Python (#16)

- Parité CLI Rust pour ce qui est utilisé : `--once`, `--status`, `--dump`, `--json`,
  `--history`, `--export-csv`, `--metrics`, `--waybar` (#22).
- Supprimer `apple-kb-monitor` (Python), `apihub-settings`, `tests/test_apple_kb.py`
  (trames portées en fixtures à l'étape 2), `rssi-helper.c`, l'unité Python ;
  PKGBUILD sans `python`, `python-dbus-fast`, `python-paho-mqtt`.
- Les 12 bugs Python ouverts (#40 à #51) sont fermés comme obsolètes au lieu d'être
  corrigés.
- **Sortie** : `depends` du PKGBUILD sans Python ; `makepkg` vert ; aucun fichier
  `.py` ni script Python dans le dépôt.

Transverse, déjà suivi : CI #9, versions #10, PKGBUILD #14 et #7, documentation #13.

## 6. Risques

| Risque | Probabilité | Mitigation |
|---|---|---|
| Une fonction écran était utilisée au quotidien (luminosité F1/F2, HA) | moyenne | Tag d'archive ; si besoin, projet séparé qui repart du tag, sans lien avec le driver |
| Deux fournisseurs BlueZ pendant la transition (Python + démon) | moyenne | Étape 3 désactive l'unité Python à l'installation du démon ; un seul `RegisterBatteryProvider` à la fois |
| Perte d'historique au changement de chemin | faible | Migration unique au 1er démarrage, ancien fichier conservé |
| Régression de décodage HID en extrayant `akm-core` | faible | Fixtures de trames réelles avant déplacement ; tests verts avant/après |
| `setcap` perdu (copie manuelle du binaire, FS sans xattr) | faible | RSSI optionnel : absence = `None`, jamais une erreur |
| Widget Plasma dépendant d'une sortie JSON | faible | Schéma JSON figé et versionné (`"schema": 1`) |
| Patch `DeviceItem.qml` écrasé par une mise à jour KDE (A11) | élevée | Hors chemin critique ; à remplacer par le seul `BatteryProvider1` (KDE lit déjà `Battery1`) |
| Temps de build qui reste élevé pour l'UI | certaine | Le démon et `akm-core` n'en dépendent plus ; profil sans LTO pour itérer |
