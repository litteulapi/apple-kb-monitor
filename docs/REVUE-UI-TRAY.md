# Revue UI et tray : faut-il changer l'architecture d'interface ?

> Revue du 2026-10-01, sur `main` @ `620013d` (démon `apple-kb-monitord` 3.1.0 installé et actif).
> Poste de mesure : PC01, Manjaro, Plasma 6.7.5 Wayland, Qt 6.11.2, Kirigami/KQuickCharts 6.30, breeze-icons 6.30, adwaita-icon-theme 50.
> Instantané daté, non mis à jour avec le code. Plan suivi dans les issues Gitea « UI » (milestones v3.1 et v3.2).

## 1. Verdict

**On ne change pas de toolkit, on déplace le centre de gravité de l'interface.**

1. **Sous Plasma 6, l'interface principale devient le plasmoïde, logé dans la zone de notification** (option b).
   Il s'affiche seul quand le démon est sur le bus (`X-Plasma-DBusActivationService`, même mécanisme que
   l'applet Bluetooth de Plasma avec `org.bluez`), se met à jour sur signal D-Bus et utilise les composants
   Plasma (thème, accent, HiDPI, accessibilité, animations désactivables). Coût marginal : il tourne dans `plasmashell`, déjà résident.
2. **Le tray SNI passe dans le démon** (option d, déjà décidée dans `REVUE-ARCHITECTURE-GLOBALE.md` §4 et #61, mais
   **pas faite** : le SNI est toujours dans `apihub-app`). Il sert de repli pour tout ce qui n'est pas Plasma
   (GNOME + AppIndicator, Sway/waybar, Hyprland, XFCE). Il **se retire de lui-même** quand le plasmoïde le réclame,
   pour éviter les doublons.
3. **On garde la fenêtre egui (option a), en fenêtre secondaire lancée à la demande** (diagnostic, historique
   long), avec trois corrections : instance unique, `app_id` Wayland, thème et accent du système (via le portail).
   Elle ne tourne plus en tâche de fond : −8 Mo RSS et un processus en moins à l'ouverture de session.
4. **L'option (c), une application Qt/Kirigami (cxx-qt ou qmetaobject), est rejetée.** Une fenêtre Kirigami minimale
   mesure 228 Mo de RSS, contre 93 Mo pour la fenêtre egui complète. Elle ajoute une chaîne C++/Qt/moc au build Rust
   et ne fait rien que le plasmoïde ne fasse déjà dans Plasma. On ne la reconsidère que si l'étape U6 échoue (§7).

## 2. Mesures (réelles, sur ce poste)

| Processus / scénario | RSS | PSS | CPU au repos | Remarque |
|---|---:|---:|---|---|
| `apple-kb-monitord` (démon, 12 threads) | 6,1 Mo | 3,8 Mo | 0 tick / 10 s | `systemctl` : 1,7 Mo cgroup, pic 3,4 Mo |
| `apihub-app` en mode tray seul (6 threads) | 8,0 Mo | 5,3 Mo | 0 tick / 10 s | ne fait que le SNI et le client D-Bus |
| `apihub-app --show` (fenêtre egui ouverte, 9 threads) | 92,7 Mo | 54,1 Mo | 1 tick / 10 s (0,1 %) | EGL NVIDIA + Mesa + libwayland chargés |
| `plasmawindowed com.agenceapi.devicehub` (widget seul hors panneau) | 211 Mo | 124 Mo | 18 ticks / 10 s | comprend tout le runtime Plasma ; dans `plasmashell` le coût marginal est faible (non isolable) |
| Fenêtre Kirigami minimale (`qml6`, 3 champs) | 228 Mo | 135 Mo | — | base de coût de l'option (c), sans aucune logique |
| `plasmashell` (référence) | 563 Mo | 516 Mo | — | déjà résident |

| Autre mesure | Valeur |
|---|---|
| Binaires | `apihub-app` 8,7 Mo (statique sauf libc/libm/libgcc), `apple-kb-monitord` 3,0 Mo |
| Crates dans `Cargo.lock` | 430 (eframe 0.29 → winit 0.30.13, glutin 0.32, wgpu 22, accesskit 0.16) |
| Enregistrement du tray après lancement | 4 ms |
| `qmllint` des 3 fichiers QML | 0 avertissement |
| Chargement du widget (`plasmawindowed`, journaux QML activés) | aucune erreur QML |
| Portail `org.freedesktop.appearance` | `color-scheme=1` (sombre), `accent-color=(0.85,0.35,0.24)`, `contrast=0`, `reduced-motion=0` : tout est lisible, egui peut donc suivre le système |
| UPower | expose déjà le clavier (`battery_hid_04odbo56ocao42oee_battery_71`, 99 %) : l'applet Batterie de Plasma affiche déjà le pourcentage seul. Notre interface doit apporter **plus** (tension, type de pile, RSSI, autonomie, diagnostic, plusieurs claviers). |

### 2.1 Défauts constatés pendant la revue

| # | Défaut | Preuve |
|---|---|---|
| D1 | **Icône de tray en double** : le bouton « ApiHub Settings… » du widget lance `apihub-app --show`, qui est une seconde instance complète et enregistre un second SNI | `RegisteredStatusNotifierItems` passe de `…-1788577` à `…-1788577` + `…-1893563` |
| D2 | **Infobulle et icône du tray jamais rafraîchies par le host** : aucun `NewToolTip`/`NewIcon`/`LayoutUpdated` n'est émis (seulement déclarés). Le host Plasma met ses propriétés en cache et les relit sur ces signaux : l'infobulle risque de rester figée (à confirmer visuellement) | `grep new_tool_tip\|new_icon\|layout_updated src/` → déclarations uniquement |
| D3 | Icône fixe (`apihub-scarab`) : le niveau de batterie n'apparaît qu'en survol | `tray.rs:76` |
| D4 | `GetGroupProperties` renvoie une liste vide : les hosts GTK (libdbusmenu, waybar) qui l'utilisent affichent un menu vide ou figé | `tray.rs:305` |
| D5 | Catégorie SNI `ApplicationStatus` au lieu de `Hardware` (Plasma range l'icône avec les applis, pas avec Bluetooth/Batterie) | `tray.rs:61` |
| D6 | Widget : interrogation toutes les 15 s alors que le démon émet `StateChanged` et `PropertiesChanged`, et que Plasma 6.7 fournit `DBus.Properties` et `DBus.SignalWatcher` | `main.qml` Timer ; `dbusplugin.qmltypes` |
| D7 | Widget : sentinelle RSSI incohérente : l'infobulle teste `rssi !== 0` alors que « inconnu » vaut 127 → affiche « RSSI 127dBm » ; un vrai 0 dBm (le démon renvoie 0 en ce moment) est masqué | `main.qml` toolTipSubText |
| D8 | Fenêtre egui : thème sombre forcé (`Visuals::dark()`), couleurs `from_rgb` en dur, pas d'`app_id` Wayland (icône générique dans la barre des tâches, `StartupWMClass=apihub` sans effet sous Wayland) | `main.rs:93`, `open_window()` |
| D9 | Widget : pas dans la zone de notification (`X-Plasma-NotificationArea` absent), double `Kirigami.Separator`, imports inutiles (`PlasmaExtras` utilisé seulement pour le titre, `P5` dans FullRepresentation) | `metadata.json`, `FullRepresentation.qml` |
| D10 | Mono-clavier partout : l'interface D-Bus n'expose qu'un seul appareil (`Battery`, `Model`, `Mac` au niveau racine) | `service.rs` |

D1 et D7 sont des bugs (issues `bug` séparées) ; les autres sont absorbés par les étapes du §7.

## 3. Comparaison des options

Notes : ++ très bon, + bon, = neutre, − faible, −− mauvais.

| Critère | (a) egui + SNI maison (actuel) | (b) plasmoïde principal + démon + petit SNI | (c) app Qt/Kirigami (cxx-qt) | (d) SNI dans le démon |
|---|---|---|---|---|
| Wayland / Plasma 6 : HiDPI fractionnaire | + (winit 0.30, fractional-scale-v1) | ++ (Plasma) | ++ | sans objet (rendu par le host) |
| Thème clair/sombre, accent, contraste | −− aujourd'hui (sombre forcé) ; + après U4 (portail) | ++ (Kirigami.Theme, ColorScheme) | ++ | ++ si icônes symboliques |
| Cohérence visuelle avec Plasma | − (widgets egui) | ++ | + (Kirigami en fenêtre) | ++ (rendu par le host) |
| Accessibilité (AT-SPI, Orca, clavier) | = (accesskit 0.16 compilé, rôles pauvres) | + (Accessible.* QML, navigation Plasma) | + | + (dbusmenu `accessible-desc`) |
| Autres bureaux (GNOME, Sway, Hyprland) | + (fenêtre partout) | −− (Plasma uniquement) | = (Qt partout, mal intégré à GNOME) | ++ (SNI standard) |
| Mémoire au repos (session) | 8 Mo permanents | 0 processus en plus (dans plasmashell) | ≥ 228 Mo à l'ouverture | ≈ +1 Mo dans le démon (estimé : un thread zbus + 2 objets) |
| Démarrage | 4 ms (tray), fenêtre < 1 s | instantané (applet chargée par plasmashell) | ~1 s (moteur QML) | 0 (déjà lancé) |
| Maintenabilité | = (Rust seul ; 430 crates pour la fenêtre) | + (QML déclaratif, 277 lignes actuelles) | −− (Rust + C++ + moc + CMake/cxx-build, API cxx-qt encore mouvante) | ++ (`tray.rs` est déjà en zbus pur et lit déjà un `akm_core::Watch`) |
| Risque | faible | faible : le widget existe et se charge sans erreur | élevé | moyen : un bug de tray ne doit pas tuer l'acquisition (thread isolé, `catch_unwind`) |
| Effort | — | M (2-3 j) | L (≥ 2 semaines) | S (1 j) |

**Lecture** : (b) et (d) sont complémentaires. (b) est la meilleure interface sous Plasma, (d) la meilleure
présence hors Plasma, et à elles deux elles couvrent tout pour un coût quasi nul. La fenêtre egui (a) reste utile
pour le diagnostic et pour les bureaux sans Plasma. Elle n'a pas besoin de tourner en permanence. (c) coûte cher et
ne rapporte rien qui ne soit déjà couvert.

### 3.1 Pourquoi pas un tray dans un binaire séparé (`apihub-tray`) ?

Il aurait l'avantage d'isoler les pannes, mais coûterait un processus de plus (≈ 5-8 Mo), un second client D-Bus et un second cycle de vie à
superviser. Le démon possède déjà la connexion de session, le `Watch` et la logique de ré-enregistrement auprès du
watcher (#38/#74). Le SNI n'utilise aucune bibliothèque graphique : il tourne sans `WAYLAND_DISPLAY`.
L'isolation se fait dans le démon : thread dédié, panique attrapée et relance, aucune écriture dans le `Watch`.

## 4. Architecture d'interface cible

```
                         apple-kb-monitord  (systemd --user, seul lecteur du clavier)
   ┌───────────────────────────────────────────────────────────────────────────────┐
   │ acquisition ─► Watch(Snapshot) ─┬─► com.agenceapi.AppleKbMonitor1 (session)   │
   │                                 │     + /devices/<mac>  (v3.2, ObjectManager) │
   │                                 │     + ClaimTray()/ReleaseTray()             │
   │                                 ├─► tray SNI + dbusmenu (thread isolé)        │
   │                                 │     visible si aucun client n'a « réclamé » │
   │                                 └─► BatteryProvider1, historique, notif.      │
   └──────────────┬────────────────────────────┬───────────────────────────┬───────┘
                  │ PropertiesChanged          │ SNI / dbusmenu            │ org.freedesktop.Application
                  ▼                            ▼                           ▼  (activation D-Bus)
   Plasma 6 : plasmoïde dans la zone      GNOME (AppIndicator), Sway/     apihub-app (egui), à la demande,
   de notification (Hardware),            waybar, Hyprland, XFCE :        instance unique, jeton
   chargé seul si le démon est là,        icône dynamique + menu          xdg-activation transmis
   appelle ClaimTray() → SNI retiré
```

Règle d'arbitrage du tray (sans dépendre de `XDG_CURRENT_DESKTOP`, que l'unité systemd n'a pas toujours) :

1. Le plasmoïde appelle `ClaimTray()` quand il se charge et quand le démon réapparaît.
2. Le démon note le nom unique de l'appelant et **libère** son nom `org.kde.StatusNotifierItem-<pid>-1` : le watcher retire l'item.
3. Quand ce nom unique disparaît (plasmashell tué ou redémarré, `NameOwnerChanged`), le démon reprend le nom et se ré-enregistre en moins de 2 s.
4. Option `tray = "auto" | "always" | "never"` dans `config.toml` (`auto` par défaut).

## 5. Le tray 2026 : spécification

### 5.1 Icône dynamique

On installe **notre propre jeu d'icônes symboliques** dans `hicolor/scalable/status/`. Les thèmes ne nomment pas les
icônes de la même façon (constaté sur ce poste) :

| Thème | Nom du niveau 50 % | Conséquence |
|---|---|---|
| Breeze 6.30 | `battery-050-symbolic` (pas de `battery-level-*`) | `battery-level-50-symbolic` se replie vers `battery` (perte du niveau) |
| Adwaita 50 | `battery-level-50-symbolic` (pas de `battery-050`) | l'inverse |
| Breeze | `input-keyboard-battery` existe, mais **sans niveaux** | inutilisable pour une jauge |

Jeu fourni (SVG 16×16 en grille de 22, contour d'un clavier avec une jauge de 5 segments sous les touches) :

```
apihub-kb-battery-{000,010,…,100}-symbolic.svg      11 niveaux, arrondi à la dizaine inférieure (jamais « 100 » sous 95 %)
apihub-kb-battery-{000,…,100}-charging-symbolic.svg Magic Keyboard en charge (statut noyau « Charging »)
apihub-kb-battery-caution-symbolic.svg              ≤ 10 %  (partie jauge en classe ColorScheme-NegativeText + error)
apihub-kb-disconnected-symbolic.svg                 appairé mais hors ligne (clavier barré)
apihub-kb-missing-symbolic.svg                      aucun clavier connu / démon sans source
```

- Coloration : `fill="currentColor"` avec `class="ColorScheme-Text"` et une feuille de style Breeze (`.ColorScheme-Text{color:#232629}`).
  Plasma recolore selon le thème. GTK (GNOME, waybar) recolore tout fichier `-symbolic`. Les classes `error`,
  `warning` et `success` portent l'état sur les deux.
- Choix de l'icône : seau de 10 %, avec une **hystérésis de 2 points** : à 49-51 %, l'icône ne bascule pas à chaque mesure.
  `NewIcon` n'est émis que si le nom change.
- Pas de pourcentage dessiné dans l'icône SNI. Il faudrait passer par `IconPixmap` (raster ARGB), ce qui casse le HiDPI et le
  thème. Le pourcentage exact va dans l'infobulle. Sous Plasma, le plasmoïde affiche un badge vectoriel optionnel (§5.3).

### 5.2 Propriétés SNI

| Propriété | Valeur |
|---|---|
| `Id` / `Title` | `apple-kb-monitor` / « Clavier Apple » (traduit) |
| `Category` | `Hardware` |
| `Status` | `Active` ; `NeedsAttention` à ≤ 10 % ou en cas de déconnexion inattendue depuis plus de 60 s ; `Passive` si l'option « masquer quand tout va bien » est cochée et que tout va bien (> 20 %, connecté) |
| `IconName` / `AttentionIconName` | §5.1 / `apihub-kb-battery-caution-symbolic` |
| `ToolTip` | titre `Apple Wireless Keyboard — 99 %` ; description en texte brut, une info par ligne : `2,90 V · alcaline`, `Signal excellent (−48 dBm)`, `Autonomie ≈ 41 j`, `Mis à jour il y a 12 s` |
| `ItemIsMenu` | `false` : clic gauche = action, clic droit = menu |
| `Activate` | Plasma : ouvre le popup du plasmoïde (s'il a réclamé le tray, le SNI n'existe pas). Ailleurs : active `apihub-app` (activation D-Bus, fenêtre unique) |
| `SecondaryActivate` (clic milieu) | `Refresh()` immédiat |
| `Scroll` | ignoré |
| `ProvideXdgActivationToken(token)` | extension KDE : le jeton est mémorisé puis passé à `org.freedesktop.Application.Activate({"activation-token": …})`, ce qui donne le focus à la fenêtre sous Wayland sans vol de focus |
| Signaux | `NewIcon`, `NewToolTip`, `NewStatus(status)` émis **seulement** quand la valeur change (corrige D2/D3) |

### 5.3 Menu (dbusmenu)

Un seul clavier :

```
┌──────────────────────────────────────────────┐
│ ⌨  Apple Wireless Keyboard (A1314)           │  en-tête, désactivé, icon-name input-keyboard
│    ▰▰▰▰▰▰▰▰▰▱  99 %   2,90 V · alcaline       │  désactivé
│    📶 Signal excellent (−48 dBm)              │  désactivé ; « indisponible » si 127
│    ⏱  Autonomie estimée ≈ 41 jours            │  désactivé ; masqué sans estimation
│    ⇪  Verr. Maj active                        │  seulement si active
├──────────────────────────────────────────────┤
│    Ouvrir le tableau de bord…                 │  apihub-app (ou popup Plasma)
│    Diagnostic…                                │  apihub-app --tab diag
│    Reconnecter                                │  visible si hors ligne : org.bluez Device1.Connect
│    Paramètres Bluetooth…                      │  kcmshell6 kcm_bluetooth | gnome-control-center bluetooth
├──────────────────────────────────────────────┤
│  ☐ Masquer l'icône quand tout va bien         │  toggle-type checkmark → Status Passive
└──────────────────────────────────────────────┘
```

Plusieurs claviers (v3.2) : une ligne par clavier, dont un sous-menu reprend le bloc d'infos et les actions de l'appareil :

```
┌──────────────────────────────────────────────┐
│ ⌨  Magic Keyboard (bureau)     82 %   ▸      │
│ ⌨  Apple Wireless Keyboard     11 % ⚠ ▸      │  disposition « alert »
│ ⌨  Magic Keyboard (salon)   hors ligne ▸      │
├──────────────────────────────────────────────┤
│    Ouvrir le tableau de bord…                 │
│  ☐ Masquer l'icône quand tout va bien         │
└──────────────────────────────────────────────┘
```

Icône agrégée = le clavier **connecté** le plus faible ; infobulle = une ligne par clavier.

Règles dbusmenu :
- Pas de « Quitter » : le démon est un service. On peut masquer l'icône, pas couper l'acquisition depuis un menu.
- Toutes les entrées ont des `id` stables, `label` avec `_` mnémonique, `icon-name` symbolique, et `accessible-desc` sur les lignes d'info (par exemple « Batterie 99 pour cent, 2,90 volts »).
- `disposition` : `alert` à ≤ 10 %, `warning` à ≤ 20 %.
- `GetLayout`, `GetGroupProperties` et `GetProperty` **implémentés tous les trois** (corrige D4). `LayoutUpdated(rev, 0)` est émis quand la structure change (clavier ajouté ou retiré, connecté ou déconnecté) ; `ItemsPropertiesUpdated` quand seuls des libellés changent.
- Libellés FR/EN selon `LANG` (table statique, sans gettext dans le démon).

### 5.4 Comportement par environnement

| Environnement | Rendu | Notes |
|---|---|---|
| Plasma 6 (cas nominal) | plasmoïde dans la zone de notification, catégorie Matériel, à côté de Bluetooth et Batterie | SNI retiré par `ClaimTray()`. Le popup est le tableau de bord. |
| Plasma 6, plasmoïde désactivé par l'utilisateur | SNI du démon | l'arbitrage le fait revenir de lui-même |
| GNOME 47+ avec extension AppIndicator | SNI : icône symbolique recolorée, menu dbusmenu ; l'infobulle n'est pas affichée par l'extension | tout ce qui compte est aussi dans le menu |
| GNOME sans extension | aucun watcher, donc pas d'icône (silencieux) | UPower montre déjà la batterie dans Paramètres > Énergie ; notification à ≤ 10 % envoyée par le démon |
| Sway / Hyprland + waybar | module `tray` de waybar (SNI + dbusmenu GTK) | `GetGroupProperties` indispensable ; module `custom` piloté par événements (`--waybar --follow`, une ligne JSON par `StateChanged`) : voir #22 |
| Session sans bus de session / SSH | pas de SNI, aucune erreur bloquante | le démon reste headless |

### 5.5 Animations

- Pas d'animation par alternance d'icônes dans le SNI : elle coûterait du CPU et du bus pour rien, et irait contre `reduced-motion`. La charge est signalée par l'icône `-charging`. La connexion en cours (BlueZ `Connected` false → true) passe par une icône `disconnected` puis le niveau réel.
- Dans le plasmoïde, les transitions utilisent `Kirigami.Units.longDuration`, que Plasma met à 0 quand l'utilisateur coupe les animations. Prévus : remplissage de la jauge (`NumberAnimation`), pulsation lente du segment en charge, fondu « hors ligne ».
- egui : `ctx.style().animation_time` est mis à 0 si le portail renvoie `reduced-motion=1`.

### 5.6 Accessibilité

- Plasmoïde : `Accessible.name` / `Accessible.description` sur l'icône compacte (« Clavier Apple, batterie 99 pour cent, connecté »), `Accessible.role: Accessible.ProgressBar` sur la jauge, focus clavier dans le popup, contrastes tirés de `Kirigami.Theme` (rien en dur). L'état n'est jamais porté par la seule couleur : chaque couleur est doublée d'un texte (« faible », « hors ligne »).
- SNI : `accessible-desc` sur les entrées, infobulle en texte brut lisible par Orca.
- egui : libellés accesskit explicites sur les valeurs (`Label` avec `widget_info`), ordre de tabulation, aucune information portée par un émoji seul (aujourd'hui ✅/❌ dans Diag).

## 6. Maquettes

Les valeurs affichées sont illustratives (sauf 99 %, 2,90 V, ADC 900 et la MAC, relevés sur `GetState` pendant la revue).

### 6.1 Plasmoïde : représentation compacte (zone de notification, 22 px)

```
 ┌────┐   ┌────┐   ┌────┐   ┌────┐
 │⌨▰▰▰│   │⌨▰▱▱│   │⌨▱▱▱│   │⌨ ⃠ │
 │  99│   │  42│   │ 9 ⚠│   │    │
 └────┘   └────┘   └────┘   └────┘
  normal   moyen    critique  hors ligne
 (badge % optionnel, désactivé par défaut comme dans l'applet Batterie de Plasma)
```

### 6.2 Plasmoïde : popup (PlasmaExtras.Representation, 20 gridUnits)

```
┌────────────────────────────────────────────────────┐
│ Claviers Apple                          ⟳   ⚙      │  en-tête : Refresh(), paramètres
├────────────────────────────────────────────────────┤
│ ⌨ Apple Wireless Keyboard (A1314)     ● Connecté   │
│   99 %  ▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▱                        │
│   2,90 V · alcaline        Autonomie ≈ 41 j        │
│   Signal  ▂▄▆█  −48 dBm    Firmware 0x0050         │
│   ╭─ 30 jours ───────────────────────────────╮     │  KQuickCharts LineChart, History(since)
│   │▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔╲▁▁▁▁       │     │
│   ╰──────────────────────────────────────────╯     │
├────────────────────────────────────────────────────┤
│ ⌨ Magic Keyboard (salon)          ○ Hors ligne     │  v3.2 : une carte par clavier
│   dernière valeur 64 % · il y a 2 h   [Reconnecter]│
├────────────────────────────────────────────────────┤
│ [ Diagnostic… ]                 [ Tableau de bord ]│
└────────────────────────────────────────────────────┘
 Démon absent : PlaceholderMessage « Le service apple-kb-monitord ne tourne pas »
                + bouton « Démarrer » (systemctl --user start, par activation D-Bus)
```

### 6.3 Fenêtre egui (secondaire, à la demande)

```
┌ Apple Keyboard Monitor ─────────────────────────── _ □ × ┐   app_id com.agenceapi.ApiHub
│  [Clavier]  [Historique]  [Diagnostic]                    │   couleurs et accent du portail
├───────────────────────────────────────────────────────────┤
│  Batterie                         Radio                   │
│  99 %  ▰▰▰▰▰▰▰▰▰▱  alcaline       RSSI −48 dBm  ▂▄▆█       │
│  Tension 2,903 V   ADC 900        TX 8 dBm               │
│  Autonomie ≈ 41 j                 Lien 15 ms / 2,0 s      │
│  Appareil : A1314 · ISO · 04:DB:56:CA:42:EE · fw 0x0050   │
└───────────────────────────────────────────────────────────┘
```

## 7. Plan de migration incrémental

Chaque étape se livre seule, rien ne régresse si l'étape suivante n'arrive pas, et chaque étape a des critères de
sortie mesurables. Les numéros d'issues sont en §8.

### U1 — Tray SNI dans le démon et `apihub-app` en instance unique (v3.1, effort S)

- `tray.rs` passe dans `apple-kb-monitord` (thread isolé, `catch_unwind` + relance, lecture seule du `Watch`) ; `apihub-app` ne crée plus de SNI.
- `apihub-app` devient `DBusActivatable=true` (`org.freedesktop.Application`, nom `com.agenceapi.ApiHub`) : un deuxième lancement active la fenêtre existante (corrige D1).
- `Activate` du SNI → `org.freedesktop.Application.Activate`, avec le jeton de `ProvideXdgActivationToken`.
- Plus d'autostart d'`apihub-app` (#21 : le démon suffit).
- **Sortie** : `RegisteredStatusNotifierItems` contient exactement 1 item après 3 lancements de `apihub-app --show` ; aucun `apihub-app` au repos (`pgrep -x apihub-app` vide) ; RSS du démon ≤ 8 Mo ; icône revenue < 2 s après `plasmashell --replace` ; démon lancé sans `WAYLAND_DISPLAY` → aucune erreur fatale.

### U2 — Tray dynamique conforme 2026 (v3.1, effort M)

- Jeu d'icônes `apihub-kb-*-symbolic` (§5.1), hystérésis, `Category=Hardware`, `Status` Active/Passive/NeedsAttention.
- Signaux `NewIcon`/`NewToolTip`/`NewStatus`/`LayoutUpdated` émis sur changement réel (corrige D2, D3, D5).
- Menu §5.3, `GetGroupProperties`/`GetProperty` complets (D4), `accessible-desc`, FR/EN.
- **Sortie** : `busctl --user monitor` montre au plus 1 `NewIcon` par changement de seau sur 1 h ; 0 tick CPU sur 60 s au repos ; captures Breeze clair, Breeze sombre et Adwaita jointes à l'issue ; menu identique sous Plasma et sous waybar (Sway imbriqué) ; test unitaire du choix d'icône (seaux + hystérésis) et du `GetGroupProperties` non vide.

### U3 — Plasmoïde dans la zone de notification, piloté par événements (v3.1, effort M)

- `metadata.json` : `X-Plasma-NotificationArea: true`, `X-Plasma-NotificationAreaCategory: Hardware`, `X-Plasma-DBusActivationService: com.agenceapi.AppleKbMonitor1`.
- `DBus.Properties` / `DBus.SignalWatcher` sur `StateChanged` à la place du `Timer` de 15 s (D6) ; `GetState` seulement à l'apparition du service.
- `ClaimTray()`/`ReleaseTray()` côté démon et côté widget (arbitrage §4).
- Compacte : même jeu d'icônes, badge % en option. Popup §6.2 avec `PlasmaExtras.Representation`, sparkline KQuickCharts via `History(since)`, `PlaceholderMessage` si le démon est absent.
- Correction de la sentinelle RSSI (D7), nettoyage (D9), `i18n()` FR/EN, `Accessible.*` (§5.6).
- **Sortie** : `grep -c Timer contents/ui/*.qml` = 0 ; `qmllint` 0 avertissement ; l'applet apparaît dans la zone de notification au `systemctl --user start apple-kb-monitord` et disparaît à l'arrêt ; aucune icône SNI en double tant que le plasmoïde est chargé, retour du SNI < 2 s après kill de plasmashell ; Orca lit « Clavier Apple, batterie N pour cent » ; latence d'affichage d'un changement < 1 s (journal du démon contre `console.log` du widget).

### U4 — Fenêtre egui intégrée au bureau (v3.1, effort S)

- `with_app_id("com.agenceapi.ApiHub")`, `.desktop` renommé `com.agenceapi.ApiHub.desktop` (icône correcte dans le gestionnaire de tâches).
- Thème depuis le portail : `color-scheme` (clair/sombre en direct, `SettingChanged`), `accent-color` pour la sélection et les jauges, `contrast=1` pour les contours renforcés, `reduced-motion`. Suppression de `Visuals::dark()` et de toutes les couleurs `from_rgb` en dur (palette sémantique positive/neutre/négative).
- accesskit : libellés sur les valeurs ; Diag sans émoji porteur de sens.
- **Sortie** : basculer Plasma clair/sombre met la fenêtre à jour sans relance ; `grep -c from_rgb src/main.rs` = 0 ; arbre AT-SPI (Accerciser) qui expose onglets et valeurs ; icône de l'appli visible dans la barre des tâches Wayland.

### U5 — Plusieurs claviers (v3.2, effort L)

- Démon : un objet `/com/agenceapi/AppleKbMonitor1/devices/<mac>` par clavier (interface `…Device`), `org.freedesktop.DBus.ObjectManager` ; les propriétés racine restent celles du clavier « principal » (compatibilité ascendante).
- Tray : icône agrégée et sous-menus (§5.3) ; plasmoïde : une carte par clavier ; egui : sélecteur d'appareil.
- **Sortie** : test `dbus_session.rs` avec 2 sources simulées (ajout, retrait, reconnexion) ; clients v3.1 inchangés et fonctionnels ; démonstration avec 2 claviers réels ou un clavier réel + une source simulée.

### U6 — Diagnostic et historique dans le plasmoïde, fenêtre egui optionnelle (v3.2, effort M)

- Pages « Historique » (KQuickCharts, 7/30/90 j) et « Diagnostic » (méthode D-Bus `Diagnose()` du démon, qui reprend les vérifications de l'onglet Diag) dans le popup.
- `apihub-app` passe derrière une feature Cargo `gui` (activée par défaut dans le PKGBUILD) et reste l'interface des bureaux sans Plasma.
- **Point de décision (c)** : on reconsidère Kirigami seulement si, après U4, la fenêtre egui ne passe pas les critères d'accessibilité ou de thème, ou si plus de 30 % des retours portent sur son aspect. Sinon (c) est classée définitivement.
- **Sortie** : sous Plasma, un utilisateur fait tout (consulter, diagnostiquer, reconnecter) sans ouvrir `apihub-app` ; `Diagnose()` couvert par un test sans matériel.

Hors périmètre de cette revue, mais en dépendance : #22 (sortie waybar par événements, CLI Rust), #68 (LED et F1/F2).

## 8. Suivi Gitea

| Étape | Issue | Milestone | Recoupe (feuille de route F, créée en parallèle) |
|---|---|---|---|
| U1 tray dans le démon, instance unique | [#115](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/115) | v3.1 | #61, #62, #21 |
| U2 tray dynamique | [#116](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/116) | v3.1 | #87 (F06), #86 (F05), #104 (F24), #114 (F34) |
| U3 plasmoïde dans la zone de notification | [#117](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/117) | v3.1 | #97 (F17), #114 |
| U4 fenêtre egui intégrée | [#118](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/118) | v3.1 | #114 |
| U5 plusieurs claviers | [#119](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/119) | v3.2 | #93 (F12), #94 (F13) |
| U6 historique et diagnostic dans le plasmoïde | [#120](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/120) | v3.2 | #96 (F15), #106 (F26), #111 (F31) |
| D1 tray en double (bug) | [#121](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/121) | v3.1 | corrigé par U1 |
| D7 sentinelle RSSI du widget (bug) | [#122](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/122) | v3.1 | absorbable par U3 |

Les issues U portent le découpage architectural (où vit le code, dans quel ordre). Les issues F portent le détail fonctionnel.
