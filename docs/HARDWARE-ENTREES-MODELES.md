# Entrées clavier et couverture des modèles

Audit du 2026-10-01, PC01 (Manjaro, noyau 7.1.13, keyd v2.6.0, BlueZ 5.87), clavier réel
A1314 ISO « Clavier de maria #1 » `04:DB:56:CA:42:EE` (`0005:05AC:0256`, pilote `apple`).
Base de code : `main` @ `a2b7359`. Branche du livrable : `re/entrees-modeles`.

**Méthode : lecture seule.** Aucune écriture sur le clavier (ni SET_REPORT, ni rapport de sortie,
ni EV_LED), pas de sudo, aucun service arrêté. Accès au matériel limités à :
`/sys` (uevent, `report_descriptor`, `capabilities`, LED `brightness`, paramètres `hid_apple`)
et `evtest /dev/input/event28` en mode capacités (le nœud était déjà saisi par keyd : evtest a
signalé « grabbed by another process » et n'a reçu aucun évènement). Le clavier s'est mis en veille
à 04:00:33 ; aucun accès matériel ensuite. Aucun échantillonnage d'évènements de touches n'a été
fait : ce qui est marqué « non observé » n'a pas été vu sur le fil.

Sources : `drivers/hid/hid-apple.c`, `hid-ids.h`, `hid-input.c`, `hid-magicmouse.c`
(torvalds/linux master du 2026-10-01), selftest noyau `tools/testing/selftests/hid/tests/test_apple_keyboard.py`,
`rvaiya/keyd` master (`daemon.c`, `device.c`, `vkbd/uinput.c`), gist xloc/9f1ecca9 (descripteur du
Magic Keyboard 2021). Fixtures et provenance : `tests/fixtures/models/README.md`.

Tests sans matériel : `python3 -m pytest tests/live/re -q` (18 réussis, 4 échecs attendus
marqués avec leur issue). Analyseur de descripteur : `tests/live/re/hid_rdesc.py <fichier>`.

---

## 1. Côté entrées de l'A1314 (BCM2042) — mesuré

Descripteur capturé (224 o) **identique octet pour octet** au descripteur du selftest noyau
`AppleKeyboard` (`test_identique_au_selftest_noyau`).

| Rapport | Type | Contenu (bit : usage) | Noyau (hid-apple / hid-input) | Driver aujourd'hui |
|---|---|---|---|---|
| `0x01` | entrée 8 o | modificateurs E0–E7, 1 o réservé, 6 codes (0–255) | touches ; traduction Fn selon `fnmode` | non lu (le moniteur de réveil le lit dans le flux et le jette) |
| `0x01` | sortie 1 o | bits 0–4 : NumLock, CapsLock, ScrollLock, Compose, Kana (page `0x08`) | `input51::{numlock,capslock,scrolllock,compose,kana}` | Rust : lecture sysfs + écriture EV_LED (§3) ; Python : écriture hidraw brute (#128) |
| `0x11` | entrée 1 o | bit 3 `000C:00B8` Éjection ; bit 4 `00FF:0003` Fn | `KEY_EJECTCD`, `KEY_FN` | ignoré (#130) |
| `0x12` | entrée 1 o | bits 0–4 : lecture/pause, avance, retour, piste suivante, précédente | `KEY_PLAYPAUSE`, `KEY_FASTFORWARD`, `KEY_REWIND`, `KEY_NEXTSONG`, `KEY_PREVIOUSSONG` | ignoré ; émission réelle **non observée** (les F7–F9 passent par le rapport 0x01 + table Fn) |
| `0x13` | entrée 1 o | bit 0 `FF01:000A`, bit 1 `FF01:000C` (NoPref) | non mappé (vendor) | Rust : compte tout `0x13` sans décoder (#129) ; Python : décode les 2 bits |
| `0x47` | entrée 1 o | `0006:0020` Battery Strength 0–255 | `power_supply hid-<mac>-battery-71` (71 = 0x47), 99 % lu | batterie (noyau d'abord) |
| `0x09` | feature 3 o | `FF01:000B` + 2 o constants | — | lu (« device state ») |

Capacités evdev (`tests/fixtures/a1314_iso/evtest_capabilities.txt`, 182 codes) : EV_SYN, EV_KEY,
EV_MSC (MSC_SCAN), EV_LED (5 LED). Touches spéciales présentes : `KEY_FN`, `KEY_EJECTCD`,
`KEY_BRIGHTNESSDOWN/UP`, `KEY_SCALE`, `KEY_DASHBOARD`, `KEY_KBDILLUM{DOWN,UP,TOGGLE}`,
`KEY_PREVIOUSSONG`, `KEY_PLAYPAUSE`, `KEY_NEXTSONG`, `KEY_FASTFORWARD`, `KEY_REWIND`, `KEY_MUTE`,
`KEY_VOLUMEDOWN/UP`, `KEY_SEARCH`, `KEY_MICMUTE`, `KEY_SLEEP`, `KEY_POWER`, `KEY_STOPCD`
(hid-apple déclare l'union de toutes ses tables ; l'A1314 n'en émet qu'une partie, cf. ci-dessous).

**État Fn** (`/sys/module/hid_apple/parameters`, lisibles sans droit) : `fnmode=1`, `iso_layout=-1`,
`swap_opt_cmd=0`, `swap_ctrl_cmd=0`, `swap_fn_leftctrl=0`. `fnmode=1` vient de `modprobe/hid_apple.conf`
(redondant avec le défaut 3 = auto → 1 pour un clavier Apple, déjà noté dans #68). Aucun code du
driver ne lit ces paramètres (#88, #92 pour l'écriture ; lecture proposée dans #130).

Table Fn appliquée par le noyau aux 9 PID BCM2042 (`magic_keyboard_alu_fn_keys`, fnmode 1 = média
par défaut, F-key avec Fn) : F1 luminosité −, F2 +, F3 `KEY_SCALE`, F4 `KEY_DASHBOARD`,
**F5 sans traduction**, **F6 `KEY_NUMLOCK`**, F7–F9 médias, F10–F12 volume, Fn+⌫ = Suppr,
Fn+↵ = Inser, Fn+flèches = Pg/Début/Fin. Les 9 PID ont `APPLE_NUMLOCK_EMULATION` : quand la LED
NumLock est allumée au niveau noyau, J/K/L/U/I/O/M… deviennent des touches du pavé (hid-apple l.575).

## 2. keyd et remappage — lu dans le code et dans /etc/keyd

`/etc/keyd/apple-keyboard.conf` = `keyd/apple-keyboard.conf` du dépôt (identiques).
`[ids] 05ac:0256` seulement. keyd (pid actif) saisit `event28` (EVIOCGRAB) et réémet sur
« keyd virtual keyboard » (`input49`).

Correspondance effective avec `fnmode=1` sur l'A1314 :

| Touche | Sans Fn → keyd | Avec Fn → keyd |
|---|---|---|
| F3 | `KEY_SCALE` → `M-z` | `KEY_F3` → `M-z` |
| F4 | `KEY_DASHBOARD` → `M-g` | `KEY_F4` → **non remappé** |
| F5 | `KEY_F5` → `M-l` | `KEY_F5` → `M-l` |
| F6 | `KEY_NUMLOCK` → `M-d` | `KEY_F6` → `M-d` |
| `kbdillumdown/up` | jamais émis par l'A1314 (lignes mortes) | — |
| Éjection | `KEY_EJECTCD` → non remappé | — |

Défauts : #126 (ids limités à un PID, F4 asymétrique, tables 2015/2021 non couvertes : sur un
Magic Keyboard 2021 F6 = `KEY_SLEEP`).

## 3. LED via keyd — vérification du code (sans écrire de LED)

Chemin Rust (`akm-core/src/led.rs`) : `led_target_in()` → clavier virtuel keyd s'il existe, sinon
evdev Apple trouvé par `HID_ID`. Écriture d'un `input_event` EV_LED de 24 o. keyd
(`daemon.c`) relaie tout EV_LED reçu par son périphérique virtuel vers **tous les périphériques
qu'il a saisis** via `device_set_led()` (écriture sur leur fd saisi) : le chemin est correct
**pour l'A1314 0x0256**. Lecture d'état : LED `inputN::capslock` dont le parent HID est un modèle
Apple (correct, ignore `input49` et `input36`). Seul appelant : `flash_capslock(5)` (alerte
batterie) ; rien n'écrit NumLock.

État lu : `input49::capslock=0`, `input51::capslock=0`, `numlock` 0/0 → cohérents à l'instant
de l'audit.

Défauts (#125) :
1. cible keyd choisie même si keyd ne saisit pas le clavier (tous les PID ≠ 0x0256 avec la
   configuration livrée) → flash perdu en silence ;
2. le cœur input ignore un EV_LED égal à l'état courant du clavier virtuel : désynchronisé, le
   premier basculement est avalé ;
3. `set_led(0, true)` (API publique) déclencherait l'émulation NumLock de hid-apple sur BCM2042.

Python `--led` : rapport de sortie brut sur hidraw et état agrégé sur toutes les LED du système (#128).

## 4. Matrice modèles × fonctions

Légende : **OK** supporté et prouvé · **P** partiel · **NON** absent/cassé · **NT** non testé
(déduit du code/des sources, pas de matériel) · « — » sans objet.

| Famille (PID) | Détection Rust | Détection Python | Batterie noyau | Télémétrie HID vendor | RSSI | Fn / fnmode | Éjection | Réveil 0x13 | LED CapsLock (flash) | Remap keyd |
|---|---|---|---|---|---|---|---|---|---|---|
| **A1314 2011 ISO `05AC:0256`** (réel) | OK (test `real_a1314_iso_uevent`) | OK | OK (99 %, `-71`) | OK (autre audit) | OK (helper) | noyau OK ; driver NON (#130) | noyau OK ; driver NON | P (compté, non décodé #129) | OK via keyd (code) | OK (F4 partiel #126) |
| A1314 2011 ANSI/JIS `0255/0257` | OK (table) | OK (nom) | NT (même descripteur supposé) | NT | NT | idem | idem | NT | **NON** si keyd actif (#125) | **NON** (#126) |
| A1314 2009 `0239–023B` | OK | **NON** (absents, #127) | NT | NT | NT | NT | NT | NT | NON si keyd (#125) | NON (#126) |
| A1255 `022C–022E` | OK | **faux** (022C « JIS ») / absent (#127) | NT | NT | NT | NT | NT | NT | NON si keyd | NON |
| A1016 blanc (2003) | NON : PID absent de hid-ids.h, pilote `hid-generic`, pas de Fn hid-apple | « 0x0220 = A1016 » **faux** (c'est le filaire ALU) | NT | — | NT | — | — | NT | NON si keyd | NON |
| Magic Keyboard 2015 A1644/A1843 `0267/026C` BT `004C` | OK (famille MagicKeyboard, pas de feature vendor) | **NON** (vendor 004C ignoré ; 0267 étiqueté « Touch ID ») | NT (BT : batterie seulement si le descripteur la déclare ; USB : `APPLE_RDESC_BATTERY` + GET_REPORT noyau toutes les 60 s) | — (volontairement) | NT | noyau `magic_keyboard_2015_fn_keys` | pas de touche | sans objet (descripteur inconnu) | NON si keyd | NON |
| Magic Keyboard 2021 A2450 `029C` | OK | NON | NT ; descripteur publié : rapport `0x90` page Batterie (`0085:0065` charge, `0085:0044` en charge) → power_supply + statut charge | — | NT | Fn = `FF01:0003` dans `0x01` → `KEY_FN` | pas de touche ; touche Verrou `000C:019E` → `KEY_COFFEE` (= SCREENLOCK), non exploitée | **pas de rapport 0x13** : moniteur inutile (#129) | NON si keyd | NON (F6 = `KEY_SLEEP`) |
| Touch ID 2021 A2449/A2520 `029A/029F` | OK | NON | NT | — | NT | NT | — | NT | NON si keyd | NON |
| Magic Keyboard 2024 USB-C `0320–0322` | OK | NON | NT | — | NT | NT | — | NT | NON si keyd | NON |
| Câble USB (tout MK, `0003:05AC`) | OK (table) mais 1er hidraw du PID, plusieurs interfaces possibles ; `HID_UNIQ` sans `:` → pas de MAC → pas de batterie noyau trouvée par MAC ; BlueZ ne signale rien → machine jamais déclenchée | NON (bus 0003 exclu) | noyau OK (hid-apple timer) | — | — | — | — | — | NT | NON |
| Magic Mouse/Trackpad 1 `05AC:030D/030E` | rejeté OK ; **mais** watcher les prend pour un clavier (#124) | **pris pour un clavier** + sondes feature vendor (#127) | — | — | — | — | — | — | — | — |
| Magic Mouse/Trackpad 2 (+USB-C) `004C:0269/0265/0323/0324` | rejeté OK ; watcher #124 ; rapport souris MM2 = `0x12` (≠ médias A1314) | ignoré (vendor) | hid-magicmouse (USB) ; BT : #95 | — | — | — | — | — | — | — |
| AirPods / iPhone (Modalias `004C`) | watcher : **Connected → clavier masqué** (#124) | — | — | — | — | — | — | — | — | — |

Preuves de la colonne « Détection Rust » : `test_17_pid_egaux_a_hid_ids`,
`test_famille_coherente_avec_le_nom_noyau`, `test_detection_par_cas` (25 cas avec `HID_ID` de
`families.json`), plus les tests unitaires existants de `model.rs`. La table Rust (17 PID,
vendors 05AC et 004C) est **conforme à hid-ids.h** ; les deux vendors sont acceptés pour
chaque PID (le noyau, lui, n'enregistre les BCM2042 qu'en `05AC` BT et les Magic Keyboard qu'en
`004C` BT / `05AC` USB : sans conséquence, la combinaison inverse n'existe pas sur le fil).

Règle udev `70-apple-kb-hidraw.rules` : couvre tous les cas positifs
(`test_udev_uaccess_couvre_les_claviers`) mais aussi souris, trackpads et tout HID USB Apple
(`test_udev_trop_large_documente`) : `uaccess` sur leur hidraw. Pas d'issue ouverte (impact
limité à l'utilisateur de siège) ; à resserrer si la table des modèles devient la référence.

## 5. Fonctions utilisateur manquantes

| Fonction | Base matérielle | Suivi |
|---|---|---|
| Lire et afficher le `fnmode` effectif | sysfs `hid_apple/parameters/fnmode` | #130 (lecture), #88/#92 (bascule) |
| État Fn en direct (appuyée) | `0x11` bit 4 / `KEY_FN` | #130 |
| Touche Éjection exploitable (ex. Suppr, verrouillage) | `0x11` bit 3 / `KEY_EJECTCD` | #130, #102 |
| Réveil décodé (prêt / demande de connexion) | `0x13` bits 0/1 | #129, #130, #109 |
| Touche Verrou des MK 2021+ | `000C:019E` | #130 |
| Indicateur CapsLock | LED sysfs, déjà lue | #101 |
| Remap par famille (tables 2015/2021) | tables hid-apple | #126, #102 |
| Mode filaire USB (interface, numéro de série au lieu du MAC) | `0003:05AC` | #112 |
| Batterie Trackpad/Mouse | `0x90` / hid-magicmouse | #95 |
| Validation matérielle des familles non testées | descripteurs à capturer (§6) | #23 |

## 6. À capturer quand un modèle est disponible (lecture seule)

`/sys/class/hidraw/hidrawN/device/report_descriptor`, `uevent`, `bluetoothctl info`,
`/sys/class/power_supply/hid-*`, `evtest` en mode capacités. Les déposer sous
`tests/fixtures/models/<famille>/` et renseigner `rdesc` dans `families.json` : les tests
`test_rapport_0x13_seulement_bcm2042` et suivants s'appliqueront automatiquement.

## 7. Issues créées

| # | Type | Objet |
|---|---|---|
| [#124](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/124) | bug | watcher : tout appareil Apple (AirPods, souris) masque le clavier |
| [#125](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/125) | bug | LED : cible keyd sans vérifier la saisie ; EV_LED avalé ; garde-fou NumLock |
| [#126](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/126) | bug | keyd : `[ids]` limité à 05ac:0256, F4 asymétrique |
| [#127](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/127) | bug | Python : table des modèles fausse, souris/trackpad pris pour des claviers |
| [#128](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/128) | bug | Python `--led` : écriture hidraw brute, état agrégé global |
| [#129](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/129) | bug | wake-monitor : 0x13 non décodé, actif sans rapport 0x13 |
| [#130](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/130) | feature | Fn, Éjection, réveil, Verrou décodés sur D-Bus |
