# Renommer le clavier (#141, #192, #248)

Deux noms existent. Le premier est la voie par défaut ; le second est préparé, sa trame est **établie octet pour octet par désassemblage** (E1, `RE-NOM-PROPRE-E1.md`), et son écriture réelle reste derrière **trois verrous** (§4) parce que deux risques matériels ne sont **pas mesurés** (§5.3).

| | (a) Alias côté poste | (b) Nom propre, stocké dans le clavier |
|---|---|---|
| Commande | `akmctl rename <nom>` / `--reset` | `akmctl rename --device-name <nom>` (essai à blanc par défaut), `--show`, `--restore`, `--write-device-name` |
| Où | BlueZ, `org.bluez.Device1.Alias`, persisté dans `/var/lib/bluetooth/<adaptateur>/<MAC>/info` (`Alias=`) | micrologiciel du clavier (BCM2042) : lu dans `0x51-0x54` (4 × 8 o ASCII), écrit par Apple dans `0x55` `LongDeviceName` (65 o : id + 64) |
| Valeur actuelle | `alex` (alias BlueZ du gérant) | `Clavier de maria #1` (identique à `HID_NAME` et au nom distant BlueZ) [mesuré, docs/AUDIT-DECODAGE-HID.md] |
| Visible par | ce poste uniquement (KDE Bluetooth, `bluetoothctl`, tray, widget, `akmctl`) | tout appareil qui s'appaire au clavier |
| Risque | nul : propriété BlueZ réversible, ni déconnexion ni réappairage | §6 |
| État | **implémenté, voie par défaut** | **trame établie (E1) ; écriture réelle derrière 3 verrous, verrou 1 fermé par défaut** |

## 1. (a) Alias : ce qui est livré

* `akmctl rename <nom>` / `akmctl rename --reset` (`--mac` pour cibler un clavier). Codes retour : 0 OK, 1 erreur (nom refusé, BlueZ), 2 démon absent.
* D-Bus session : `com.agenceapi.AppleKbMonitor1.SetAlias(s mac, s nom) -> s` sur l'objet racine, `Device.SetAlias(s nom) -> s` sur l'objet clavier ; propriété `Name` (alias, sinon nom propre) sur les deux. Nom vide = retour au nom d'origine (comportement BlueZ).
* Validation (`akm-core::alias`, partagée par tous les clients) : espaces de bord retirés, max 64 caractères et 248 octets UTF-8 (limite HCI), refus des caractères de contrôle (Cc), séparateurs de ligne/paragraphe, caractères invisibles et surcharges bidirectionnelles (U+200B-200F, 2028-202E, 2060-2064, 2066-2069, FEFF). Le démon n'écrit que sur un appareil BlueZ dont le `Modalias` est Apple.
* Tray : « Renommer le clavier… » (boîte `kdialog`, sinon `zenity`, sinon ouverture de la fenêtre). Fenêtre egui : champ + Renommer / Réinitialiser. Widget Plasma : champ + Renommer / Réinitialiser. **Ces boutons ne touchent qu'à l'alias.**
* Affichage : infobulle du tray, en-tête du menu, `akmctl status [--json]` (`name`, `alias`), `apple-kb-monitord --json` (`name`, `keyboard.device.alias`), JSON du snapshot, widget.
* Un renommage fait ailleurs (`bluetoothctl`, Paramètres KDE) est repris via `PropertiesChanged.Alias`.
* Limite : `HID_NAME` du noyau (`/sys/.../uevent`) garde l'ancien nom jusqu'à la prochaine reconnexion ; l'application affiche l'alias en priorité.

## 2. (b) Nom propre : interfaces

| Interface | Effet | Accès matériel |
|---|---|---|
| `akmctl rename --device-name --show` | nom lu dans `0x51-0x54` + octets, depuis le cache du démon | **aucun** (cache) |
| `akmctl rename --device-name <nom> [--dry-run]` | valide le nom, affiche les 65 octets qui seraient envoyés (chaque champ **[désassemblage]** avec son adresse), le pré-vol (sans la MTU : elle demande `pkexec`), **sauvegarde** le nom actuel, l'état des trois verrous et les risques non mesurés | **aucun** |
| `akmctl rename --device-name <nom> --write-device-name` | séquence gardée (§4) : refus immédiat si le verrou 1 est fermé (rien n'est touché) ; sinon pré-vol **avec lecture de la MTU** (verrou 2), sauvegarde, confirmation (verrou 3), **une** écriture, reconnexion, relecture | une écriture `0x55` (65 o) si les trois verrous sont levés |
| `akmctl rename --device-name --restore <sauvegarde.json> [--write-device-name]` | retour arrière : réécrit exactement les 32 octets sauvegardés (même protocole, mêmes trois verrous, nouvelle confirmation) | idem |
| D-Bus : propriété **lecture seule** `DeviceNameOnKeyboard` (s, `""` = pas encore lu) | nom propre du cache | aucun |
| `akmctl status` : ligne `On kb:` ; `--json` : `name_on_keyboard` | idem | aucun |

* **Aucune méthode D-Bus n'écrit le nom propre**, aucun bouton de la fenêtre, du tray ni du widget : la seule voie est la commande interactive. (La spécification admettait une méthode D-Bus protégée par l'uid de l'appelant et un jeton à usage unique ; ne pas l'exposer du tout est plus strict et supprime ce risque.)
* Le démon lit `0x51-0x54` **une fois par connexion**, après `0x4F` et `0x60`, en priorité basse : ce qui ne tient pas dans le budget de 4 s attend la rafale suivante sans marquer la lecture incomplète (`registry::DAEMON_DEFERRED_ONCE_IDS`). C'est la lecture qu'Apple fait aussi (`deviceNameFromHardware`, [décompilé]). Le cache est remis à zéro à chaque connexion : c'est ce qui permet la relecture après l'écriture.
* `--device-name` et l'alias s'excluent : `akmctl rename Bureau --device-name x` est une erreur d'usage (64).

## 3. Validation du nom propre (`akm-core::devname::validate`, puis `apple_name_bytes`)

Deux étages, le second modèle exactement Apple :

1. **Notre validation** (plus stricte qu'Apple, pour que la relecture `0x51-0x54` vérifie le nom entier) : ASCII imprimable (`0x20`-`0x7E`), **1 à 32 caractères**, **rien n'est rogné**, pas d'espace en tête ni en fin, aucun caractère de contrôle ni non ASCII, refus de `\` (échappement du fichier `info` de BlueZ). `#` reste permis.
2. **Le codage d'Apple** (`apple_name_bytes`, [désassemblage] `0x4d3b9`-`0x4d43f`) : `[nom length]` en unités UTF-16 doit valoir 1..=64, sinon `kIOReturnBadArgument` **avant toute trame** ; puis UTF-8 et suppression des derniers caractères un à un tant que `strlen > 64` (jamais de caractère coupé) ; pas de terminateur, le bourrage vient du `calloc`. Testé sur des noms UTF-8 (`é`×40 → 32 `é` = 64 o ; 32 emoji = 64 unités acceptées → 16 emoji = 64 o ; 33 emoji = 66 unités → refus ; 63 `a` + `é` → 63 o). Sur le sous-ensemble ASCII ≤ 32 que l'étage 1 laisse passer, les deux coïncident.

## 4. Séquence gardée (`akm-core::devname::run`) et les trois verrous

Ordre strict, arrêt au premier échec, aucune répétition automatique, chaque décision et chaque octet journalisés (`[devname] …` sur stderr, puis `[hid-write] …` à la porte matérielle) :

1. nom validé, trame construite (`frames_for` = la trame Apple, §5.2) ;
2. **preuve** : `SEQUENCE_PROOF` vaut `EstablishedByDisassembly` (construction établie par E1) ; `NotProven` refuserait avant tout pré-vol. Ce n'est **pas** un verrou levé par l'utilisateur : c'est l'état de la connaissance ;
3. **verrou 1 — configuration** : `[apple] allow_device_name_write = true` dans `$XDG_CONFIG_HOME/apple-kb-monitor/config.toml` (défaut **false**, `CONFIGURATION.md`). Fermé → `REFUSED (lock 1)` avec les deux lignes à ajouter ; **rien n'est touché** : ni pré-vol, ni `pkexec`, ni sauvegarde ;
4. **pré-vol** : clavier connecté ; batterie ≥ 20 % (ou état `0x30` « normal », non publié par le démon : le pourcentage décide) ; disjoncteur fermé (`keyboard.breaker_open`) ; aucune lecture récente en échec (`keyboard.incomplete`, `kb_error`) ; `akmctl doctor` vert ; stdin **et** stdout sont un terminal ; **verrou 2 — MTU du canal de contrôle** : `akmctl` lance `pkexec /usr/lib/apple-kb-monitor/akm-hid-control inspect --mac <MAC>` (authentification administrateur, **une fois** par commande) ; le helper duplique (`pidfd_getfd`) la socket L2CAP PSM `0x0011` que `bluetoothd` tient vers ce clavier et lit `getsockopt(SOL_L2CAP, L2CAP_OPTIONS)` — **lecture seule**, aucun `setsockopt`, aucun octet envoyé (le verbe `inspect` n'atteint pas le chemin `send`) — puis imprime `mtu out N in M`. `akmctl` exige **`out` ≥ 66** (`MIN_CONTROL_MTU` : `0x53` + 65 octets). MTU inconnue (helper ancien sans `mtu`, socket absente ou double, `pkexec` refusé) ou < 66 → `REFUSED (lock 2)`, rien d'écrit. Pourquoi : `IOBluetoothHIDDriver::setReportWL` fragmente en DATC à `MTU − 1` [décompilé `0x5ad6`], mais `hidp` Linux envoie le rapport en **un** message L2CAP et le noyau refuse un message plus long que l'`omtu` négociée (`EMSGSIZE`), erreur que `hidp` traite comme fatale pour la session HID [source `net/bluetooth/hidp/core.c`, `l2cap_chan_send`]. La MTU négociée avec ce clavier **n'a jamais été capturée** (`RE-LIAISON-BLUETOOTH.md` §2 : canaux ouverts avant le début de `btmon`, « PSM 0 ») : elle est donc **lue au moment de l'écriture**, jamais supposée ;
5. cache `0x51-0x54` complet de la connexion courante (sinon arrêt) ;
6. **sauvegarde** avant toute écriture : `~/.local/state/apple-kb-monitor/devname-backup-<AAAAMMJJTHHMMSSZ>.json`, fichier neuf (`create_new`, jamais écrasé), **0600**, dossier 0700, `fsync` ; contenu : MAC, 4 fragments hex, nom, horodatage, source `daemon-cache` ;
7. **verrou 3 — confirmation** : `--write-device-name` passé **et** le nom retapé exactement dans le terminal (l'invite rappelle les risques non mesurés U5 et U3) ; tout autre texte annule ; jamais en non interactif ;
8. connexion revérifiée juste avant l'écriture (déconnecté → arrêt, rien écrit) ; ouverture du nœud hidraw sous le verrou HID partagé avec le démon (`hidraw::WriteDoor`), disjoncteur, espacement de 1 s après le dernier accès ;
9. **une** écriture via `WriteSession` (opération `DeviceName`, id `0x55`, exactement 64 octets de données, une seule fois par session) et la **porte de 65 octets** de `hid_write_feature` (`HIDIOCSFEATURE` dimensionné à 65 = `_IOWR('H', 6, 65)`, tableau fixe `[u8; 65]`, réservé à `DeviceName` ; la porte d'un octet reste celle de `Shutdown`/`Forget`) ;
10. attente de la reconnexion (au plus 180 s) avec la consigne « éteignez le clavier (3 s), attendez 5 s, rallumez-le » : le nom ne se relit qu'après une reconnexion ;
11. **vérification** (U4) : relecture `0x51-0x54` (cache de la nouvelle connexion) comparée octet pour octet aux 32 premiers octets écrits — c'est le critère ; le `Device1.Name` de BlueZ après reconnexion est **affiché à titre d'information** (Apple, lui, lance un Remote Name Request HCI ; BlueZ peut servir son cache jusqu'à sa prochaine requête de nom) ; différence → **retour arrière guidé** affiché : `akmctl rename --device-name --restore <sauvegarde> --write-device-name` (nouvelle commande, nouvelle session, mêmes trois verrous).

Barrières indépendantes : (1) verrou 1 fermé par défaut ; (2) verrou 2 mesuré sur la socket vivante, jamais constant ; (3) verrou 3 interactif ; (4) registre : `0x55` n'est écrivable que par l'opération `DeviceName`, 64 octets de données exactement, une fois par session ; (5) porte matérielle : deux ioctl de taille fixe seulement (1 octet ; 65 octets pour `DeviceName` + `0x55`), balayage des 256 ids × 3 opérations testé ; (6) aucun chemin D-Bus, fenêtre, tray ni widget.

## 5. Ce qu'Apple envoie : établi par désassemblage (E1)

### 5.1 Sources

* **Lion 10.7.5** `IOBluetooth.framework` x86_64, `-[AppleBluetoothHIDDevice setDeviceName:]` à `0x4d2fe` (1 211 o), comparé à **10.5.8** i386 (`0x4fc10`) : `RE-NOM-PROPRE-E1.md` §3-4, instructions citées avec adresses, recontrôlées à l'`objdump`. Les 65 octets envoyés sont identiques d'un système à l'autre : protocole stable de 2009 à 2012.
* Personnalité du 598 [plist] : `0x50` (sans taille), `0x51-0x54` (8 o), `0x55` (64 o). Mesures : `0x51-0x54` se lisent, `0x50`/`0x55` refusent le GET (`0x03`) [mesuré].
* **macOS 26.5 n'écrit jamais le nom** : `setDeviceName:` ne change que le cache hôte (RE-MACOS-SILICON.md §3.4) ; IOBluetooth ne fait que **lire** `0x51-0x54` (`deviceNameFromHardware`).
* Délai : 1000 ms est le **délai d'attente** passé à `IOHIDDeviceInterface::setReport` [désassemblage `0x4d4ba`] (attente du HANDSHAKE), pas un espacement : il n'y a qu'une trame. Sur le fil HIDP, le SET Feature est préfixé `0x53` par la pile [décompilé `setReportWL`].

### 5.2 Trames de référence (fixture `tests/fixtures/devname/lion_setdevicename_frames.json`)

Test `devname::tests::frames_match_the_lion_fixture_byte_for_byte` : `frames_for(nom)` reproduit les deux exemples de la fixture octet pour octet (rapport de 65, fil de 66, relecture attendue) ; `three_locks_lifted_the_fixture_frame_is_sent_once_after_the_backup` vérifie que l'espion reçoit exactement ces octets, une fois, après la sauvegarde.

```
« alex »
report : 55 61 6c 65 78 00 × 60
wire   : 53 55 61 6c 65 78 00 × 60

« Clavier Apple A1314 du gerant 01 » (32 o)
report : 55 43 6c 61 76 69 65 72 20 41 70 70 6c 65 20 41 31 33 31 34 20 64 75 20 67 65 72 61 6e 74 20 30 31 00 × 32
wire   : 53 55 43 6c 61 76 69 65 72 20 41 70 70 6c 65 20 41 31 33 31 34 20 64 75 20 67 65 72 61 6e 74 20 30 31 00 × 32
```

| Octets | Contenu | Preuve |
|---|---|---|
| fil 0 | `53` SET_REPORT Feature, ajouté par la pile | [décompilé] `setReportWL` `0x5ad6` |
| 0 | `55` `LongDeviceName` | [désassemblage] `movb %bl,(%r13)` `0x4d444` |
| 1-n | le nom en **UTF-8**, caractères entiers, sans terminateur | [désassemblage] `_UTF8StringFromString` `0x4d416`, `strncpy(buf+1, utf8, strlen)` `0x4d43f` |
| n+1-64 | bourrage **`0x00`** (`calloc(65, 1)`) | [désassemblage] `0x4d407` |
| longueur | 65 octets remis à `setReport` | [désassemblage] `movzbl %r12b,%r8d` `0x4d4b1` |
| après | aucune autre trame (pas de `0x50`, pas de `0x51-0x54`, aucune lecture) ; Remote Name Request HCI | [désassemblage] `0x4d4e6`-`0x4d4f8` |

La trame d'hypothèse de l'ancien §5.2 (`Clavier de maria #1`) est confirmée octet pour octet ; elle reste un test (`frame_matches_the_documented_reference_byte_for_byte`), plus aucun champ n'est marqué `[hypothèse]`.

### 5.3 Inconnues : tranchées et restantes

| # | Inconnue | Verdict |
|---|---|---|
| U1 | contenu des 64 octets | **tranchée [désassemblage]** : UTF-8, bourrage `0x00`, pas de préfixe de longueur, pas de terminateur explicite |
| U2 | ordre des trames | **tranchée [désassemblage]** : une seule trame `0x55` de 65 o, puis HCI Remote Name Request ; nom vide ou > 64 unités UTF-16 refusé avant toute trame |
| U7 | rôle de `0x50` | **tranchée [désassemblage]** : validation des 4 fragments uniquement, jamais envoyé à un clavier déclarant `LongDeviceName` |
| U6 | MTU / fragmentation | **côté Apple tranchée** (1 trame de 66 o si MTU ≥ 66, sinon DATC) ; **côté Linux : lue au pré-vol** (verrou 2), jamais mesurée a priori pour ce clavier |
| U4 | `0x51-0x54` reflètent-ils `0x55`, et quand | **partielle** : Apple ne relit pas `0x51-0x54`, il attend le nouveau nom HCI sans reconnexion. Décision : notre vérification est la **relecture `0x51-0x54` après reconnexion** (critère de réussite, retour arrière si différent) **et** l'affichage informatif du `Device1.Name` BlueZ. Que les fragments reflètent `0x55` ne sera su qu'à la première écriture |
| U3 | persistance (piles) | **RISQUE NON MESURÉ** : Apple ne réécrit jamais le nom à la reconnexion (donc le clavier est censé le mémoriser, [déduction]) ; le descripteur du Magic Keyboard déclare son `0x55` *volatile*. Rien ne garantit la survie d'un changement de piles (E3) |
| U5 | réponse du micrologiciel `0x0050` à un SET `0x55` | **RISQUE NON MESURÉ** : le refus du GET (`0x03`) ne dit rien du SET ; Apple attend un HANDSHAKE SUCCESSFUL dans les 1000 ms et traite tout autre résultat comme un échec sans réessai. Une réponse d'erreur serait visible dans `[hid-write] failed` ; un HANDSHAKE absent se traduirait par un délai d'attente du noyau |

### 5.4 Expériences

| # | Lève | État |
|---|---|---|
| E1 | U1, U2, U7, U6 (Apple) | **faite** : `RE-NOM-PROPRE-E1.md`, fixture §5.2 |
| E2 | U6 (Linux) | **devenue un pré-vol** : `akm-hid-control inspect --mac <MAC>` (getsockopt lecture seule) ; une capture `btmon` d'une reconnexion donnerait la même valeur dans les `Configure Response` du PSM 17 et peut la documenter ici comme [mesuré] |
| E3 | U3 | à faire après la première écriture : comparer `0x51-0x54` (cache), `HID_NAME` et le `Name` BlueZ avant/après un changement de piles |
| E4 | U3, U5 | descripteurs HID publics de claviers Apple de la même génération déclarant `0x55` : drapeaux Feature (volatile ou non) |

## 6. Risques de l'écriture réelle

1. **Réponse du micrologiciel inconnue (U5)** : écrire un rapport vendeur sur un micrologiciel ancien sans mode de récupération connu. Une erreur de numéro toucherait un voisin (`0x5A/0x60/0xEB` étalonnage, `0xD0-0xFB`) : le registre ne le permet pas (opération `DeviceName` = `0x55` seul, 64 octets, porte de 65 octets).
2. **Perte de pairage** : si le clavier redémarre ou si le nom entre dans les données de lien, le pairage peut sauter ; un clavier Bluetooth sans pairage ne se ré-appaire pas sans autre clavier. Avoir un **second clavier** fonctionnel.
3. **Session HID coupée** si la trame dépasse la MTU : exclu par le verrou 2 (refus si inconnue ou < 66).
4. **Nom perdu** au changement de piles si `0x55` est volatile (U3) : la sauvegarde permet de réécrire, sous réserve que l'écriture fonctionne.
5. **Gain faible** : l'alias (a) donne déjà le même affichage sur ce poste.

## 7. Procédure pour le gérant : quatre commandes

Préalables : un second clavier fonctionnel, piles ≥ 20 %, `akmctl doctor` vert, le paquet **réinstallé** avec cette version (le helper `akm-hid-control` doit connaître le verbe `inspect`, sinon `REFUSÉ (verrou 2)` « reinstall the package »). Tout se passe **dans un terminal** (konsole) ; les messages sont en français si `LANG` commence par `fr`, en anglais sinon. **Rien à éditer dans `config.toml`.**

| # | Commande | Effet | Écrit ? |
|---|---|---|---|
| 1 | `akmctl rename --device-name --show` | nom propre actuel (cache du démon) et ses 32 octets | non |
| 2 | `akmctl rename --device-name "<nom>" --check` | **tout le pré-vol** : doctor, batterie, disjoncteur, lecture récente, puis **une seule** demande `pkexec` (authentification administrateur) pour lire la MTU du canal de contrôle (verrou 2, lecture seule) ; **sauvegarde** du nom actuel ; affichage du **plan** (65 octets du rapport, 66 sur le fil, chemin de la sauvegarde, commande de retour arrière, risques U5/U3) ; s'arrête **avant** toute demande de consentement. Code 0 = tout serait accepté ; 11 = refusé (la raison est affichée) | non |
| 3 | `akmctl rename --device-name "<nom>" --write-device-name` | même pré-vol et même plan, puis : taper exactement **`ECRIRE`** (verrou 1, consentement pour **cette exécution seulement**, rien n'est écrit dans `config.toml`), **retaper le nom** (verrou 3), **une** écriture, puis la consigne « **éteignez le clavier 3 s, attendez 5 s, rallumez-le** » avec compte à rebours (≤ 180 s), relecture `0x51-0x54`, verdict `✓` ou `✗` | **oui, une trame `0x55`**, seulement après `ECRIRE` + nom |
| 4 | *(seulement si `✗`)* `akmctl rename --device-name --restore ~/.local/state/apple-kb-monitor/devname-backup-<horodatage>.json --write-device-name` | retour arrière : la commande **exacte** est affichée dans le plan et dans le verdict `✗` (§8) ; même protocole, nouvelle confirmation | oui, les 32 octets sauvegardés |

Ce qui reste verrouillé, et comment chaque verrou se lève :

* **Verrou 1 (consentement).** Deux formes, au choix : (a) dans un terminal **interactif** (stdin **et** stdout sont un TTY), taper exactement `ECRIRE` (sensible à la casse, sans accent) après l'affichage du plan, de la sauvegarde et des risques : ce consentement vaut pour l'exécution en cours seulement, **aucune écriture dans `config.toml`** ; (b) pour l'automatisation sans terminal, la clé `[apple] allow_device_name_write = true` reste la seule voie. Sans TTY et sans la clé : `REFUSÉ (verrou 1)` avant tout pré-vol, tout `pkexec`, toute sauvegarde (code 10), exactement comme avant. Avec la clé, `ECRIRE` n'est pas redemandé mais le nom retapé l'est toujours.
* **Verrou 2 (MTU ≥ 66)** : inchangé, lu sur la socket vivante par **une seule** demande `pkexec` par commande (`--check` comme `--write-device-name` ; `--dry-run` ne la lit pas).
* **Verrou 3 (nom retapé)** : inchangé ; `ECRIRE` ne le remplace pas.
* Barrières fixes : preuve `EstablishedByDisassembly`, registre (`0x55` seul, 64 octets, une fois par session), porte matérielle de 65 octets, aucun chemin D-Bus/fenêtre/tray/widget. Le module des Paramètres système (onglet « Nom ») n'écrit rien : ses boutons « **Vérifier (sans écrire)** » et « **Écrire le nom dans le clavier…** » ouvrent un terminal (`konsole`, sinon `xterm`, sinon `x-terminal-emulator`) sur les commandes 2 et 3 avec le nom validé (ASCII imprimable, 1-32) passé en **un seul argument** (`--device-name=<nom>`, `QProcess`, jamais de shell) ; tout le reste (plan, `ECRIRE`, nom, écriture) se passe dans ce terminal.

Codes de sortie de `--device-name` : 0 réussi (ou `--check` vert) · 1 erreur générique · 2 démon absent · **10** refusé verrou 1 · **11** refusé pré-vol (verrou 2 compris) · **12** annulé au clavier (`ECRIRE` ou nom incorrect ; rien d'écrit, la sauvegarde reste) · **13** écrit mais pas de reconnexion dans le délai · **14** écrit, relecture différente (retour arrière affiché) · 64 usage.

## 8. Retour arrière

`akmctl rename --device-name --restore ~/.local/state/apple-kb-monitor/devname-backup-<horodatage>.json --write-device-name` : vérifie la sauvegarde (4 × 8 octets, ASCII puis NUL seulement, cohérente avec le nom), affiche les octets, puis suit **le même protocole** (verrou 1, pré-vol avec MTU, confirmation en retapant le nom sauvegardé, une écriture de 65 octets, reconnexion, relecture). Sans `--write-device-name` : affichage seul. Aucune nouvelle sauvegarde n'est faite pendant un retour arrière ; aucun retour arrière n'est automatique.
