# Roadmap fonctionnalités — clavier Apple (2026-10-01)

Objectif du gérant : « le meilleur driver et le meilleur menu tray 2026, avec le plus de fonctionnalités liées au clavier ».
Périmètre : claviers Apple Bluetooth/USB (et, en extension, Magic Trackpad/Mouse appairés au même poste). Écran, DDC et MQTT hors sujet (#59).

Légende des preuves : **[mesuré]** = constaté sur le poste (noyau 7.1.13, Magic Keyboard `0005:05AC:0256`) ; **[source]** = documentation, code noyau ou projet cité ; **[hypothèse]** = non vérifié, à prouver avant développement.

---

## 1. Point de départ (code `main` @ 620013d)

| Brique | État |
|---|---|
| `apple-kb-monitord` (démon user, D-Bus `com.agenceapi.AppleKbMonitor1`) | lecture seule : `Battery`, `Voltage`, `Rssi`, `Connected`, `Model`, `Mac`, `GetState`, `Refresh`, `History(since)`, signal `StateChanged` ; **un seul clavier** |
| Notifications | un seul seuil `--threshold N` (défaut 20 %) |
| Tray (`apihub-app/src/tray.rs`) | lignes d'information + « Show Window » + « Quit » ; aucune action clavier |
| Historique | JSONL + `estimate_remaining` (akm-core/history.rs), graphe 24 h dans l'appli |
| Réglages `hid_apple` | lus dans l'onglet Diag, **jamais écrits** |
| keyd | un fichier statique `keyd/apple-keyboard.conf` |

---

## 2. Benchmark 2026 — ce qui rend les outils comparables excellents

| Outil | Points forts à reprendre | Source |
|---|---|---|
| **Solaar** (Logitech) | icône tray = batterie de l'appareil **le plus faible** ; icônes symboliques ; réglages **mémorisés et réappliqués** à la reconnexion ; appairage/désappairage depuis l'UI ; CLI couvrant presque toute la GUI (`solaar show/config/pair`) ; éditeur de règles (actions sur touches « diverties ») ; vue diagnostic détaillée | [usage](https://pwr-solaar.github.io/Solaar/usage/), [capabilities](https://pwr-solaar.github.io/Solaar/capabilities/) |
| **Piper / libratbag** | séparation **démon D-Bus (ratbagd) / GUI mince**, activation D-Bus à la demande ; modèle profils → boutons/LED ; profils matériels | [libratbag](https://github.com/libratbag/libratbag), [D-Bus API](https://libratbag.github.io/dbus.html), [piper](https://github.com/libratbag/piper) |
| **OpenRGB** | **SDK réseau** + bindings multiples, profils chargeables par API, système de plugins | [OpenRGB SDK](https://gitlab.com/CalcProgrammer1/OpenRGB/-/blob/master/Documentation/OpenRGBSDK.md?ref_type=heads), [plugins](https://openrgb.org/plugins.html) |
| **keyd / kanata** | remappage noyau (evdev+uinput) ; **couches dépendantes de l'application** (keyd-application-mapper, kanata-switcher/qanata) ; tap-hold, macros | [kanata](https://github.com/jtroo/kanata), [comparatif](https://dev.to/argenkiwi/keyboard-remapping-on-steroids-keyd-vs-kanata-vs-keydo-501e) |
| **Karabiner-Elements** (macOS) | **règles par appareil** (`device_if`), comportement des touches F par appareil, **visualiseur d'événements**, bibliothèque de règles communautaire, CLI de bascule de profil | [CLI](https://karabiner-elements.pqrs.org/docs/manual/misc/command-line-interface/), [JSON](https://karabiner-elements.pqrs.org/docs/json/root-data-structure/) |
| **GNOME Bluetooth Battery Meter** | indicateur par appareil, niveau détaillé dans le menu rapide, styles/couleurs configurables | [extension](https://github.com/maniacx/Bluetooth-Battery-Meter) |
| **Bluetooth Battery Indicator** | panneau = plus faible, menu = tous les appareils `org.bluez.Battery1` ; demande récurrente : **afficher seulement sous un seuil** | [dosment](https://github.com/dosment/bluetooth-battery-indicator), [issues](https://github.com/MichalW/gnome-bluetooth-battery-indicator/issues) |
| **magic-trackpad-battery** | lecture `HIDIOCGINPUT` du rapport `0x90` ; **alertes 20/15/10/5 %** ; reconnexion automatique | [dépôt](https://github.com/mmarfil/magic-trackpad-battery) |
| **KDE PowerDevil / Bluedevil** | notification batterie faible des périphériques, mais **seuil non réglable par appareil** et notifications erronées signalées en Plasma 6.7.5 / KF6 6.30 | [discuss.kde.org](https://discuss.kde.org/t/energy-notification-works-bad-from-plasma-6-7-5/50171), [Fedora](https://discussion.fedoraproject.org/t/erroneous-low-battery-notifications/201767) |

**Synthèse** : les meilleurs combinent (a) un démon unique et une API publique (D-Bus/SDK), (b) des réglages **par appareil réappliqués à la reconnexion**, (c) une CLI équivalente à la GUI, (d) un tray qui montre l'appareil le plus faible et donne des **actions**, pas seulement de l'information, (e) des seuils d'alerte multiples et configurables. Aucun outil Linux n'offre aujourd'hui de prévision d'autonomie, de détection de changement de piles ni de bascule Fn depuis le tray : c'est le créneau différenciant.

---

## 3. Ce que le matériel et le noyau permettent réellement

| Capacité | Statut | Détail |
|---|---|---|
| `fnmode` runtime | **[mesuré]** `/sys/module/hid_apple/parameters/fnmode` = 1, `root 644` (écriture root) ; **[source]** valeurs 0 désactivé, 1 fkeyslast, 2 fkeysfirst, 3 auto (défaut), 4 fkeysdisabled | [hid-apple.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-apple.c) |
| `iso_layout` | **[mesuré]** -1 ; **[source]** -1 auto, 0 off, 1 on | idem |
| `swap_opt_cmd` | **[mesuré]** 0 ; **[source]** 0 Mac, 1 Windows, 2 côté gauche seulement | idem |
| `swap_ctrl_cmd`, `swap_fn_leftctrl` | **[mesuré]** 0/0, `root 644` | idem |
| Portée des paramètres | **[source]** paramètres de **module** : globaux à tous les claviers `hid_apple` (pas par appareil) | idem |
| Batterie | **[mesuré]** `power_supply hid-04:db:56:ca:42:ee-battery-71` : `capacity`=99, `status`=Discharging, `scope`=Device ; UPower l'expose | — |
| Batterie BT Magic Keyboard 2021 / Trackpad 2 / Mouse 2 | **[source]** `hid-magicmouse` ne lit la batterie qu'en USB ; en BT le power_supply reste à 0 %. Série de correctifs Alec Hall (juil.–août 2026, respin demandé) | [Phoronix](https://www.phoronix.com/news/Linux-7.0-Fix-Magic-Trackpad-2), [patch](https://ratatoskr.run/linux-input/2026/07/17256372/t) |
| Rapport `0x90` (trackpad/souris) | **[source]** `HIDIOCGINPUT` : octet 1 drapeaux (bit 1 = en charge), octet 2 capacité 0–100 | [magic-trackpad-battery](https://github.com/mmarfil/magic-trackpad-battery) |
| Feature reports BCM2042 (A1314) | **[source projet]** 21 rapports lus (`0xEA` % précis, `0xF5` ADC, `0x5A` courbe, `0x4F` firmware, `0xFF` build…) — lecture seule | README, akm-core/decode.rs |
| LED CapsLock | **[mesuré]** `/sys/class/leds/input49::capslock/brightness`, `root 644` ; pilotée par l'état du verrouillage (cf. #68) | — |
| Rétroéclairage | **[source]** `APPLE_BACKLIGHT_CTL` / `APPLE_MAGIC_BACKLIGHT` = claviers **internes** MacBook T2 et Touch Bar uniquement. **Aucun Magic Keyboard externe n'a de rétroéclairage.** → non retenu | hid-apple.c |
| Touch ID (Magic Keyboard 2021/2024) | **[source]** réservé aux Mac Apple Silicon (Secure Enclave), non pris en charge par Linux → non retenu | [Phoronix 5.16](https://www.phoronix.com/news/Linux-5.16-Apple-Magic-2021), [Apple](https://support.apple.com/en-au/guide/mac-studio/apde6983e836/2022/mac/13) |
| Touche Eject (A1314) | **[source]** mappée `KEY_EJECTCD` ; remappable via keyd | hid-apple.c |
| Mise à jour firmware | **[hypothèse]** DFU propriétaire Apple, aucun outil Linux connu → non retenu (lecture de version seulement) | — |
| Température / délai de veille réglable | **[hypothèse]** non exposés par les rapports connus ; écrire des feature reports inconnus = risque de brique → non retenu | — |
| Appairage / connexion | **[mesuré]** `org.bluez /org/bluez/hci0/dev_04_DB_56_CA_42_EE` : `Connect`, `Disconnect`, `Trusted`, `RemoveDevice` disponibles via BlueZ | — |
| keyd | **[mesuré]** `/usr/bin/keyd` installé ; kanata absent | — |

**Conséquence d'architecture** : toute écriture (`hid_apple`, `/etc/keyd`) exige root. Un **seul helper privilégié minimal, autorisé par polkit** (action `com.agenceapi.AppleKbMonitor.configure`), avec liste blanche de paramètres/valeurs, est un prérequis commun (F07). Pas d'écriture directe depuis l'appli utilisateur.

---

## 4. Fonctionnalités priorisées

Valeur : ★★★ forte · ★★ moyenne · ★ faible. Effort : S ≤ 1 j, M 2–4 j, L ≥ 5 j. Risque : B bas, M moyen, H haut.

| ID | Fonctionnalité | Valeur | Faisab. | Risque | Effort | Jalon | Issue |
|---|---|---|---|---|---|---|---|
| F01 | Alertes batterie multi-seuils configurables (défaut 30/15/5 %) avec hystérésis | ★★★ | certaine | B | S | 1 | #82 |
| F02 | Prévision d'autonomie en jours (régression sur l'historique), tray + tooltip + D-Bus | ★★★ | certaine | M (bruit des mesures) | M | 1 | #83 |
| F03 | Notifications connexion / déconnexion / reconnexion (silencieuses par défaut) | ★★★ | certaine | B | S | 1 | #84 |
| F04 | Détection de remplacement de piles / recharge + journal « durée de vie par jeu de piles » | ★★★ | probable | M | M | 1 | #85 |
| F05 | Menu tray riche : %, autonomie, connexion, RSSI, modèle + actions (Fn, rafraîchir, copier diagnostic, réglages) | ★★★ | certaine | B | M | 1 | #86 |
| F06 | Icône tray dynamique symbolique (paliers, critique, déconnecté, en charge), compatible Breeze clair/sombre | ★★★ | certaine | B | S | 1 | #87 |
| F07 | Helper privilégié polkit + bascule Fn lock (fnmode 1↔2) depuis le tray, persistée dans modprobe.d | ★★★ | certaine | M (root) | M | 1 | #88 |
| F08 | Inversions Option/Cmd (0/1/2), Ctrl/Cmd, Fn/Ctrl gauche depuis les réglages | ★★ | certaine | M | S | 1 | #89 |
| F09 | Disposition ISO/ANSI (`iso_layout` -1/0/1) depuis les réglages | ★★ | certaine | B | S | 1 | #90 |
| F10 | Mode ne-pas-déranger : respect du DND Plasma + plages horaires ; le seuil critique passe toujours | ★★ | certaine | B | S | 1 | #91 |
| F11 | CLI Rust `akmctl` : `status --json`, `get/set fnmode`, `watch` (flux JSON des signaux), codes retour | ★★★ | certaine | B | M | 1 | #92 (waybar/CSV/Prometheus : #22) |
| F12 | API D-Bus v2 : multi-appareils (`/devices/<mac>`), méthodes d'écriture, signaux `BatteryLevelCrossed`, `ConnectionChanged` | ★★★ | certaine | M (compat v1) | M | 1 | #93 |
| F13 | Multi-claviers simultanés ; tray = appareil le plus faible (modèle Solaar) | ★★★ | certaine | M | M | 2 | #94 |
| F14 | Magic Trackpad 2 / Magic Mouse 2 : batterie BT via rapport `0x90` tant que le noyau ne la publie pas | ★★ | probable | M | M | 2 | #95 |
| F15 | Historique 7/30/90 j : graphes % et tension, rotation/rétention bornée | ★★ | certaine | B | M | 2 | #96 |
| F16 | Export historique CSV/JSON + archive de diagnostic | ★★ | certaine | B | S | 2 | #22 |
| F17 | Widget Plasma enrichi : jauge, sparkline 7 j, autonomie, bouton Fn lock | ★★★ | certaine | B | M | 2 | #97 |
| F18 | Runner KRunner (« clavier », « fn lock », « batterie clavier ») | ★★ | certaine | B | M | 2 | #98 |
| F19 | Raccourcis globaux KGlobalAccel : bascule Fn lock, afficher l'état | ★★ | certaine | B | S | 2 | #99 (F1/F2 : #68) |
| F20 | OSD Plasma (`org.kde.osdService`) au changement de Fn mode / disposition | ★★ | certaine | B | S | 2 | #100 |
| F21 | Indicateur CapsLock à l'écran (OSD) et dans le tray | ★ | certaine | B | S | 2 | #101 |
| F22 | Remappage keyd par profils (Mac, PC, Dev, Eject→Suppr) : éditeur, `keyd check`, rechargement, retour arrière | ★★★ | certaine | H (clavier inutilisable si erreur) | L | 2 | #102 |
| F23 | Réglages par clavier (MAC) mémorisés et réappliqués à la reconnexion | ★★★ | probable (hid_apple global) | M | M | 2 | #103 |
| F24 | Actions Bluetooth depuis le tray : reconnecter, déconnecter, oublier, ré-appairer guidé | ★★ | certaine | M | M | 2 | #104 |
| F25 | Qualité de liaison : historique RSSI, compteur de déconnexions, alerte liaison instable | ★★ | certaine | B | M | 2 | #105 |
| F26 | Assistant de diagnostic guidé avec correctifs en un clic (udev, hidraw, keyd, BlueZ, UPower, helper) | ★★ | certaine | M | M | 2 | #106 |
| F27 | Profils keyd par application (couches selon la fenêtre active, KWin) | ★★ | probable (Wayland) | M | L | 3 | #107 |
| F28 | Santé des piles : tension vs %, type de piles (alcaline/NiMH), recommandations | ★★ | probable | M | M | 3 | #108 |
| F29 | Statistiques d'usage sans keylogging (heures actives via événements de réveil) | ★ | probable | M (vie privée) | M | 3 | #109 |
| F30 | Notifications actionnables (« Me rappeler demain », « Ouvrir l'historique ») | ★★ | certaine | B | S | 3 | #110 |
| F31 | Module KCM dans Configuration du système (Matériel → Clavier Apple) | ★★ | certaine | M | L | 3 | #111 |
| F32 | Mode filaire USB/Lightning/USB-C : détection câble, état de charge, bascule de source | ★ | certaine | B | S | 3 | #112 |
| F33 | Visualiseur d'événements de touches (codes HID → evdev → keyd), à la Karabiner EventViewer | ★★ | certaine | B | M | 3 | #113 |
| F34 | Internationalisation FR/EN (gettext / fluent) de l'appli, du tray et des notifications | ★★ | certaine | B | M | 3 | #114 |
| F35 | Exporteur Prometheus et module waybar natifs Rust | ★ | certaine | B | S | 3 | #22 |

**Non retenus** (matériel ou risque) : rétroéclairage, Touch ID, mise à jour firmware, réglage du délai de veille, capteur de température (cf. §3).

---

## 5. Critères d'acceptation (testables)

- **F01** — `config.toml` `[alerts] thresholds=[30,15,5]` ; simulation (akm-core, sans matériel) 100→0 % : exactement 3 notifications, une par franchissement ; oscillation 15↔16 % : aucune nouvelle notification (hystérésis ≥ 3 points) ; seuil 5 % en urgence `critical`.
- **F02** — sur un historique synthétique à pente constante de 1 %/jour, l'estimation est à ± 10 % ; < 48 h de données → « estimation indisponible » ; valeur exposée en D-Bus (`RemainingSeconds`, -1 si inconnue) et dans le tooltip.
- **F03** — `bluetoothctl disconnect` puis `connect` : une notification « déconnecté » puis « reconnecté (N %) » ; pas de notification au démarrage de session ; désactivable par option.
- **F04** — saut ≥ 20 points de % ou ≥ 300 mV vers le haut en < 1 h = événement `BatteryReplaced` journalisé (date, %, mV) ; la commande `akmctl batteries` liste les jeux de piles avec leur durée ; les alertes F01 sont réarmées.
- **F05** — le menu dbusmenu contient : ligne niveau + autonomie, état de connexion, RSSI, sous-menu « Touches de fonction » (cases radio), « Rafraîchir », « Copier le diagnostic » (presse-papiers), « Réglages… », « Quitter » ; vérifiable par `dbus-send … com.canonical.dbusmenu.GetLayout`.
- **F06** — 6 états d'icône (100/80/60/40/20/critique) + déconnecté ; noms d'icônes symboliques existant dans Breeze ou fournis dans `icons/` ; test unitaire de la fonction niveau→nom.
- **F07** — action polkit `com.agenceapi.AppleKbMonitor.configure` (allow_active=auth_admin_keep) ; le helper refuse tout paramètre/valeur hors liste blanche (test) ; bascule depuis le tray : `cat /sys/module/hid_apple/parameters/fnmode` passe de 1 à 2 et inversement ; persistance écrite dans `/etc/modprobe.d/hid_apple.conf` et relue après redémarrage.
- **F08** — chaque valeur (0/1/2 pour swap_opt_cmd ; 0/1 pour les autres) est appliquée et relue dans sysfs ; libellés explicites (« Disposition Windows », etc.).
- **F09** — iso_layout -1/0/1 appliqué ; sur ISO, la touche `<>` émet `KEY_102ND` (vérifié avec `evtest`).
- **F10** — avec « Ne pas déranger » Plasma actif, les notifications non critiques ne sont pas envoyées (journalisées) ; la critique (≤ 5 %) l'est ; plages horaires dans la config, testées par horloge simulée.
- **F11** — `akmctl status --json` valide contre un schéma JSON versionné ; `akmctl get fnmode`, `akmctl set fnmode 2` (passe par F07) ; `akmctl watch` émet une ligne JSON par signal ; code retour 0/1/2 (OK/erreur/absent) ; tests sans matériel via le démon factice.
- **F12** — introspection : `/com/agenceapi/AppleKbMonitor1/devices/<mac>` par appareil, interface v1 conservée (compat) ; méthodes `SetFnMode`, `SetSwapOptCmd`, `SetIsoLayout` (délèguent au helper) ; signaux émis lors d'un test d'intégration avec bus de session privé.
- **F13** — avec 2 appareils (dont un simulé), le tray affiche le plus faible, le menu les liste tous ; un débranchement n'affecte pas l'autre.
- **F14** — Magic Trackpad 2 BT : % lu via `0x90` égal à celui de macOS/USB à ± 2 % ; si le noyau publie un `power_supply` non nul, la source noyau est prioritaire (pas de double comptage) ; aucun blocage du thread en cas de timeout.
- **F15** — graphes 7/30/90 j ; fichier d'historique borné (rotation ≤ 5 Mo) ; points invalides rejetés (déjà #39).
- **F17** — le plasmoïde affiche jauge, sparkline 7 j, autonomie et bascule Fn via D-Bus ; fonctionne démon arrêté (affiche « démon absent »), validé par `plasmoidviewer`.
- **F18** — taper « fn lock » dans KRunner propose « Basculer les touches de fonction » ; « batterie clavier » affiche le % et l'autonomie.
- **F19** — raccourci configurable visible dans Configuration du système → Raccourcis ; déclenche la bascule F07.
- **F20** — chaque changement de fnmode/disposition affiche un OSD Plasma avec le nouvel état.
- **F21** — appui CapsLock : OSD « Verr. Maj activé/désactivé » (option) et badge dans le tray.
- **F22** — 3 profils livrés ; `keyd check` exécuté avant toute application, échec = refus ; application atomique + rollback automatique si aucun appui de confirmation sous 15 s ; Eject remappable.
- **F23** — deux claviers avec réglages différents : à la reconnexion de chacun, ses réglages keyd sont réappliqués ; pour les paramètres `hid_apple` globaux, l'UI l'indique explicitement.
- **F24** — « Reconnecter », « Déconnecter », « Oublier » appellent BlueZ (vérifié par `busctl monitor`) ; « Oublier » demande confirmation.
- **F25** — RSSI conservé 7 j ; > 3 déconnexions/h ou RSSI < -80 dBm pendant 10 min → notification « liaison instable ».
- **F26** — chaque contrôle renvoie OK/KO + correctif ; les correctifs root passent par le helper ; test par environnement simulé (règle udev absente → KO).
- **F27** — focus d'une fenêtre listée → couche keyd activée en < 200 ms (KWin script ou D-Bus) ; retour à la couche par défaut au changement de focus.
- **F28** — tension et % tracés ensemble ; choix du type de piles dans la config adapte la courbe ; recommandation affichée sous 2 jeux de piles consécutifs < 30 jours.
- **F29** — aucun code de touche stocké (revue + test) ; heures actives par jour dérivées des réveils et de l'activité ; désactivé par défaut.
- **F30** — les notifications portent des actions ; « Me rappeler demain » re-planifie l'alerte 24 h plus tard (horloge simulée).
- **F31** — module KCM chargé par `kcmshell6 kcm_applekb` ; mêmes réglages que l'appli, écrits via D-Bus.
- **F32** — câble branché : source « USB », état `Charging`, BT ignoré pour le même numéro de série.
- **F33** — fenêtre listant, par appui : code HID, code evdev, sortie keyd ; lecture seule.
- **F34** — toutes les chaînes extraites ; locale `fr_FR` et `en_US` complètes ; test : aucune chaîne non traduite.

---

## 6. Ordre de livraison

**Jalon 1 — v3.1 « Tray utile et alertes justes »** (dépend du démon #61/#62) : F12 → F07 → F01, F03, F10 → F02, F04 → F06, F05 → F08, F09 → F11.
Livrable : un tray qui informe et agit, des alertes multi-seuils sans bruit, une API D-Bus v2 et une CLI scriptable.

**Jalon 2 — v3.2 « Multi-appareils et intégration KDE »** : F13 → F14 → F15, F16 → F17, F18, F19, F20, F21 → F23 → F22 → F24, F25, F26.
Livrable : tous les périphériques Apple dans un seul tray, intégration Plasma native, remappage keyd par profils sûr.

**Jalon 3 — v4.0 « Différenciation »** : F28, F30, F32 → F33 → F27 → F29 → F31, F34, F35.
Livrable : santé des piles, profils par application, KCM, i18n.

---

## 7. Écart signalé

Les issues #18, #19 et #20 (courbe circadienne, écrans LG, MQTT) concernent le périmètre écran retiré par #59 : elles devraient être fermées ou déplacées.
