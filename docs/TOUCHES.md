# Touches spéciales, KDE et mapping manuel (#247)

Clavier : Apple Wireless Keyboard A1314 ISO, `05ac:0256` (aluminium 2011, Bluetooth).
Poste de référence : Manjaro, noyau 7.1.13, KDE Plasma 6 (Wayland), `hid_apple fnmode=1`, sans keyd.

Le mapping **par défaut ne change rien** : le paquet n'installe aucun fichier hwdb et ne modifie
aucun paramètre. Les présets ne s'appliquent que sur demande (`akmctl keymap preset <nom>` puis
`akmctl keymap apply`), jamais à l'installation.

## 1. Ce que fait chaque touche aujourd'hui

`fnmode=1` (touches multimédia par défaut, Fn + touche = F1…F12). Source : `akmctl keys --check`
(table du noyau + liaisons KDE lues dans KGlobalAccel le 2026-10-01, sans appuyer sur une touche).

| Touche | Légende Apple | Code evdev | Keysym (xkb) | Action KDE | État |
|---|---|---|---|---|---|
| F1 | luminosité − | `KEY_BRIGHTNESSDOWN` | XF86MonBrightnessDown | PowerDevil « Diminuer la luminosité de l'écran » | **[mesuré par le gérant]** : l'écran LG 34GK950F (DDC) baisse |
| F2 | luminosité + | `KEY_BRIGHTNESSUP` | XF86MonBrightnessUp | PowerDevil « Augmenter la luminosité de l'écran » | **[mesuré par le gérant]** |
| F3 | Exposé / Mission Control | `KEY_SCALE` | XF86LaunchA → Qt « Launch (C) » | KWin `ExposeAll` (présenter les fenêtres, tous les bureaux) | à tester |
| F4 | Dashboard / Launchpad | `KEY_ALL_APPLICATIONS` (= `KEY_DASHBOARD`) | XF86LaunchB → Qt « Launch (D) » | **aucune** (Launch (D) n'est lié à rien) | correctif : `akmctl keymap kde-apply` |
| F5 | (sans pictogramme) | `KEY_F5` (pas de traduction dans la table du noyau) | F5 | aucune : touche F5 pour l'application | à tester |
| F6 | (sans pictogramme) | `KEY_NUMLOCK` | Num_Lock | aucune (KWin bascule le Verr. Num) | **piège**, voir §4 |
| F7 | piste précédente | `KEY_PREVIOUSSONG` | XF86AudioPrev | « Contrôleur de média » `previousmedia` | à tester |
| F8 | lecture / pause | `KEY_PLAYPAUSE` | XF86AudioPlay | `playpausemedia` | à tester |
| F9 | piste suivante | `KEY_NEXTSONG` | XF86AudioNext | `nextmedia` | à tester |
| F10 | muet | `KEY_MUTE` | XF86AudioMute | « Volume audio » `mute` | à tester |
| F11 | volume − | `KEY_VOLUMEDOWN` | XF86AudioLowerVolume | `decrease_volume` | à tester |
| F12 | volume + | `KEY_VOLUMEUP` | XF86AudioRaiseVolume | `increase_volume` | à tester |
| ⏏ Éjecter | éjecter | `KEY_EJECTCD` (usage 0x0C:0xB8) | XF86Eject | **aucune** | remappable (§5) |

Avec **Fn** : F1…F12 envoient `KEY_F1`…`KEY_F12` à l'application (sauf liaison globale : sur ce
poste Fn+F12 = F12 ouvre Yakuake). Fn+⌫ = Suppr, Fn+↩ = Inser, Fn+↑/↓ = Page préc./suiv.,
Fn+←/→ = Début/Fin. Éjecter ne dépend pas de Fn.

Les touches F5/F6 du clavier de rétroéclairage (`KEY_KBDILLUM*`) n'existent pas sur l'A1314 (pas
de rétroéclairage, la table aluminium du noyau ne les traduit pas). Si une touche produisait
`KEY_KBDILLUMUP`, KDE appellerait PowerDevil « Augmenter la luminosité du clavier » (liaison
présente) qui ne trouverait aucun rétroéclairage : rien de visible. Une touche **sans** action KDE
est simplement transmise à l'application qui a le focus ; si celle-ci ne l'utilise pas, rien ne se
passe (pas d'erreur, pas d'OSD).

## 2. La chaîne noyau → evdev → KDE

1. **HID → code evdev** (`drivers/hid/hid-input.c`, `hid_keyboard[]`) : l'usage HID devient un code
   `KEY_*`. Le *scancode* vu par udev est l'usage : `0x7003a` = F1 … `0x70045` = F12,
   `0xc00b8` = Éjecter, `0xff0003` = Fn (`apple_input_mapping`).
2. **udev hwdb** (facultatif) : `KEYBOARD_KEY_<scancode>=<nom>` remplace ce code par
   `EVIOCSKEYCODE` (règle `60-evdev.rules`, builtin `keyboard`). C'est la seule couche que
   `akmctl keymap` écrit.
3. **`hid_apple`** (`drivers/hid/hid-apple.c`, `hidinput_apple_event`) : dans l'ordre
   `swap_fn_leftctrl`, `iso_layout`, `swap_opt_cmd`, `swap_ctrl_cmd`, puis la couche Fn du modèle.
   Pour le PID `0x0256` (`USB_DEVICE_ID_APPLE_ALU_WIRELESS_2011_ISO`, quirks
   `APPLE_NUMLOCK_EMULATION | APPLE_HAS_FN | APPLE_ISO_TILDE_QUIRK`), la table est
   `magic_keyboard_alu_fn_keys` : F1→BRIGHTNESSDOWN, F2→BRIGHTNESSUP, F3→SCALE, F4→DASHBOARD,
   F6→NUMLOCK, F7→PREVIOUSSONG, F8→PLAYPAUSE, F9→NEXTSONG, F10→MUTE, F11→VOLUMEDOWN,
   F12→VOLUMEUP (drapeau `APPLE_FLAG_FKEY`), et sans drapeau ⌫→DELETE, ↩→INSERT, ↑→PAGEUP,
   ↓→PAGEDOWN, ←→HOME, →→END. **F5 n'est pas dans la table** (toujours `KEY_F5`).
   Selon `fnmode` pour les entrées `APPLE_FLAG_FKEY` :

   | fnmode | sans Fn | avec Fn |
   |---|---|---|
   | 0 disabled | F1…F12 | F1…F12 (et Fn+⌫ etc. inactifs) |
   | 1 fkeyslast (**actuel**) | média / luminosité | F1…F12 |
   | 2 fkeysfirst | F1…F12 | média / luminosité |
   | 3 auto | = 1 sur un clavier Apple (= 2 sur un clavier « non Apple ») | |
   | 4 fkeysdisabled | F1…F12 | F1…F12 (Fn+⌫ etc. actifs) |

4. **evdev → keysym** : le compositeur (KWin, libinput) prend le code evdev + 8 comme keycode xkb ;
   `/usr/share/X11/xkb/symbols/inet` (section `evdev`) donne le keysym (`<I232>` →
   XF86MonBrightnessDown, `<I128>` → XF86LaunchA, `<I212>` → XF86LaunchB, `<I169>` → XF86Eject…).
5. **keysym → touche Qt → KGlobalAccel** : Qt fait XF86LaunchA → `Key_LaunchC`, XF86LaunchB →
   `Key_LaunchD` (d'où la liaison par défaut de KWin `ExposeAll = Launch (C)`, pensée pour la touche
   Exposé des claviers Apple). KGlobalAccel déclenche l'action liée : PowerDevil, KWin, MPRIS…
6. **PowerDevil → écran** : `org.kde.ScreenBrightness/display0` = « LG Electronics 34GK950F »
   (externe, DDC/CI) : PowerDevil sait déjà régler cet écran, F1/F2 le pilotent.

Preuves relevées sans lire de frappe (2026-10-01) :

- `modinfo hid_apple` (7.1.13) : paramètres `fnmode` (0-4), `iso_layout`, `swap_opt_cmd`,
  `swap_ctrl_cmd`, `swap_fn_leftctrl` ; `rightalt_as_rightctrl` et `ejectcd_as_delete`
  n'existent pas dans ce pilote.
- `/sys/class/input/event28/device/capabilities/key` (sysfs, sans ouvrir le nœud) : bits 0xE0/0xE1
  (luminosité), 0x78 (SCALE), 0xCC (ALL_APPLICATIONS), 0x45 (NUMLOCK), 0xA3-0xA5 (médias),
  0x71-0x73 (volume), 0xA1 (EJECTCD), 0x1D0 (FN) présents.
- KGlobalAccel `action(i)` (D-Bus, lecture) : MonBrightnessDown/Up → `org_kde_powerdevil`,
  LaunchC → `kwin ExposeAll`, LaunchD → rien, médias → `mediacontrol`, volume → `kmix`,
  Eject → rien.
- `systemd-hwdb query 'evdev:<modalias de event28>'` : seulement `KEYBOARD_LED_NUMLOCK=0`
  (`60-keyboard.hwdb`, entrée des claviers Apple sans fil) : aucun remappage actif.

Conclusion : **rien à corriger dans la chaîne pour F1/F2, F3, F7-F12**. Les manques sont côté KDE
(F4, Éjecter) et le piège F6.

## 3. `akmctl keys`

```text
akmctl keys            # F1-F12 + Éjecter : code sans Fn / avec Fn, keysym, action KDE
akmctl keys --check    # + verdict [ok] (action KDE liée) / [app] (pour l'application) / [--] (rien)
akmctl keys --all      # toutes les touches connues (modificateurs, flèches, Fn, ⌫, ↩…)
akmctl keys --json     # pour les scripts (table + actions KDE)
```

La table est calculée (modèle de `hid-apple.c` + paramètres sysfs + hwdb installé) ; l'action KDE
est demandée à KGlobalAccel (`action(i)` de la touche Qt). Aucune frappe n'est lue, aucun nœud
`event`/`hidraw` n'est ouvert. La fenêtre a la même table dans l'onglet **Touches**.

## 4. Pièges connus

- **F6 = Verr. Num.** Le quirk `APPLE_NUMLOCK_EMULATION` de `hid_apple` transforme, tant que la
  LED Num est allumée, J K L U I O 7 8 9 M 0 ; / P - en touches du pavé numérique. Si des lettres
  « tapent des chiffres », rappuyer F6. Neutraliser F6 : `akmctl keymap set F6 KEY_F6`… ne suffit
  **pas** (le noyau retraduit `KEY_F6`) ; il faut une cible hors table, p. ex.
  `akmctl keymap set F6 KEY_F19` (F6 devient F19 avec ou sans Fn).
- **Le hwdb agit avant `hid_apple`** : une touche remappée vers un code absent de la table Fn perd
  sa couche Fn (elle envoie la même chose avec ou sans Fn) ; remappée vers `KEY_F1`…`KEY_F12`, elle
  hérite de la couche Fn de la touche visée. `akmctl keys` le montre.
- **F13-F18 ne sont pas « F13 » pour xkb** : `<FK13>` = XF86Tools, `<FK14>`-`<FK18>` =
  XF86Launch5-9 ; `KEY_F19` est le premier vrai F19. Pour un raccourci libre, viser `KEY_F19`…
- **Le remappage survit à la suppression du fichier** (`EVIOCSKEYCODE`) jusqu'à la reconnexion du
  clavier : `akmctl keymap reset`/`rollback` réécrivent donc d'abord les codes par défaut.
- **Les paramètres `hid_apple` sont globaux** à tous les claviers Apple de la machine.

## 5. Mapping manuel (sans keyd, sans saisie exclusive, sans uinput)

Fichier : `~/.config/apple-kb-monitor/keymap.toml` (créé par les commandes ; sous-ensemble TOML
strict, toute erreur est refusée avec son numéro de ligne).

```toml
schema = 1
active = "default"

[profile.default]
preset = "apple"                       # apple | fkeys | linux-pc
models = ["05ac:0256"]                 # PID des claviers aluminium visés

[profile.default.params]               # facultatif, par-dessus le préset
# fnmode = 1

[profile.default.keys]                 # touche physique ou usage HID → code KEY_*
Eject = "KEY_DELETE"
F6 = "KEY_F19"
"0x70039" = "KEY_LEFTCTRL"             # Verr. maj → Ctrl
```

Touches nommées : `F1`…`F12`, `Eject`, `Fn`, `Esc`, `Backspace`, `Enter`, `CapsLock`, `Grave`,
`NonUS`, `LeftCtrl`, `LeftShift`, `LeftAlt` (Option), `LeftCmd`, `RightShift`, `RightAlt`,
`RightCmd`, `Up`, `Down`, `Left`, `Right` ; sinon un usage HID en liste blanche (page clavier
`0x7xxxx` ayant un code par défaut, `0xc00b8`, `0xff0003`). Cibles : noms `KEY_*` de
`linux/input-event-codes.h` (les mêmes que ceux qu'accepte udev).

Présets (paramètres `hid_apple` qu'ils fixent ; aucune touche remappée) :

| Préset | Titre | Paramètres |
|---|---|---|
| `apple` (défaut) | Apple (légende des touches) | `fnmode=1 swap_opt_cmd=0` = l'état actuel |
| `fkeys` | F1-F12 classiques | `fnmode=2 swap_opt_cmd=0` |
| `linux-pc` | Linux PC (Cmd↔Alt) | `fnmode=1 swap_opt_cmd=1` (ordre Ctrl Méta Alt d'un PC) |

Le préset « Apple complet » (F3 Mission Control, F4 Launchpad) = `apple` + `akmctl keymap kde-apply`.

```text
akmctl keymap show [--hwdb]            # profils ; --hwdb : le fichier que `apply` installerait
akmctl keymap set <touche> <KEY_…>     # [--profile NOM]
akmctl keymap unset <touche>
akmctl keymap preset apple|fkeys|linux-pc
akmctl keymap use <profil>
akmctl keymap apply [--dry-run]        # installe (mot de passe administrateur)
akmctl keymap reset                    # retire le hwdb, mapping du noyau rétabli tout de suite
akmctl keymap rollback                 # revient au fichier installé précédemment
akmctl set param <nom> <valeur> [--persist]   # fnmode 0-4, iso_layout -1..1, swap_opt_cmd 0-2, swap_ctrl_cmd 0-1, swap_fn_leftctrl 0-1
akmctl get param [<nom>]
```

`apply` calcule un plan (vide pour le profil par défaut : rien n'est demandé), écrit le hwdb dans
`/run/user/<uid>/apple-kb-monitor/keymap.hwdb` et lance `pkexec akm-keymap-helper install`
(action polkit `com.agenceapi.AppleKbMonitor.install-keymap`) ; les paramètres passent par
`pkexec akm-helper set-params … --persist` (action `set-fnmode`, `/etc/modprobe.d/hid_apple.conf`).
Le helper ne reçoit aucun chemin : il lit ce seul fichier (propriétaire = l'appelant, un lien,
non inscriptible par d'autres, 16 Kio), le valide ligne par ligne, le réécrit sous forme
canonique, garde l'ancien en `/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb.akm-bak`, écrit
`/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb` (0644 root:root, rename atomique) puis lance
`systemd-hwdb update` et `udevadm trigger --settle --subsystem-match=input --action=change`.

Fichier produit (exemple) :

```text
# Generated by apple-kb-monitor (akmctl keymap apply): do not edit, use `akmctl keymap`.
# profile: default

evdev:input:b0005v05ACp0256*
 KEYBOARD_KEY_7003f=f19
 KEYBOARD_KEY_c00b8=delete
```

Validation faite (lecture seule, rien dans `/etc`) : ce fichier placé dans une racine temporaire,
`systemd-hwdb update --root=<tmp> --strict` puis `systemd-hwdb query --root=<tmp>
'evdev:<modalias réel de event28>'` renvoie exactement les `KEYBOARD_KEY_*` attendus ; la base
du système reste inchangée (`KEYBOARD_LED_NUMLOCK=0` seul). `udevadm test` n'a **pas** été lancé
sur le nœud : il exécute le builtin `keyboard`, qui ouvre `/dev/input/event28` et appliquerait
`EVIOCSKEYCODE` au vrai clavier.

D-Bus (démon, interface `com.agenceapi.AppleKbMonitor1.Keymap` sur
`/com/agenceapi/AppleKbMonitor1`) : `KeyTable(b) → s`, `Keymap() → s`, `SetKey(s profil, s touche,
s code) → s` (`""` = rétablir), `SetPreset(s, s) → s`, `UseProfile(s) → s`, `Apply() → s`,
`Reset() → s`. L'onglet **Touches** de la fenêtre s'en sert.

keyd reste une alternative facultative (exemple dans `/usr/share/doc/apple-kb-monitor/examples/keyd/`,
voir KEYD.md) ; rien ici n'en dépend.

## 6. Raccourcis KDE manquants

```text
akmctl keymap kde-apply --dry-run      # ce qui serait ajouté
akmctl keymap kde-apply                # ajoute
akmctl keymap kde-apply --undo         # retire exactement ce qui a été ajouté
```

Seule liaison fournie : **F4 (Launch (D)) → plasmashell « Activer le lanceur d'applications »**
(équivalent du Launchpad). Elle n'est ajoutée que si Launch (D) n'est lié à rien, et elle s'ajoute
aux touches existantes de l'action (Méta, Alt+F1 restent). Aucune autre liaison n'est modifiée.
Éjecter n'a volontairement pas d'action imposée (la remapper au besoin, §5).

## 7. À tester (appuyer sur chaque touche, cocher)

Prérequis : `akmctl keys --check` (affiche la même table, n'appuie sur rien). Fenêtre de test :
un navigateur ouvert, un lecteur audio (MPRIS) qui joue, un éditeur de texte.

- [x] **F1** : la luminosité de l'écran LG baisse, OSD Plasma. *(mesuré par le gérant)*
- [x] **F2** : la luminosité remonte. *(mesuré par le gérant)*
- [ ] **F3** : « Présenter les fenêtres » (toutes les fenêtres, tous les bureaux) s'ouvre ; F3 à nouveau ferme.
- [ ] **F4** : rien ne se passe (attendu aujourd'hui). Après `akmctl keymap kde-apply` : le lanceur d'applications s'ouvre.
- [ ] **F5** : dans le navigateur, la page se recharge (touche F5 transmise).
- [ ] **F6** : le Verr. Num s'allume (OSD Plasma s'il est activé) ; dans l'éditeur, `j` tape `1`. **Rappuyer F6** (`j` retape `j`).
- [ ] **F7 / F8 / F9** : piste précédente / lecture-pause / piste suivante dans le lecteur, OSD média.
- [ ] **F10** : coupe le son (OSD), F10 à nouveau le rétablit.
- [ ] **F11 / F12** : volume − / + (OSD).
- [ ] **⏏ Éjecter** : rien ne se passe (aucune action KDE).
- [ ] **Fn+F1** … **Fn+F12** : la touche F correspondante est envoyée à l'application (Fn+F5 recharge, Fn+F11 plein écran du navigateur ; Fn+F12 ouvre Yakuake sur ce poste).
- [ ] **Fn+⌫** supprime à droite ; **Fn+↩** = Inser ; **Fn+↑/↓** = page préc./suiv. ; **Fn+←/→** = début/fin de ligne.
- [ ] Mapping manuel (facultatif, réversible) : `akmctl keymap set Eject KEY_DELETE && akmctl keymap apply`,
      puis ⏏ supprime à droite et `akmctl keys` affiche `Eject [hwdb: KEY_DELETE]` ;
      `akmctl keymap reset` : ⏏ redevient sans effet, sans reconnecter le clavier.
