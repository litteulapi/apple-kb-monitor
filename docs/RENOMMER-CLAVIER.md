# Renommer le clavier (#141, #192, #248)

Deux noms existent. L'alias ne concerne que ce poste ; le nom propre est stocké dans le clavier et vu par tous les appareils. Les deux se changent par une commande ordinaire.

| | (a) Alias côté poste | (b) Nom propre, stocké dans le clavier |
|---|---|---|
| Commande | `akmctl rename <nom>` / `--reset` | `akmctl rename --device-name <nom>` (écrit, après **une** confirmation), `--check`, `--dry-run`, `--show`, `--restore` |
| Où | BlueZ, `org.bluez.Device1.Alias`, persisté dans `/var/lib/bluetooth/<adaptateur>/<MAC>/info` (`Alias=`) | micrologiciel du clavier (BCM2042) : lu dans `0x51-0x54` (4 × 8 o ASCII), écrit dans `0x55` `LongDeviceName` (65 o : id + 64) |
| Visible par | ce poste uniquement (KDE Bluetooth, `bluetoothctl`, tray, widget, `akmctl`) | tout appareil qui s'appaire au clavier |
| État | **implémenté** | **éprouvé sur le matériel** le 02/10/2026 (micrologiciel `0x0050`) : écriture acceptée, relue identique dans la même connexion |

## 1. (a) Alias : ce qui est livré

* `akmctl rename <nom>` / `akmctl rename --reset` (`--mac` pour cibler un clavier). Codes retour : 0 OK, 1 erreur (nom refusé, BlueZ), 2 démon absent.
* D-Bus session : `com.agenceapi.AppleKbMonitor1.SetAlias(s mac, s nom) -> s` sur l'objet racine, `Device.SetAlias(s nom) -> s` sur l'objet clavier ; propriété `Name` (alias, sinon nom propre) sur les deux. Nom vide = retour au nom d'origine (comportement BlueZ).
* Validation (`akm-core::alias`, partagée par tous les clients) : espaces de bord retirés, max 64 caractères et 248 octets UTF-8 (limite HCI), refus des caractères de contrôle (Cc), séparateurs de ligne/paragraphe, caractères invisibles et surcharges bidirectionnelles (U+200B-200F, 2028-202E, 2060-2064, 2066-2069, FEFF). Le démon n'écrit que sur un appareil BlueZ dont le `Modalias` est Apple.
* Tray : « Renommer le clavier… » (boîte `kdialog`, sinon `zenity`, sinon ouverture de la fenêtre). Fenêtre egui : champ + Renommer / Réinitialiser. Widget Plasma : champ + Renommer / Réinitialiser. **Ces boutons ne touchent qu'à l'alias.**
* Affichage : infobulle du tray, en-tête du menu, `akmctl status [--json]` (`name`, `alias`), `apple-kb-monitord --json` (`name`, `keyboard.device.alias`), JSON du snapshot, widget.
* Un renommage fait ailleurs (`bluetoothctl`, Paramètres KDE) est repris via `PropertiesChanged.Alias`.
* **Traçabilité (2026-10-02, alias réinitialisé deux fois sans cause trouvée).** Inventaire de tout ce qui écrit l'alias dans l'arbre : (1) `apple_kb_monitord::alias::rename`, seul écrivain, appelé par `SetAlias` (racine et `Device`, donc `akmctl rename`, le KCM `NamePage`/`Store.qml`, le widget `DaemonLink.qml`), par le tray « Renommer le clavier… » et par la fenêtre quand le démon est absent ; (2) **indirectement**, `Adapter1.RemoveDevice` (`akmctl repair`, l'oubli `forget`, « Oublier » de Plasma, `bluetoothctl remove`) : BlueZ efface le répertoire de l'appareil, alias compris, et un nouvel appairage reprend le nom propre ; l'alias est copié dans la sauvegarde `forget-*.json` mais **jamais restauré**. Aucun test, ni `selftest`, ni `doctor` n'écrit l'alias : les tests passent par des moteurs factices (`FakeAlias`, `Spy`) ou un bus privé (`dbus-run-session`, `tests/e2e`). Désormais chaque écriture est journalisée avec son appelant (nom D-Bus, pid, programme lu dans `/proc/<pid>/comm`, ou « tray »), l'alias obtenu est mémorisé dans `~/.local/state/apple-kb-monitor/alias.json` (moteur BlueZ réel seulement), le démon journalise en avertissement tout changement venu d'ailleurs (`BlueZ alias of … changed OUTSIDE this monitor`, avec l'alias attendu, qui l'a posé et quand) et `akmctl selftest` signale en **info** un alias différent de celui mémorisé.
* Limite : `HID_NAME` du noyau (`/sys/.../uevent`) garde l'ancien nom jusqu'à la prochaine reconnexion ; l'application affiche l'alias en priorité.

## 2. (b) Nom propre : la commande

```
akmctl rename --device-name "Bureau"
```

Ce que l'on voit (français si la locale commence par `fr`, anglais sinon) :

```
Nom stocké dans le clavier : « Clavier de alice #1 » → « Bureau »
Pré-vol : ok (doctor vert, batterie 99 %, disjoncteur fermé, MTU sortante 185 >= 66)
Sauvegarde du nom actuel : ~/.local/state/apple-kb-monitor/devname-backup-<horodatage>.json
Écrire « Bureau » dans la mémoire du clavier ? [o/N] o
✓ Nom écrit dans le clavier et relu identique : « Bureau »
  BlueZ peut afficher l'ancien nom jusqu'à une prochaine connexion ; la persistance après un changement de piles n'est pas mesurée.
```

| Commande | Effet | Écrit ? |
|---|---|---|
| `akmctl rename --device-name <nom>` | pré-vol, sauvegarde, **une** confirmation `[o/N]` (`[y/N]` en anglais), une écriture, relecture immédiate, verdict | oui |
| `… --yes` (`-y`) | la même chose sans la question ; fonctionne sans terminal (scripts, module KDE) | oui |
| `… --check` | tout le pré-vol et la sauvegarde ; ne demande rien | non |
| `… --dry-run` | les 65 octets qui seraient envoyés, champ par champ ; aucun `pkexec` | non |
| `… --verbose` | ajoute le détail : trame, octets sur le fil, preuve par désassemblage, journal `[devname]` | selon la commande |
| `akmctl rename --device-name --show` | nom lu dans `0x51-0x54` et ses octets, depuis le cache du démon | non |
| `akmctl rename --device-name --restore <sauvegarde.json>` | réécrit une sauvegarde, même déroulé (une confirmation ou `--yes`) | oui |

Sans terminal et sans `--yes`, la commande refuse **avant** tout pré-vol, tout `pkexec` et toute sauvegarde (code 10) et dit d'ajouter `--yes`. `--write-device-name` (ancienne option) reste accepté et ne fait rien. La clé `[apple] allow_device_name_write` de `config.toml` est obsolète : lue, ignorée.

Codes de sortie : **0** écrit et relu identique (ou `--check` vert) · 1 erreur · 2 démon absent · **10** pas de confirmation possible · **11** pré-vol refusé · **12** annulé (réponse autre que oui ; rien d'écrit) · **13** écrit mais relecture impossible (vérifier plus tard avec `--show`) · **14** relu différent (la commande de retour arrière est affichée) · 64 usage.

Aucun mot de passe n'est demandé dans la session locale active : la lecture de la MTU passe par l'action polkit `com.agenceapi.AppleKbMonitor.hid-inspect` (`pkexec akm-hid-control inspect`, lecture seule, `allow_active = yes`).

### Module des Paramètres système

Onglet « Nom » : le bouton « **Écrire le nom dans le clavier…** » ouvre une boîte de confirmation (« Écrire « X » dans la mémoire du clavier ? »), puis lance `akmctl rename --device-name=<nom> --yes` ; « **Vérifier (sans écrire)** » lance la même commande avec `--check`. Le résultat s'affiche dans la page. Pas de terminal, pas de shell : `QProcess` avec une liste d'arguments, le nom en **un seul argument**, délai borné à 90 s (`KCM.md`).

## 3. Faits mesurés (02/10/2026, clavier réel, micrologiciel `0x0050`)

| # | Question | Réponse |
|---|---|---|
| U5 | le micrologiciel accepte-t-il un SET Feature `0x55` ? | **[mesuré] oui** : l'écriture de 65 octets réussit (HANDSHAKE SUCCESSFUL) ; la liaison n'a pas bougé (disjoncteur à 0) |
| U4 | `0x51-0x54` reflètent-ils `0x55`, et quand ? | **[mesuré] oui, dans la même connexion**, sans éteindre ni rallumer le clavier : relus ~75 s après l'écriture, le nom écrit puis `0x00` jusqu'à 32 octets |
| U6 | MTU sortante du canal de contrôle | **[mesuré] 185** (≥ 66 requis) ; relue à chaque commande, jamais supposée |
| — | `Device1.Name` de BlueZ | **[mesuré]** garde l'ancien nom en cache ; il se rafraîchit à une connexion ultérieure |
| U3 | persistance après extinction ou changement de piles | **non mesurée** : Apple ne réécrit jamais le nom à la reconnexion (le clavier est donc censé le mémoriser, [déduction]) ; la sauvegarde permet de le réécrire |

La trame est celle de Lion 10.7.5 `-[AppleBluetoothHIDDevice setDeviceName:]`, établie octet pour octet par désassemblage (U1, U2, U7 : `RE-NOM-PROPRE-E1.md`) : **un** SET Feature `0x55` de 65 octets, `55` + le nom en UTF-8 + bourrage `0x00`, soit `53 55 …` (66 octets) sur le fil ; aucune autre trame.

```
« alex »
report : 55 61 6c 65 78 00 × 60
wire   : 53 55 61 6c 65 78 00 × 60
```

Référence : `tests/fixtures/devname/lion_setdevicename_frames.json` ; `devname::tests::frames_match_the_lion_fixture_byte_for_byte` et `confirmed_the_fixture_frame_is_sent_once_after_the_backup_then_read_back` vérifient que l'espion reçoit exactement ces octets, une fois, après la sauvegarde.

## 4. Déroulé d'une écriture (`akm-core::devname::run`)

Arrêt au premier échec, **jamais de réessai**.

1. **Nom validé** (`devname::validate`) : ASCII imprimable (`0x20`-`0x7E`), 1 à 32 caractères, rien n'est rogné, pas d'espace en tête ni en fin, pas de `\` (échappement du fichier `info` de BlueZ). 32 = ce que `0x51-0x54` peuvent relire.
2. **Pré-vol** : clavier connecté ; piles ≥ 20 % (ou état « normal ») ; disjoncteur fermé ; dernière lecture complète ; `akmctl doctor` vert ; MTU sortante du canal de contrôle L2CAP ≥ 66, lue par l'unique `pkexec akm-hid-control inspect --mac <MAC>` (`getsockopt`, lecture seule). Linux `hidp` ne fragmente pas : une trame trop longue couperait la session HID.
3. **Sauvegarde** du nom actuel, avant toute écriture : `~/.local/state/apple-kb-monitor/devname-backup-<AAAAMMJJTHHMMSSZ>.json`, fichier neuf, **0600** (dossier 0700).
4. **Confirmation unique** `[o/N]` (ou `--yes`).
5. Connexion revérifiée ; ouverture du nœud hidraw sous le verrou HID partagé avec le démon (`hidraw::WriteDoor`).
6. **Une** écriture par `WriteSession` et le registre : opération nommée `DeviceName`, id `0x55`, exactement 64 octets de données, une fois par session ; porte matérielle de 65 octets.
7. Attente de l'espacement (1 s), puis **relecture immédiate** de `0x51-0x54` par la même porte (`WriteDoor::read_name`), à travers `read_policy::SafeSource` : registre, 1 s entre deux demandes, disjoncteur, arrêt au premier échec ; chaque trame doit faire l'id + 8 octets.
8. **Comparaison** aux 32 premiers octets écrits, verdict (codes 0, 13, 14).
9. Le démon est prévenu (D-Bus `RereadName`) : il oublie ses fragments `0x51-0x54` et les relit (au plus une fois toutes les 30 s).

Protections inchangées : registre (`0x55` écrivable par `DeviceName` seulement), `WriteSession`, politique de lecture et budgets, disjoncteur (celui du démon décide), MTU ≥ 66, sauvegarde avant écriture, jamais de réessai. Aucune méthode D-Bus n'écrit dans le clavier.

## 5. Retour arrière

```
akmctl rename --device-name --restore ~/.local/state/apple-kb-monitor/devname-backup-<horodatage>.json --yes
```

La commande exacte est affichée quand le nom relu diffère (code 14). Elle vérifie la sauvegarde (4 × 8 octets, ASCII puis NUL, cohérente avec le nom) et suit le même déroulé, sans nouvelle sauvegarde. Sans `--yes`, elle pose la même question `[o/N]`. Aucun retour arrière n'est automatique.

## 6. Ce qui reste à savoir

* **U3, persistance** : non mesurée. À faire (E3) : comparer `akmctl rename --device-name --show` avant et après un changement de piles. Si le nom est perdu, la commande du §5 le réécrit.
* Le nom affiché par BlueZ (`Device1.Name`, donc KDE et `bluetoothctl` quand aucun alias n'est posé) suit à une connexion ultérieure. `HID_NAME` du noyau aussi.
