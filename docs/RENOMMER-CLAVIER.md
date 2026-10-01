# Renommer le clavier (#141, #192, #248)

Deux noms existent. Le premier est la voie par défaut ; le second est préparé, mais son écriture réelle est **refusée** tant que la séquence exacte d'Apple n'est pas prouvée octet pour octet (§5).

| | (a) Alias côté poste | (b) Nom propre, stocké dans le clavier |
|---|---|---|
| Commande | `akmctl rename <nom>` / `--reset` | `akmctl rename --device-name <nom>` (essai à blanc par défaut), `--show`, `--restore` |
| Où | BlueZ, `org.bluez.Device1.Alias`, persisté dans `/var/lib/bluetooth/<adaptateur>/<MAC>/info` (`Alias=`) | micrologiciel du clavier (BCM2042) : lu dans `0x51-0x54` (4 × 8 o ASCII), écrit par Apple dans `0x55` `LongDeviceName` (64 o) |
| Valeur actuelle | `Clavier de maria #1` | `Clavier de maria #1` (identique à `HID_NAME` et au nom distant BlueZ) [mesuré, docs/AUDIT-DECODAGE-HID.md] |
| Visible par | ce poste uniquement (KDE Bluetooth, `bluetoothctl`, tray, widget, `akmctl`) | tout appareil qui s'appaire au clavier |
| Risque | nul : propriété BlueZ réversible, ni déconnexion ni réappairage | §6 |
| État | **implémenté, voie par défaut** | **préparation complète ; écriture réelle refusée (`NotProven`)** |

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
| `akmctl rename --device-name <nom> [--dry-run]` | valide le nom, affiche le pré-vol, **sauvegarde** le nom actuel, affiche chaque octet qui serait envoyé avec son niveau de preuve, les inconnues et les expériences | **aucun** |
| `akmctl rename --device-name <nom> --write-device-name` | séquence gardée (§4) ; aujourd'hui : **refus `NotProven`** avant tout pré-vol, toute sauvegarde, toute confirmation | aucun aujourd'hui |
| `akmctl rename --device-name --restore <sauvegarde.json> [--write-device-name]` | retour arrière : réécrit exactement les 32 octets sauvegardés (même protocole, nouvelle confirmation) ; refusé aussi tant que `NotProven` | aucun aujourd'hui |
| D-Bus : propriété **lecture seule** `DeviceNameOnKeyboard` (s, `""` = pas encore lu) | nom propre du cache | aucun |
| `akmctl status` : ligne `On kb:` ; `--json` : `name_on_keyboard` | idem | aucun |

* **Aucune méthode D-Bus n'écrit le nom propre**, aucun bouton de la fenêtre, du tray ni du widget : la seule voie est la commande interactive. (La spécification admettait une méthode D-Bus protégée par l'uid de l'appelant et un jeton à usage unique ; ne pas l'exposer du tout est plus strict et supprime ce risque.)
* Le démon lit `0x51-0x54` **une fois par connexion**, après `0x4F` et `0x60`, en priorité basse : ce qui ne tient pas dans le budget de 4 s attend la rafale suivante sans marquer la lecture incomplète (`registry::DAEMON_DEFERRED_ONCE_IDS`). C'est la lecture qu'Apple fait aussi (`deviceNameFromHardware`, [décompilé]). Le cache est remis à zéro à chaque connexion : c'est ce qui permet la relecture après l'écriture.
* `--device-name` et l'alias s'excluent : `akmctl rename Bureau --device-name x` est une erreur d'usage (64).

## 3. Validation du nom propre (`akm-core::devname::validate`)

ASCII imprimable (`0x20`-`0x7E`), **1 à 32 caractères** (ce que `0x51-0x54` peuvent relire : 4 × 8 ; l'interface Apple autorise 64), **rien n'est rogné** (ce qui est validé est exactement ce qui serait écrit), pas d'espace en tête ni en fin, aucun caractère de contrôle ni non ASCII, refus de `\` (caractère d'échappement du fichier `info` de BlueZ, où `\s`, `\n`… seraient relus autrement). Tout autre ASCII imprimable est accepté par BlueZ (UTF-8 ≤ 248 o) et par SDP ; `#` reste permis (le nom actuel en contient un).

## 4. Séquence gardée (`akm-core::devname::run`)

Ordre strict, arrêt au premier échec, aucune répétition automatique, chaque décision et chaque octet journalisés (`[devname] …` sur stderr, puis `[hid-write] …` à la porte matérielle) :

1. nom validé, trames construites ;
2. **preuve** : `SEQUENCE_PROOF` doit valoir `Proven` ; aujourd'hui `NotProven` → refus, **rien n'est touché** (pas même un pré-vol) ;
3. **pré-vol** : clavier connecté ; batterie ≥ 20 % (ou état `0x30` « normal », non publié par le démon : le pourcentage décide) ; disjoncteur fermé (`keyboard.breaker_open`) ; aucune lecture récente en échec (`keyboard.incomplete`, `kb_error`) ; `akmctl doctor` vert (connecté, santé `connected`, verdict ok/info) ; stdin **et** stdout sont un terminal ;
4. cache `0x51-0x54` complet de la connexion courante (sinon arrêt) ;
5. **sauvegarde** avant toute écriture : `~/.local/state/apple-kb-monitor/devname-backup-<AAAAMMJJTHHMMSSZ>.json`, fichier neuf (`create_new`, jamais écrasé), **0600**, dossier 0700, `fsync` ; contenu : MAC, 4 fragments hex, nom, horodatage, source `daemon-cache` (octets lus une fois par le démon, espacement 1 s) ;
6. **confirmation** : `--write-device-name` passé **et** le nom retapé exactement dans le terminal ; tout autre texte annule ; jamais en non interactif ;
7. connexion revérifiée juste avant l'écriture (déconnecté → arrêt, rien écrit) ; ouverture du nœud hidraw sous le verrou HID partagé avec le démon (`hidraw::WriteDoor`), disjoncteur, espacement de 1 s après le dernier accès ;
8. **une** écriture par trame via `WriteSession` (opération `DeviceName`, id `0x55`, exactement 64 octets de données, une seule fois par session) ;
9. attente de la reconnexion (au plus 180 s) avec la consigne « éteignez le clavier (3 s), attendez 5 s, rallumez-le » : le nom ne se relit qu'après une reconnexion ;
10. **vérification** : relecture `0x51-0x54` (cache de la nouvelle connexion) comparée octet pour octet aux 32 premiers octets écrits ; différence → **retour arrière guidé** affiché : `akmctl rename --device-name --restore <sauvegarde> --write-device-name` (nouvelle commande, donc nouvelle session, nouvelle confirmation).

Barrières indépendantes contre une écriture de `0x55` aujourd'hui : (1) `NotProven` ; (2) la porte matérielle `hid_write_feature` n'a qu'un ioctl d'**un** octet et refuse toute opération qui porte des données ; (3) registre : `0x55` n'est écrivable que par l'opération `DeviceName`, 64 octets exactement, une fois.

## 5. Ce qu'Apple envoie, et ce qui n'est pas prouvé

### 5.1 Sources

* **Lion 10.7** `-[AppleBluetoothHIDDevice setDeviceName:]` (RE-PILOTES-ANCIENS.md §5, ligne L11) [désassemblage] : si la personnalité déclare `LongDeviceName` (cas du PID 598 = `0x0256`) : **un SET Feature `0x55` de 64 octets** ; sinon `0x51`…`0x54` (8 o chacun) puis `0x50` `DeviceNameChange` ; ensuite une requête de nom distant HCI. `getMaxDeviceNameLength` = 64 si `0x55` est déclaré, 32 sinon.
* Personnalité du 598 [plist] : `0x50` (sans taille), `0x51-0x54` (8 o), `0x55` (64 o). Mesures : `0x51-0x54` se lisent, `0x50`/`0x55` refusent le GET (`0x03`) [mesuré].
* **macOS 26.5 n'écrit jamais le nom** : `setDeviceName:` ne change que le cache hôte (RE-MACOS-SILICON.md §3.4) ; le noyau n'a aucun appelant de `setExtendedReport` hors `WillShutdown` (RE-GHIDRA-KEXT.md §4) ; IOBluetooth ne fait que **lire** `0x51-0x54` (`deviceNameFromHardware`, tampon 10, délai 1000 ms, RE-GHIDRA-IOBLUETOOTH.md §2).
* Délai : 1000 ms est le **délai d'attente** passé à `IOHIDDeviceInterface::setReport` [désassemblage], pas un espacement entre trames ; dans le noyau, 1000 ms est la marge de la minuterie de garde (RE-GHIDRA-KEXT.md §2.3). Sur le fil HIDP, le SET Feature est préfixé `0x53` par la pile.

### 5.2 Trame de référence générée (hypothèse U1) pour le nom mesuré

Octets remis au noyau (65) puis sur le fil (66), pour `Clavier de maria #1` ; test `devname::tests::frame_matches_the_documented_reference_byte_for_byte` :

```
report : 55 43 6c 61 76 69 65 72 20 64 65 20 6d 61 72 69 61 20 23 31 00 × 45
wire   : 53 55 43 6c 61 76 69 65 72 20 64 65 20 6d 61 72 69 61 20 23 31 00 × 45
```

| Octets | Contenu | Preuve |
|---|---|---|
| fil 0 | `53` SET_REPORT Feature, ajouté par la pile | [désassemblage] `setReportWL` |
| 0 | `55` `LongDeviceName` | [plist] + [désassemblage] L11 |
| 1-64 | 64 octets de données | [plist] `size` = 64 ; [désassemblage] `getMaxDeviceNameLength` |
| 1-n | le nom en ASCII | **[hypothèse]** (relecture ASCII [mesuré]) |
| n+1-64 | bourrage NUL | **[hypothèse]** (`0x53`/`0x54` relus bourrés de NUL [mesuré]) |
| après | aucune autre trame (pas de `0x50`, pas de `0x51-0x54`) | [désassemblage] L11 |

Ce n'est **pas** une trame Apple observée : c'est la construction de l'hypothèse U1. Elle deviendra la référence quand E1 l'aura confirmée.

### 5.3 Inconnues (pourquoi `NotProven`)

| # | Inconnue |
|---|---|
| U1 | contenu des 64 octets de `0x55` : codage (ASCII, UTF-8, MacRoman), terminateur NUL, bourrage (NUL ? espace ?), préfixe de longueur. L11 donne l'id et la taille, **ni listing ni adresse** |
| U2 | RE-MACOS-SILICON §3.4 qualifie l'ordre d'écriture de **déduction** (« non observé chez Apple ») alors que RE-PILOTES-ANCIENS L11 cite un désassemblage de Lion : le listing de `setDeviceName:` n'est pas dans le dépôt |
| U3 | persistance : NVRAM ou volatile (le descripteur du Magic Keyboard déclare son Feature `0x55` *volatile*, RE-COMMANDES-VENDEUR §1.1) ; perdu au changement de piles ? |
| U4 | `0x51-0x54` reflètent-ils une écriture de `0x55` (32 premiers octets ?), et quand (aussitôt, après un reset, après une reconnexion) ? |
| U5 | le micrologiciel `0x0050` répond-il HANDSHAKE SUCCESSFUL à un SET `0x55` (le refus du GET ne dit rien du SET) ? |
| U6 | chemin Linux : ioctl hidraw de 65 octets → `hidp` → 66 octets sur le canal de contrôle : MTU L2CAP négocié avec ce clavier inconnu (Apple fragmente en DATC, RE-GHIDRA-KEXT §2.5) |
| U7 | rôle de `0x50` `DeviceNameChange` : seulement sur la voie à 4 fragments d'après L11 (jamais envoyé ici) ; jamais écrit par aucun macOS examiné |

### 5.4 Expériences de validation passives (aucune écriture)

| # | Lève | Expérience |
|---|---|---|
| E1 | U1, U2, U7 | Ghidra sur IOBluetooth.framework de Lion 10.7.5 : adresse de `-[AppleBluetoothHIDDevice setDeviceName:]`, conversion de chaîne (`getCString:maxLength:encoding:` et sa constante d'encodage), taille passée à `setReport`, `memset`/bourrage, délai, `0x50` ou non ; commiter la trame de référence de `Clavier de maria #1` ici (§5.2) |
| E2 | U6 | capture `btmon` d'une reconnexion ordinaire : MTU des Configure Request/Response L2CAP du PSM 17 (contrôle) |
| E3 | U3 | comparer `0x51-0x54` (cache), `HID_NAME` et le `Name` BlueZ avant et après un changement de piles |
| E4 | U3, U5 | descripteurs HID publics de claviers Apple de la même génération déclarant `0x55` : drapeaux Feature (volatile ou non) et taille |

**Pour lever `NotProven`** : E1 fournit la trame de référence ; on remplace §5.2 par la trame Apple, on fait correspondre `devname::frames_for` octet pour octet (test de référence), on ajoute à la porte matérielle un ioctl de 65 octets réservé à `DeviceName`, puis on passe `SEQUENCE_PROOF` à `Proven` dans un commit relu, avec accord écrit du gérant. Un test refuse toute autre construction de `SequenceProof::Proven` dans le code de production.

## 6. Risques de l'écriture réelle

1. **Brique / état micrologiciel** : écrire un rapport vendeur sur un micrologiciel ancien sans mode de récupération connu. Une erreur de numéro toucherait un voisin (`0x5A/0x60/0xEB` étalonnage, `0xD0-0xFB`) : le registre ne le permet pas (opération `DeviceName` = `0x55` seul).
2. **Perte de pairage** : si le clavier redémarre ou si le nom entre dans les données de lien, le pairage peut sauter ; un clavier Bluetooth sans pairage ne se ré-appaire pas sans autre clavier.
3. **Nom illisible ou tronqué** si U1 est faux (bourrage, codage) : retour arrière par la sauvegarde, sous réserve que l'écriture fonctionne.
4. **Nom perdu** au changement de piles si `0x55` est volatile (U3).
5. **Gain faible** : l'alias (a) donne déjà le même affichage sur ce poste.

## 7. Procédure pour le gérant

Aujourd'hui (aucune écriture possible, rien n'est risqué) :

1. `akmctl rename --device-name --show` : nom propre actuel (cache du démon).
2. `akmctl rename --device-name "<nom voulu>"` : vérifie le nom, affiche le pré-vol, **sauvegarde** le nom actuel (chemin affiché), montre les 66 octets qui seraient envoyés et leur niveau de preuve.
3. Décider s'il faut lancer E1-E4 (§5.4). Tant que E1 n'est pas fait, `--write-device-name` répond `REFUSED (NotProven)`.

Le jour où la séquence est prouvée (`Proven`), avec accord écrit et nom choisi :

1. Avoir un second clavier fonctionnel, piles neuves (≥ 20 %).
2. `akmctl doctor` doit être vert ; `akmctl rename --device-name --show` doit afficher le nom actuel.
3. `akmctl rename --device-name "<nom>" --write-device-name` dans un terminal ; relire le résumé ; retaper le nom exactement.
4. Quand la commande le demande : éteindre le clavier (3 s), attendre 5 s, le rallumer ; attendre la vérification (≤ 3 min).
5. Résultat `✓` : terminé (la sauvegarde reste dans `~/.local/state/apple-kb-monitor/`). Résultat `✗ différent` : lancer la commande de retour arrière affichée.

## 8. Retour arrière

`akmctl rename --device-name --restore ~/.local/state/apple-kb-monitor/devname-backup-<horodatage>.json --write-device-name` : vérifie la sauvegarde (4 × 8 octets, ASCII puis NUL seulement, cohérente avec le nom), affiche les octets, puis suit **le même protocole** (pré-vol, confirmation en retapant le nom sauvegardé, une écriture, reconnexion, relecture). Sans `--write-device-name` : affichage seul. Aucune nouvelle sauvegarde n'est faite pendant un retour arrière ; aucun retour arrière n'est automatique.
