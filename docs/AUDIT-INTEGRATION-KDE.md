# Audit d'intégration KDE Plasma 6 — clavier Apple A1314 « alex »

Audit en **lecture seule** du 2026-10-01, sur la session réelle du poste (Manjaro, Wayland).
Base : `main` @ 5fe1ae5, paquet installé `apple-kb-monitor 3.1.0-10` (installé le 01/10 à 17:41).
Aucune action n'a été déclenchée sur la session : ni notification, ni `SetAlias`, ni `Refresh`, ni « Oublier »,
ni ouverture/fermeture de fenêtre, ni écriture de configuration KDE. Lectures uniquement :
`busctl get-property|introspect`, `kreadconfig6`, `upower -i`, sysfs, `/proc/bus/input/devices`,
journal, QML de Plasma extrait des greffons installés (ressources Qt compressées zstd, décompressées
hors ligne), et code source du dépôt.

Légende : **[mesuré]** = observé sur la session avec la commande citée ; **[code]** = lu dans les
sources ; **[non mesuré]** = connaissance amont non vérifiable sans action interdite.

## 0. Environnement [mesuré]

| Élément | Valeur | Commande |
|---|---|---|
| Plasma / KWin | 6.7.5 / 6.7.5, Wayland | `plasmashell --version`, `kwin_wayland --version` |
| bluedevil, powerdevil, kglobalacceld | 6.7.5 | `pacman -Q` |
| BlueZ / UPower | 5.87 / 1.91.3 | `bluetoothctl --version`, `upower --version` |
| Démon | `apple-kb-monitord` 3.1.0, unité user `enabled` + `active`, `WantedBy=default.target` | `systemctl --user is-enabled/show` |
| Fenêtre | `apihub-app` ouverte (pid 2498516, enfant de `systemd --user` = activée par D-Bus) | `busctl --user list` |
| Linger | `Linger=yes` | `loginctl show-user` |
| Disposition | `kxkbrc Model=applealu_iso`, `hid_apple fnmode=1` | `kreadconfig6`, `/sys/module/hid_apple/parameters` |
| Thème | jeu de couleurs `Bart` (sombre), portail `color-scheme=1`, accent (0.85,0.35,0.24), échelle 1 | `kreadconfig6`, portail `Settings.ReadOne`, `kscreen-doctor -o` |

## 1. Applet Bluetooth et module Bluetooth (bluedevil)

### Constats

| Point | Valeur affichée | Source | Preuve |
|---|---|---|---|
| Nom | **alex** | `Device1.Alias` (bluez-qt `name` = Alias) | `busctl get-property … Device1 Alias` → `"alex"` |
| Détail « Nom distant » | **Clavier de maria #1** (ligne affichée car Name ≠ Alias) | `Device1.Name` | QML `DeviceItem.qml` : `if (model.Name !== model.RemoteName)` |
| Icône | `input-keyboard` | `Device1.Icon` | `busctl` |
| Batterie | **96 %** (sous-titre « 96 % Batterie » et infobulle « alex connecté · 96 % Batterie ») | **`org.bluez.Battery1` publié par notre fournisseur**, `Source="apple-kb-monitord (kernel power_supply)"` | `busctl introspect org.bluez /org/bluez/hci0/dev_04_DB_56_CA_42_EE` ; QML `model.Battery.percentage` |
| État | connecté, appairé, de confiance | `Device1` | `busctl` |
| Actions | Connecter/Déconnecter, **Oublier** (dialogue `ForgetDeviceDialog` → `Adapter1.RemoveDevice`) ; envoi de fichier/parcourir masqués (pas d'OBEX) | QML extrait | — |

* BlueZ ne crée `Battery1` que via un fournisseur (ou GATT BAS) : la batterie noyau HID n'y figure pas
  d'elle-même. **Sans le démon, l'applet Bluetooth n'affiche aucune batterie** (comportement attendu,
  documenté dans `bluez.rs`).
* Cohérence des pourcentages [mesuré] : `Battery1` = 96, `power_supply/capacity` = 96, UPower = 96 %,
  propriété D-Bus `Battery` du démon = 96. L'infobulle de notre icône ajoute « Affichage Apple : 100 % »
  (courbe macOS, F41 #213) : c'est la **seule** surface où 100 % apparaît ; KDE affiche partout 96 %.
  Pas de conflit de source : notre fournisseur recopie la valeur noyau.

### Écart 1.a — « Oublier » n'est pas détecté par le démon — **bug, moyenne** → #252

[code] `apple-kb-monitord/src/repair.rs::add_rules()` ne s'abonne qu'à `PropertiesChanged`
(Device1/Adapter1), `Device1.Disconnected` et `NameOwnerChanged(org.bluez)`. **Aucun abonnement à
`ObjectManager.InterfacesRemoved/InterfacesAdded`.** La purge « silencieuse » des claviers oubliés
(`sync()`, commentaire « Removed from BlueZ (forgotten by the user): stop following, silently ») n'est
exécutée que sur une ré-énumération, c'est-à-dire au redémarrage de `bluetoothd` ou à l'apparition d'un
nouvel appareil appairé.

Conséquences quand l'utilisateur clique « Oublier » dans l'applet ou dans Paramètres › Bluetooth :

1. `RemoveDevice` déconnecte d'abord le clavier (`Disconnected`, raison locale) → `Recovery` ouvre un
   épisode « Quiet » et, passé `DORMANT_GRACE`, planifie des `Connect` vers un objet qui n'existe plus ;
2. si BlueZ publie `Paired=false` avant de retirer l'objet, `on_bond_lost()` passe en `AuthFailed` et
   envoie la notification « ré-appairage nécessaire » — exactement ce que le test
   `forgotten_keyboard_is_dropped_silently_and_unpaired_one_asks_for_repair` modélise (une note émise)
   [non mesuré sur BlueZ 5.87 : demanderait un vrai « Oublier »] ;
3. `Link.Status()` et le tray continuent d'exposer le clavier jusqu'au prochain redémarrage de BlueZ.

Correctif (démon) : ajouter une règle `InterfacesRemoved` (sender `org.bluez`, arg0 path namespace
`/org/bluez`) → `KMsg::Removed(path)` qui retire l'entrée sans note ; ignorer `Paired=false` suivi
d'un retrait dans une fenêtre courte (≈ 1 s) ; `InterfacesAdded` → `resync`. Test sur bus privé
(`dbus-daemon --session` + faux `org.bluez`) : Paired=false puis InterfacesRemoved ⇒ zéro notification,
zéro `Connect`.

## 2. Applet « Batterie et luminosité » et PowerDevil

### Constats [mesuré]

* UPower ne publie **qu'un** appareil pour ce clavier : `battery_hid_04odbo56ocao42oee_battery_71`
  (native-path `hid-04:db:56:ca:42:ee-battery-71`, type **keyboard**, `model: alex`, 96 %,
  `discharging`). Aucun doublon `bluez` : UPower masque la batterie BlueZ de même numéro de série,
  comme prévu dans `bluez.rs`. → **Le clavier figure dans l'applet Batterie avec 96 % et le nom
  « alex »**, source = noyau (pas notre fournisseur).
* Le poste n'a pas de batterie interne : l'applet est en `PassiveStatus` (rangé dans la flèche de la
  zone de notification) ; le clavier apparaît dans la liste de son popup (QML `z4.qml`
  `Plasmoid.status`). Une deuxième instance `org.kde.plasma.battery` existe sur le panneau 120
  (choix utilisateur, hors périmètre).
* `powerdevilrc` : aucune clé `PeripheralBatteryLowLevel` → valeur par défaut amont **10 %**
  [non mesuré : défaut compilé dans `libpowerdevilcore`, symbole
  `defaultPeripheralBatteryLowLevelValue_helper` présent]. `powerdevil.notifyrc` contient
  l'événement `lowperipheralbattery` et la chaîne « Keyboard Battery Low (%1% Remaining) ».

### Écart 2.a — Double alerte batterie faible (PowerDevil + démon) — **amélioration, moyenne** → #254

Notre démon alerte à **30 / 15 / 5 %** (journal : `alerts: Some([30, 15, 5])`), PowerDevil alerte à
**10 %** sur le même appareil UPower (type keyboard). Sur une décharge complète l'utilisateur reçoit
4 notifications de deux applications différentes (« apple-kb-monitord » et « Gestion de l'énergie »),
dont une PowerDevil entre nos paliers 15 et 5. Correctif proposé (démon + KCM #250) : lire
`PeripheralBatteryLowLevel` (défaut 10) et soit ne pas émettre de palier ≤ ce seuil (PowerDevil le
couvre), soit proposer dans le KCM « Laisser KDE gérer l'alerte basse » ; documenter dans
`docs/CONFIGURATION.md`.

### Écart 2.b — Nom différent selon la surface KDE — **faible** → commentaire #248

[mesuré] Applet Bluetooth et applet Batterie : « alex » (Alias). KWin
`/org/kde/KWin/InputDevice/event28` `name`, `power_supply/model_name` et `/proc/bus/input/devices` :
« Clavier de maria #1 » (nom HID/firmware). Paramètres › Clavier / Périphériques d'entrée affichent donc
l'ancien nom. Pour #248 (écriture du nom dans le firmware) : KWin indexe les réglages par périphérique
(`kcminputrc [Libinput][vendor][product][nom]`) sur ce nom ; aujourd'hui aucune section n'existe
[mesuré], mais un renommage firmware ferait perdre de tels réglages s'ils étaient créés plus tard.
UPower prend le nom à l'ajout de l'appareil : après un `SetAlias`, l'applet Batterie garde l'ancien nom
jusqu'à la reconnexion [non mesuré : `SetAlias` interdit pendant l'audit].

## 3. Zone de notification

### Constats [mesuré]

* `StatusNotifierWatcher.RegisteredStatusNotifierItems` contient
  `org.kde.StatusNotifierItem-2461612-1` (pid du démon) : `Id=apple-kb-monitor`, `Category=Hardware`,
  `Title=Clavier Apple`, `IconName=apihub-kb-battery-090-symbolic`,
  `AttentionIconName=apihub-kb-battery-caution-symbolic`, infobulle FR, `ProvideXdgActivationToken`
  implémenté. Icônes installées dans `hicolor/scalable/status`, SVG à classes `ColorScheme-Text` →
  recoloriées par le thème sombre.
* `apihub-app` n'a **pas** de SNI propre (tray hérité inactif car le démon en a un) → **une seule
  icône** aujourd'hui.
* Widget `com.agenceapi.devicehub` installé mais **ajouté nulle part** : absent de `extraItems` et de
  `knownItems` de la zone de notification et de tout panneau
  (`plasma-org.kde.plasma.desktop-appletsrc`). `Tray.Mode="auto"`, `Tray.Visible=true`.

### Écart 3.a — Deux icônes quand le widget est ajouté en cours de session — **bug, moyenne** → #253

[code] `tray.rs` : `plasma_widget = plasmashell && widget_enabled()` n'est évalué qu'**au démarrage du
thread** et quand `org.kde.plasmashell` change de propriétaire. `widget_enabled()` lit
`extraItems=` une fois ; aucune surveillance du fichier. Le widget n'appelle **jamais**
`Tray.ClaimTray()` (aucune occurrence dans `plasma/`). Donc :

1. l'utilisateur coche « ApiHub » dans Configurer la zone de notification › Entrées → le widget apparaît,
   le SNI du démon reste : **deux icônes** jusqu'au redémarrage de plasmashell ou du démon ;
2. il le décoche → plus aucune icône jusqu'au prochain redémarrage si le démon s'était retiré ;
3. widget posé sur un panneau hors zone de notification (`plugin=com.agenceapi.devicehub` sans
   `extraItems`) → deux icônes **par conception** (test `widget_detection`).

Correctif : (a) widget : `ClaimTray()` dans `Component.onCompleted` et `ReleaseTray()` dans
`Component.onDestruction` (le démon libère déjà la réclamation quand le nom unique disparaît, à
vérifier) — c'est le mécanisme prévu par #117 ; (b) démon : surveiller
`plasma-org.kde.plasma.desktop-appletsrc` (inotify) ou s'abonner au signal `KConfigWatcher`
(`/kconfig/plasma-org.kde.plasma.desktop-appletsrc` `ConfigChanged`) pour réévaluer.

## 4. Raccourcis globaux (kglobalacceld)

[mesuré] Capacités evdev du clavier (`/proc/bus/input/devices`, event28) : BRIGHTNESSDOWN/UP, SCALE,
DASHBOARD, EJECTCD, médias, volume, FN présents. Clavier xkb (`xkbcli compile-keymap`) : `<I128>`
XF86LaunchA, `<I212>` XF86LaunchB, `<I169>` XF86Eject, `<I232>/<I233>` XF86MonBrightness.
`kglobalshortcutsrc` :

| Touche Apple | Qt | Action KDE liée | État |
|---|---|---|---|
| F1/F2 (luminosité) | Monitor Brightness Down/Up | PowerDevil « Diminuer/Augmenter la luminosité de l'écran » ; `display0` = LG 34GK950F (DDC, externe) | OK (confirmé par le gérant, #247) |
| F3 (Exposé) | Launch (C) | KWin `ExposeAll` (défaut KWin pensé pour Apple) | OK |
| F4 (Dashboard) | Launch (D) | **aucune** | écart déjà documenté (`docs/TOUCHES.md`, correctif `akmctl keymap kde-apply`) **non appliqué** → commentaire #247 |
| ⏏ Éjecter | Eject | **aucune** | idem → commentaire #247 |
| F7-F12 médias/volume | Media*, Volume* | `mediacontrol`, `kmix` | OK |

Aucun composant kglobalaccel propre au projet (F19 #99 ouvert). `keyd` : `failed (core-dump)`
depuis 12:35 et `/etc/keyd/` vide → déjà #246 ; aucun conflit de raccourci mesuré.

## 5. Démarrage de session, activation D-Bus, fenêtre Wayland

[mesuré] Démon : unité user `enabled`, `Type=dbus`, `BusName=com.agenceapi.AppleKbMonitor1`, fichier
d'activation D-Bus `SystemdService=` → une seule instance, démarrée à `default.target` (avec
`Linger=yes` : dès le démarrage du poste, avant la connexion). Pas d'autostart de la fenêtre
(`~/.config/autostart`, `/etc/xdg/autostart` : rien) — conforme au modèle « démon + fenêtre à la
demande » ; #21 (autostart d'`apihub-app`) est rendu caduc par #61.

Fenêtre (lecture `org.kde.KWin /KWin getWindowInfo` via le runner fenêtres, sans activation) :
`caption="Apple Keyboard Monitor"`, `resourceClass=com.agenceapi.AppleKbMonitor`,
`desktopFile=com.agenceapi.AppleKbMonitor`, icône résolue `apihub-scarab`, `skipTaskbar=false` →
**app_id, icône de barre des tâches et regroupement corrects.** `.desktop` : `StartupWMClass` =
app_id, `DBusActivatable=true` + service `com.agenceapi.AppleKbMonitor.service` présent ; le jeton
d'activation (`activation-token`) est transmis (`instance.rs`). Restauration de session : client
Wayland natif, non restauré par ksmserver (sans objet).

## 6. Veille, arrêt, verrouillage, changement d'utilisateur

[mesuré] `systemd-inhibit --list` :

| WHO | WHAT | MODE | WHY |
|---|---|---|---|
| PowerDevil | handle-power-key:…:handle-lid-switch | block | KDE handles power events |
| apple-kb-monitord | shutdown | delay | Tell the keyboard the computer is shutting down (WillShutdown) |
| apple-kb-monitord | sleep | delay | Pause keyboard reads before the Bluetooth link goes down |
| kwin_wayland | sleep | delay | Ensuring that the screen gets locked before going to sleep |

Les verrous `delay` ne s'affichent pas dans le dialogue de déconnexion de Plasma (seuls les `block`
le sont) : aucune gêne utilisateur. `apple-kb-monitor-shutdown.service` `enabled`
(`ExecStop=akmctl shutdown-notify --only-if-stopping`, `TimeoutStopSec=6`). Le verrouillage d'écran
n'a aucun effet sur le démon (pas de dépendance à la session graphique) — attendu.

hidraw : `/dev/hidraw7` (`0005:05AC:0256`) `crw-rw---- root:root` + ACL `user:adminapi:rw-` (uaccess).
Avec `Linger=yes`, le démon tourne aussi **hors session active** (avant connexion, après
déconnexion, autre utilisateur au premier plan) : l'ACL uaccess suit la session active, les lectures
vendeur deviennent alors impossibles et seule la batterie sysfs reste — à vérifier lors d'un test
d'ouverture de session (non mesuré, pas d'action sur la session). Le volet sécurité est #204.

## 7. Thème, accent, échelle, langue

* Fenêtre egui : suit `org.freedesktop.appearance color-scheme` et `accent-color` du portail, à chaud
  (`apply_appearance`) [code] ; portail = sombre + accent mesurés. OK.
* Widget : couleurs `Kirigami.Theme` uniquement, aucune couleur en dur [code]. OK.
* Tray : icônes symboliques recoloriées. OK. Échelle 1 seulement disponible : fractionnaire non testé.
* **Langue — écart (déjà #114, #249) → commentaire #114** [mesuré/code] : `LANG=fr_FR.UTF-8`.
  Infobulle et menu du tray : **français** ; notifications du démon (« Battery at …% — charge soon »),
  widget (`i18n()` sans `.mo` `plasma_applet_com.agenceapi.devicehub`), titre de fenêtre
  « Apple Keyboard Monitor », `.desktop` (aucun `Name[fr]`), `metadata.json` du widget (Name
  « ApiHub », description), raisons des verrous logind : **anglais**. Cinq surfaces sur six en anglais.

## 8. Notifications (développement en cours, #249) → commentaire #249

[code] `notify.rs` : `app_name="apple-kb-monitord"`, **aucun hint `desktop-entry`**, aucun
`x-kde-appname`/`x-kde-eventId`, pas de `.notifyrc`, `replaces_id=0` (les alertes s'empilent),
`send()` = urgence **Critical** par défaut (ignore Ne pas déranger). Dans Plasma : en-tête « apple-kb-monitord »
sans icône d'application, absent de Paramètres › Notifications › Applications, non réglable. Tout est
dans le périmètre de #249 ; s'y ajoute le chevauchement PowerDevil (#254).

## 9. KRunner, Plasma Vault, autres

KRunner : #98 ouvert ; le runner fenêtres de KWin trouve déjà la fenêtre. Plasma Vault, Activités,
KDE Connect : sans objet.

## Synthèse des écarts

| # | Écart | Gravité | Qui / fichier | Issue |
|---|---|---|---|---|
| 1.a | « Oublier » (bluedevil) non détecté : pas d'`InterfacesRemoved`, reconnexions vers un clavier oublié, notification « ré-appairage » possible | moyenne (bug) | démon, `apple-kb-monitord/src/repair.rs` | #252 |
| 3.a | Deux icônes quand le widget est ajouté en cours de session ; widget sans `ClaimTray` | moyenne (bug) | widget `main.qml` + démon `tray.rs` | #253 |
| 2.a | Alerte batterie basse en double avec PowerDevil (10 % par défaut) | moyenne | démon `alerts`/`notify.rs`, KCM #250 | #254 |
| 8 | Notifications sans `desktop-entry`, sans notifyrc, Critical par défaut | moyenne | #249 en cours | commentaire #249 |
| 7 | Langue : 5 surfaces sur 6 en anglais sous `fr_FR` | faible | #114 | commentaire #114 |
| 4 | F4 (Launch (D)) et Éjecter liés à rien ; `kde-apply` non appliqué | faible | #247 | commentaire #247 |
| 2.b | Nom « alex » (BT/Batterie) vs « Clavier de maria #1 » (KWin/Paramètres Clavier) | faible | #248 | commentaire #248 |
| 6 | Linger + uaccess : démon hors session sans hidraw (à vérifier) | faible | doc | — |
