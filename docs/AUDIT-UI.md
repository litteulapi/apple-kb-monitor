# Audit de l'interface — gels, coûts par trame, cas dégénérés (01/10/2026)

Périmètre : `apihub-app/src/*` (fenêtre egui/eframe 0.29.1), tray du démon
(`apihub-app/apple-kb-monitord/src/tray*`), widget Plasma (`plasma/`), appels
D-Bus clients (`apple-kb-monitord/src/client.rs`). Base : `main` @ 8f73964.
Déclencheur : capture du gérant du 01/10 14:50 (fenêtre « Ne répond plus »,
paquet installé 3.1.0-5), traitée à part dans #230. Ici : **tous les autres
défauts de la même famille**, chacun vérifié **par exécution**.

Méthode : relecture ligne à ligne, puis reproduction sous Xvfb (`:93-:95`) et
sous `kwin_wayland --virtual`, dans un bac à sable `bwrap` **sans
`/dev/hidraw*`, sans Bluetooth (`--unshare-net`), sans bus système réel, sans
`rssi-helper`**, sur un bus de session privé **sans répertoire de services**
(aucune activation du vrai démon possible), avec un faux démon et un faux
portail (`apihub-app/audit-ui/harness/`). Aucune écriture clavier, aucune
lecture hidraw, aucun `sudo`, service installé non touché.

## 1. Résultat

| # | Gravité | Défaut | Issue |
|---|---|---|---|
| 3.1 | haute | Fenêtre réduite sous Wayland : fil principal bloqué à jamais dans `eglSwapBuffers` (vsync) | #231 |
| 3.2 | haute | Verrou d'apparence tenu pendant les appels D-Bus du portail → `update()` bloqué | #232 |
| 3.3 | haute | `daemon_tray_present()` sans délai avant la fenêtre → aucune fenêtre, nom confisqué | #233 |
| 3.3bis | haute | `load_history()` (D-Bus) sur le fil d'interface au démarrage et au clic « Refresh » | #230 (déjà ouverte, preuves ajoutées) |
| 3.4 | moyenne | Tray du démon : UPower synchrone dans sa boucle → icône jamais inscrite si UPower fige | #234 |
| 3.5 | basse | Diag sans délai : « Running… » à jamais + enfant orphelin | #235 |
| 3.6 | basse | Graphe « 24 h » sans borne haute : un point futur écrase les 24 h | #236 |

Point commun des quatre premiers : **un appel D-Bus synchrone sans délai
maximal**. zbus 4.4 n'a pas de délai d'appel côté client et le bus de session
réel est `dbus-broker` (pas de `reply_timeout`) : un pair qui ne répond pas
bloque l'appelant **indéfiniment** (mesuré : 40 s pour un pair qui répond en
40 s, aucune coupure à 25 s).

## 2. Inventaire des appels bloquants

| Où (fichier:ligne) | Fil | Appel | Pire temps |
|---|---|---|---|
| `main.rs:73` → `source.rs:169-178` | **interface** (avant la 1ʳᵉ trame) | `Connection::session()` + `NameHasOwner` + `History(0)` + décodage JSON ; sinon lecture de `history.jsonl` | illimité (D-Bus) ; 39 ms de décodage pour 222 000 entrées |
| `main.rs:443-444` (« Refresh ») | **interface** | idem | illimité |
| `main.rs:124` | **interface** | `Mutex<Appearance>::lock` tenu par `portal.rs:89-96` pendant 2 `Read` D-Bus | illimité |
| `main.rs:780` (`vsync: true`) | **interface** | `eglSwapBuffers` attend un *frame callback* | illimité tant que la fenêtre est réduite (Wayland) |
| `main.rs:830` → `instance.rs:164-184` | principal, avant la fenêtre | `GetConnectionUnixProcessID` + `Properties.Get` sur le watcher SNI | illimité |
| `instance.rs:122-142` | principal, avant la fenêtre | `Activate` borné à 2 s par `recv_timeout` | 8 × (2 s + 0,5 s) ≈ 20 s — **correct** |
| `main.rs:556-672` | fil Diag | `Command::output()/status()` ×6, `zbus` session | illimité (UI non bloquée, mais Diag figé) |
| `rename.rs:22-33` | fil dédié | `SetAlias` | illimité, UI non bloquée — correct |
| `source.rs:100-126` | fil `state-source` | `Get(Json)`, signaux | illimité, UI non bloquée (« Waiting for keyboard data… ») — correct, vérifié avec `JSON_DELAY=30` |
| `tray.rs` (app, tray hérité) | fil `tray-sni` | inscription avec reprise (≤ 8 essais) | hors UI — correct |
| démon `tray.rs:616-622` → `actions.rs:204` | fil `tray` du démon | UPower `EnumerateDevices` + `Get` ×2 | illimité → #234 |
| démon `tray.rs:568-575` | fil `tray` | `RegisterStatusNotifierItem` | illimité (même fil) |
| démon `actions.rs:138-154` | fil par clic | `kdialog`/`zenity` `.output()` | attend l'utilisateur, hors boucle — correct |
| `plasma/…/DaemonLink.qml` | QML | `asyncCall` uniquement | non bloquant — correct |

Aucun `block_on` ni `sleep` sur le fil d'interface ; `Watch::get`
(`snapshot.rs:179`) ne tient son verrou que le temps d'un clone.

## 3. Défauts confirmés

### 3.1 Fenêtre réduite sous Wayland → gel (#231)

`NativeOptions { vsync: true }` (main.rs:780) + `request_repaint_after(2 s)`
(main.rs:139). Sous `kwin_wayland --virtual`, fenêtre réduite par un script
KWin : fil principal en `poll_schedule_timeout`, **0 tick CPU en 10 s**, pile
`Egl::SwapBuffers → libEGL_mesa → wl_display_dispatch_queue → ppoll`. Témoin
(même script, `minimized = false`) : `epoll_wait` de calloop, ticks qui
avancent. Pendant le gel : pas de `pong` (→ « Ne répond plus »), drapeaux
« afficher » (2ᵉ instance, `Activate`, tray) et « Quitter » ignorés.

### 3.2 Verrou du portail (#232)

Faux portail qui répond en 40 s : 1ʳᵉ trame affichée, puis fil 1 bloqué dans
`Mutex<apihub_app::portal::Appearance>::lock` ← `update (main.rs:124)`. Le
portail réel répond aujourd'hui en 8-23 ms : défaut latent mais sans borne
(démarrage de session, backend figé).

### 3.3 Watcher SNI figé au démarrage (#233)

Faux `org.kde.StatusNotifierWatcher` dont `Get` ne répond jamais : 30 s sans
fenêtre (`xdotool search` vide à chaque seconde), fil principal en futex. Le
processus détient déjà `com.agenceapi.AppleKbMonitor` : les relances
« lèvent » une fenêtre qui n'existe pas.

### 3.3bis `load_history()` sur le fil d'interface (#230)

`History()` à 40 s : **aucune trame pendant 40 s** (gdb : `ApiHubApp::new
(main.rs:73) → load_history (source.rs:172) → call_method`). Clic
« Refresh » avec réponse à 15 s : fil bloqué de t=5 à t=18 s. Détail et
coûts dans le commentaire ajouté à #230.

### 3.4 Tray du démon et UPower (#234)

`tray::tests::live_tray` (sans matériel) avec bus session + système privés :
UPower figé → un seul `EnumerateDevices`, **aucun**
`RegisterStatusNotifierItem` en 12 s ; UPower vivant → inscription immédiate.
L'appel est fait dans `Tray::run` avant `reconcile` : sans réponse d'UPower,
l'icône n'apparaît jamais et les actions du menu restent en file.

### 3.5 Diag (#235)

`keyd` factice (`exec sleep 100000`) : « Running… » encore affiché à +11 s,
bouton absent ; après la sortie de la fenêtre, l'enfant vit toujours (PPID
`systemd --user`). Supprimé à la main après la mesure.

### 3.6 Graphe : points futurs (#236)

200 points sur 23 h + 1 point à +30 j : les 200 points tiennent dans ~20 px,
sous-titre « 201 points over 743.0 h » sous le titre « (24 h) ». Points tous au
même instant : trait vertical, « 50 points over 0.0 h » (sans gravité).
Test `#[ignore]` `future_points_are_not_drawn_as_last_24h` (échoue tant que
#236 est ouvert).

## 4. Hypothèses vérifiées et écartées (pas de défaut)

| Hypothèse | Mesure | Verdict |
|---|---|---|
| Légende/étiquettes générées en masse (famille de la capture) | Code `main` : 2 entrées max, 5 graduations fixes (main.rs:487), textes `{:.2}` sur tension bornée à (0, 10) par `chart_points` ; propriétés `chart_legend_is_bounded`, `widest_legend_fits_the_narrowest_window` (largeur réelle egui < 440 px) | sain sur `main` |
| Géométrie du graphe sur plages dégénérées (constante, 1 point, 0 s, NaN/∞, minuscule) | `chart_geometry_stays_inside_the_plot` (2 000 cas) ; captures : 1 point → « Not enough data », constante, ±1e-12 V, vide → rendu correct | sain |
| Historique énorme | 100 000 points/24 h : fenêtre OK, idle ≈ 0,1 s CPU par trame (Xvfb/llvmpipe) ; 222 000 entrées : décodage 39 ms, `chart_points` 0,44 ms/trame (release) | acceptable ; le risque est l'attente D-Bus (#230) |
| Historique > limite de message D-Bus (1 000 000 entrées, 52 Mio) | le faux démon est déconnecté par le bus, l'app bascule proprement en repli local | hors domaine réel (rotation 90 j) |
| Démon lent sur `Json` / démon absent | UI fluide, « Waiting for keyboard data… » / repli local sans gel | sain |
| Repaint continu | `request_repaint_after(2 s)` ; aucune boucle `request_repaint()` ; ~0,5 trame/s au repos | sain |
| Redimensionnement 1×1 puis retour, 8000×8000 | pas de panique ni de gel ; 8000×8000 sous llvmpipe ≈ 1 cœur (rendu logiciel de 64 Mpx, pas un défaut de l'app) | sain |
| Thème clair, DPI ×3 (`WINIT_X11_SCALE_FACTOR=3`) | captures lisibles, palette claire appliquée | sain |
| Verrous empoisonnés | tous les `lock()` de l'UI passent par `unwrap_or_else(into_inner)` ou ignorent l'erreur ; aucun `lock().unwrap()` | sain |
| Paniques sur le fil d'interface | aucune indexation brute ni `unwrap` sur données ; seuls `expect` sur constantes | sain |
| Textes du tray (démon) avec noms 64 car., %, tensions, RSSI quelconques | `tray_view_is_bounded`, `tray_bucket_is_a_valid_icon_step` (2 000 cas) | sain |
| Widget Plasma | uniquement `asyncCall`, minuterie de secours 120 s, `Text.PlainText` partout | sain (gel impossible côté plasmashell) |

## 5. Observations sans issue (à garder en tête pour les correctifs)

* `instance.rs:36-41` et le tray hérité posent le drapeau « afficher » sans
  `request_repaint()` : la fenêtre se lève au plus 2 s plus tard (latence,
  pas un gel).
* `actions.rs:138` : chaque clic « Renommer » ouvre un nouveau `kdialog`
  (plusieurs boîtes possibles en parallèle).
* `main.qml:155` : `Math.round(b.percentage)` sans filtre 0..100 (la fenêtre
  et le tray filtrent, `KbReport::battery_pct`).

## 6. Tests ajoutés (`apihub-app/audit-ui/`, hors workspace)

* `tests/view_props.rs` : inclut **le code livré** `src/view.rs` et
  `apple-kb-monitord/src/tray/view.rs` par `#[path]` ; 7 propriétés `proptest`
  (2 000 cas chacune) + 1 test egui sans fenêtre + 1 mesure `#[ignore]` + 1
  régression `#[ignore]` (#236). Résultat : **34 réussis, 2 ignorés**.
  `cd apihub-app/audit-ui && cargo test --release`.
* `eframe-shim/` : `eframe::egui` sans winit, pour compiler `view.rs` hors fenêtre.
* `harness/` : faux démon / portail / watcher / UPower (`dbus-python`),
  bac à sable `bwrap`, scripts X11 (`run.sh`), Wayland (`wl_run.sh`) et tray
  (`tray_run.sh`). `export AKM_AUDIT_DIR=<dossier>` (binaires attendus dans
  `$AKM_AUDIT_DIR/target-uireview/{debug,release}`, sorties dans `out/`).
  Variables : `HIST_MODE` (`normal|const|single|zero_span|future|mixfuture|dup|tiny|empty`),
  `HIST_N`, `HIST_DELAY`, `HIST_DELAY2`, `JSON_DELAY`, `PORTAL_DELAY`,
  `WATCHER=hang`, `UPOWER=hang`, `FAKE_DAEMON=0`, `SHOTS`, `ACTION_AT`/`ACTION`.
