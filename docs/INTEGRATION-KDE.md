# Intégration KDE Plasma 6 : correctifs de l'audit du 2026-10-01

Suite de `docs/AUDIT-INTEGRATION-KDE.md` (branche `audit/integration-kde`). Chaque
section dit ce que fait le démon, comment le vérifier et ce qui reste volontairement
non fait.

## 1. « Oublier » depuis Plasma (#252)

Le clavier supprimé de l'extérieur (Paramètres › Bluetooth › Oublier, applet
Bluetooth, `bluetoothctl remove`) disparaît du démon **aussitôt** :

* le démon écoute `org.freedesktop.DBus.ObjectManager.InterfacesRemoved` /
  `InterfacesAdded` de BlueZ (`repair.rs`, `add_rules`) ;
* sur `InterfacesRemoved` portant `org.bluez.Device1` : l'entrée, ses
  reconnexions planifiées et son statut `Link.Status()` disparaissent, l'acquisition
  est réconciliée, et **une** notification `KeyboardRemoved` est envoyée :
  « Clavier supprimé du poste : éteignez puis rallumez-le pour le réappairer »,
  avec les boutons **Réparer…** (`akmctl repair` dans un terminal) et **Ouvrir** ;
* BlueZ publie `Paired=false` juste avant de retirer l'objet : ce `Paired=false` n'est
  cru qu'après `BOND_GRACE` (1,5 s) sans suppression. Une vraie perte de pairage
  (l'objet reste) lève toujours « ré-appairage nécessaire » ; une suppression ne le
  lève jamais ;
* `InterfacesAdded` d'un `Device1` (réappairage) fait une ré-énumération.

Test : `apple-kb-monitord/tests/bluez_forget.rs` (bus privé, faux `org.bluez`,
faux serveur de notifications) et les tests `repair::tests::forgetting_from_plasma…`.

## 2. Une seule icône (#253)

Deux mécanismes, tous deux en temps réel :

* **Réclamation par le widget.** Le widget `com.agenceapi.devicehub` appelle
  `Tray.ClaimTrayFor(<id d'instance>)` à son chargement, à chaque réapparition du
  démon et toutes les 2 min (renouvellement) ; `ReleaseTrayFor` à sa destruction.
  Tant qu'une instance tient la réclamation, le démon libère son nom SNI (l'icône
  quitte la zone de notification) ; il la remet quand la dernière instance est
  libérée, que plasmashell quitte le bus, ou qu'une réclamation n'est plus
  renouvelée depuis 5 min (une libération perdue ne laisse donc jamais le bureau
  sans icône). Plusieurs instances (plusieurs panneaux) : chacune a la sienne.
  `ClaimTray()` / `ReleaseTray()` (sans argument) restent valables.
* **Configuration de plasmashell.** `plasma-org.kde.plasma.desktop-appletsrc` est
  surveillé (un `stat` toutes les 2 s, lecture seulement si le fichier change) :
  cocher ou décocher « ApiHub » dans Configurer la zone de notification › Entrées
  retire ou remet l'icône sans redémarrer ni plasmashell ni le démon.

Mode `APPLE_KB_MONITOR_TRAY` = `auto` (défaut) | `always` | `never` inchangé.
Test : `tray::tests::widget_claim_and_configuration_…` (bus privé, faux
StatusNotifierWatcher) et `plasma/tests/run-widget-tests.sh` (faux démon).

## 3. Alertes de piles et PowerDevil (#254)

PowerDevil suit le clavier par UPower et alerte à `PeripheralBatteryLowLevel`
(`powerdevilrc`, groupe `BatteryManagement`, **10 %** par défaut) via l'événement
`lowperipheralbattery` de `powerdevil.notifyrc`. Nos paliers 30 / 15 / 5 % étaient
une seconde voix sur le même appareil.

`[notifications] defer_to_powerdevil = true` (défaut) : le démon lit ces deux
fichiers (`~/.config`, puis `/etc/xdg` ; **lecture seule**, équivalent de
`kreadconfig6 --file powerdevilrc --group BatteryManagement --key
PeripheralBatteryLowLevel`), vérifie que `org.kde.Solid.PowerManagement` est sur
le bus, que l'événement affiche encore une bulle et que le clavier figure dans
`/sys/class/power_supply/hid-*-battery` (ce que voit UPower). Si tout est vrai
(contrôle refait au plus toutes les 60 s) :

* nos alertes de **pourcentage** passent à **un seul rappel distinct** « Estimation
  selon vos piles » (événement `BatteryEstimate`), envoyé au premier palier franchi
  et seulement si l'alerte repose sur l'estimation par chimie (information que
  PowerDevil n'a pas) ; remis à zéro au changement de piles ;
* si l'alerte repose sur l'indication brute du clavier (pas d'estimation), c'est le
  chiffre que PowerDevil voit : rien n'est envoyé ;
* les alertes du clavier lui-même (`0x30`, `KeyboardAlert`) ne changent pas.

`defer_to_powerdevil = false` rétablit les paliers 30 / 15 / 5 % complets.
PowerDevil absent, bulle désactivée dans Paramètres › Notifications, ou clavier non
suivi par UPower : comportement d'avant.

## 4. Deux noms pour un clavier (#248)

Le démon expose `Name` = l'**alias** BlueZ (celui que l'utilisateur choisit). Mais :

| Où | Nom affiché | Source |
|---|---|---|
| Applet Bluetooth, applet Batterie, widget, fenêtre, icône | alias | BlueZ `Device1.Alias` |
| KWin, Paramètres › Clavier, Périphériques d'entrée | nom noyau | `HID_NAME` du périphérique d'entrée |

Le nom noyau est fixé à la création du périphérique d'entrée : il ne change qu'après
**reconnexion** du clavier (l'éteindre puis le rallumer). KWin range les réglages
par périphérique sous ce nom (`kcminputrc [Libinput][vendor][product][nom]`) :
renommer dans le clavier ferait perdre de tels réglages. `akmctl status` ajoute une
ligne `Kernel:` quand alias et nom noyau diffèrent (clé JSON `kernel_name`).

## 5. Touches F4 et Éjecter (#247)

À l'installation et à la mise à jour du paquet (jusqu'à 3.1.0-10), un message propose
`akmctl keymap kde-apply --dry-run` puis `akmctl keymap kde-apply` (F4 → lanceur
d'applications, Éjecter → menu de session ; `--undo` retire exactement ces touches).
**Le paquet ne modifie aucun raccourci KDE.** Détails : `docs/TOUCHES.md`.

## 6. Français et anglais (#114)

Langue : première variable non vide parmi `LC_ALL`, `LC_MESSAGES`, `LANG` ; `fr…`
donne le français, tout le reste l'anglais (défaut).

| Surface | Mécanisme | Test |
|---|---|---|
| Fenêtre (titre « Moniteur de clavier Apple », onglets, valeurs) | `apihub-app/src/i18n.rs` + `apihub-app/i18n/fr.po` | `i18n::tests::every_window_text_has_a_french_translation` : chaque `tr("…")` a une entrée française, aucune entrée orpheline |
| Widget Plasma | `i18n()` + `plasma/po/fr.po` compilé en `.mo` (`/usr/share/locale/fr/LC_MESSAGES/plasma_applet_com.agenceapi.devicehub.mo`), `metadata.json` `Name[fr]` | `plasma/tests/check_i18n.py` (dans `run-widget-tests.sh`) |
| Notifications, info-bulle et menu de l'icône | `notify.rs`, `tray/view.rs` | tests existants (FR ≠ EN pour chaque texte) |
| Raisons des verrous de veille et d'arrêt | `sleep.rs`, `inhibitor_reason` | `sleep::tests::inhibitor_reasons_…` |
| Lanceur | `.desktop` : `Name[fr]`, `GenericName[fr]`, `Comment[fr]`, `Keywords[fr]` | `tests/desktop_i18n.rs` |

Ajouter un texte : l'écrire en anglais dans le code (`tr("…")` / `i18n("…")`), puis
ajouter l'entrée au `.po` ; le test échoue sinon.

## 7. Démarrage de la fenêtre (#21)

Pas d'autostart d'`apihub-app` : le démon `apple-kb-monitord` (unité utilisateur,
`default.target`) porte l'icône, les alertes et les notifications ; la fenêtre est
une application D-Bus activable (`DBusActivatable=true`) lancée à la demande par
l'icône, le widget ou un bouton de notification.

## 8. La fenêtre n'a plus d'icône à elle (#62)

Jusqu'à 3.1.0, `apihub-app` enregistrait un tray « hérité » (StatusNotifierItem)
quand le démon n'avait pas le sien au moment de l'ouverture. Or le démon retire son
icône quand le widget la réclame (§2) : ouvrir la fenêtre faisait alors apparaître
une seconde icône à côté du widget. Le tray hérité est supprimé
(`apihub-app/src/tray.rs`, 503 lignes, et la sonde du StatusNotifierWatcher dans
`instance.rs`) : l'icône appartient au démon seul.

Ce qui change : démon absent, ou `APPLE_KB_MONITOR_TRAY=never`, la fenêtre ouverte
n'ajoute plus d'icône ; elle se relève par un second lancement ou par `Activate`
(inchangé). L'entrée « Quitter » de ce tray disparaît avec lui : fermer la fenêtre
termine le processus (#226).

Vérification : `grep -rn "StatusNotifier" apihub-app/src` ne renvoie rien ;
`cargo test -p apihub-app` ; scénarios `open_close`, `desktop`, `desktop_slow` de
`tests/e2e`. `main.rs` : 906 → 539 lignes (onglet Diag dans `diag_tab.rs`, tracé de
l'historique dans `history_chart.rs`, briques egui dans `widgets.rs`).

## 9. Raccourci global : basculer les touches de fonction (#99)

`data/com.agenceapi.AppleKbMonitor.shortcuts.desktop`, installé dans
`/usr/share/kglobalaccel/`, déclare à `kglobalacceld` le composant « Clavier Apple »
avec deux entrées, visibles dans Configuration du système › Raccourcis :

| Entrée | Commande | Touche par défaut |
|---|---|---|
| Clavier Apple (ouvrir la fenêtre du moniteur) | `apihub-app` | aucune |
| Basculer les touches de fonction (multimédia / F1–F12) | `apihub-app --toggle-fn` | aucune |

`X-KDE-Shortcuts=` est vide dans les deux groupes : **rien n'est lié tant que
l'utilisateur n'attribue pas une touche**. Le paquet ne modifie aucun raccourci
existant.

`apihub-app --toggle-fn` n'ouvre aucune fenêtre et ne lit ni sysfs ni le clavier :
client D-Bus du démon, il appelle `GetDevices`, lit la propriété `Device.FnMode`,
puis `Device.SetFnMode` (même chemin que le menu de l'icône et le module de
réglages : le démon vérifie l'appelant et ouvre l'authentification polkit
`com.agenceapi.AppleKbMonitor.set-fnmode`). Bascule : 2 → 1 ; 1 ou 3 → 2 ; 0 ou 4 →
1 (défaut du noyau) ; mode inconnu (`hid_apple` non chargé) : rien n'est écrit,
code de sortie 1.

Tests : `fn_toggle::tests` (bus privé sans répertoire de services, faux démon ;
contenu du fichier `.desktop` ; ligne du PKGBUILD). Non testé ici : la liaison
réelle d'une touche dans une session Plasma (demande `kglobalacceld`) et l'OSD de
#100.

## 10. KRunner (#98)

| Saisie | Résultats |
|---|---|
| `clavier`, `keyboard` (dès 3 lettres : `cla`, `key`) | « *nom* : piles 96 % » (autonomie, tension, source, âge de la lecture) · Basculer les touches de fonction · Ouvrir le moniteur de clavier Apple · Réglages du clavier Apple |
| `batterie`, `battery`, `piles` (seuls ou avec `clavier`) | la ligne des piles |
| `fn`, `fn lock`, `touches de fonction`, `function keys` | Basculer les touches de fonction (sous-texte : mode actuel) |

Valider la ligne des piles ou « Ouvrir » appelle `org.freedesktop.Application.Activate`
sur `com.agenceapi.AppleKbMonitor` (la fenêtre démarre par activation D-Bus) ;
« Réglages » lance `systemsettings kcm_applekeyboard` (à défaut `kcmshell6`) ;
« Basculer » suit le chemin du §9. Démon absent : la ligne dit « moniteur arrêté » et
la bascule n'est pas proposée ; le runner ne démarre jamais le démon.

Pièces :

* `data/plasma-runner-applekeyboard.desktop` → `/usr/share/krunner/dbusplugins/`
  (`X-Plasma-API=DBus`, service `com.agenceapi.AppleKbMonitor.Runner`, chemin
  `/runner`, `X-Plasma-Runner-Match-Regex` : KRunner n'interroge le service que pour
  une saisie commençant un des mots ci-dessus) ;
* `plasma/krunner/com.agenceapi.AppleKbMonitor.Runner.service` →
  `/usr/share/dbus-1/services/` : `apihub-app --krunner`, démarré par activation
  D-Bus à la première saisie concernée ;
* `apihub-app/src/krunner.rs` : interface `org.kde.krunner1` (`Match`, `Actions`,
  `Run`). Sans fenêtre, client du démon (propriété `Json`, `Device.FnMode`, attente
  bornée à 0,8 s), deux connexions (une sert l'objet, l'autre appelle le démon), fin
  du processus après 2 min sans requête.

Le service vit dans `apihub-app`, pas dans le démon : rien à brancher côté démon.

Tests : `krunner::tests` (correspondances, service réel `org.kde.krunner1` sur bus
privé avec faux démon, signature `a(sssida{sv})`, fichiers `.desktop` / `.service` /
PKGBUILD). Non testé ici : l'affichage dans KRunner d'une session Plasma.

## 11. Widget : sparkline 7 jours et bouton de mode Fn (#97)

Le popup du widget `com.agenceapi.devicehub` affiche, sous la jauge :

* une **sparkline des 7 derniers jours** (pourcentage, échelle 0–100 %), tracée par
  `LineChart.qml` (un `Canvas`, aucune bibliothèque de graphiques) à partir de
  `History(since)` du démon. `History.js` écarte les points invalides (hors 0–100 %,
  tension non mesurée, date future) et réduit chaque série à 120 points pour la
  sparkline (240 pour la page Historique, §12) en gardant les extrêmes ;
* une ligne **Touches de fonction** : le mode lu sur l'objet du clavier
  (`Device.FnMode`) et un bouton « Passer à F1–F12 d'abord » / « Passer aux touches
  multimédia d'abord » qui appelle `Device.SetFnMode`. Le démon vérifie l'appelant
  et ouvre l'authentification polkit ; le widget n'écrit rien lui-même. La ligne est
  masquée quand `hid_apple` n'est pas chargé (mode inconnu).

Ces lectures ne sont faites qu'à l'ouverture du popup (`onExpandedChanged`) : fermé,
le widget ne fait que suivre `StateChanged`, comme avant. Démon absent : le popup
affiche « Moniteur arrêté » et rien d'autre.

Tests (`plasma/tests/run-widget-tests.sh`) : `tst_history.qml` (259 200 points sur
90 jours bornés à 240, extrêmes conservés, points invalides écartés, bascule Fn,
chemin d'objet refusé pour une adresse invalide) et `tst_link.qml` contre le faux
démon (`History`, `FnMode` lu, `SetFnMode(2)` appelé une seule fois, relu). Non
testé ici : le rendu dans plasmashell (`plasmoidviewer` ouvrirait une fenêtre sur
le bureau).

## 12. Widget : pages Historique et Diagnostic (#120)

Le popup a trois onglets : **État** (l'affichage d'avant, §11), **Historique**,
**Diagnostic**. Tout vient de méthodes D-Bus que le démon expose déjà ; le widget ne
lance aucun processus et ne lit aucun fichier.

| Page | Contenu | Source D-Bus |
|---|---|---|
| Historique (`HistoryPage.qml`) | piles (0–100 %) et tension sur 7, 30 ou 90 jours, nombre de mesures, étendues, dates de début et de fin | `History(t since)` |
| Diagnostic (`DiagPage.qml`) | moniteur en service et version ; clavier connecté ou non, dernière erreur ; heure de la dernière lecture, lecture incomplète ; signal ; état de la liaison (tentatives, échecs, dernière erreur) ; firmware ; mode Fn | `GetState`, propriété `DaemonVersion`, `Link.Status()`, `Device.FnMode` |
| Diagnostic, boutons | **Relire** et **Reconnecter** | `Refresh()`, `Link.Reconnect()` |

La page Historique ne trace jamais plus de 240 points par série (`History.js`,
`POINTS_MAX`), quelle que soit la période.

Limites, à traiter côté démon :

* **`Diagnose()` n'existe pas.** Les contrôles de l'onglet Diag de la fenêtre
  (`apihub-app/src/diag_tab.rs` : binaires, unité systemd, hidraw, hwdb, règles udev,
  `fnmode`, rssi-helper) lancent des commandes et lisent des fichiers ; le widget ne
  le fait pas. La page renvoie à Configuration du système › Clavier Apple ›
  Diagnostic. Quand le démon exposera `Diagnose()`, il suffira d'en afficher le
  résultat dans `DiagPage.qml`.
* **`History(since)` renvoie toute la période** : sur 90 jours, la réponse JSON est
  analysée dans plasmashell avant d'être réduite à 240 points. Une variante bornée
  côté démon (nombre maximal de points) éviterait ce transfert ; la page n'est
  chargée qu'à l'ouverture de l'onglet.
* La feature Cargo `gui` d'`apihub-app` (fenêtre optionnelle) n'est pas faite.

Tests : `tst_pages.qml` instancie le popup complet et les deux pages hors écran,
sur un faux applet, et échoue sur toute liaison vers un nom absent ; `tst_link.qml`
vérifie `Link.Status`, `Reconnect`, `Refresh` et `DaemonVersion` contre le faux
démon. Non testé ici : le rendu dans plasmashell.
