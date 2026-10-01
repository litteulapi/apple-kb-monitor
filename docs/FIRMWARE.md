# Contrôle de version du firmware (#227)

Le démon lit le rapport Feature `0x4F` (u16 petit-boutiste ; `4f 50 00` = `0x0050`, égal au `bcdDevice` du
modalias) **une seule fois par connexion**, puis compare la version à une **table embarquée** de la dernière
version publique connue par identifiant produit (PID). Aucune requête réseau, aucun flash : le résultat est une
information.

## Ce que fait le programme

| Étape | Détail |
|---|---|
| Lecture | `0x4F` (classe `OncePerConnection` du registre, `akm-core/src/registry.rs`) après les trois lectures de routine, avec l'espacement de 1 s, le disjoncteur à 3 échecs et le verrou unique de `read_policy`. Le drapeau « déjà demandé » est remis à zéro à chaque `note_connection()`. Un échec ne relance pas la lecture dans la même connexion : le statut reste `unknown`. |
| Table | `akm-core/src/firmware.rs`, `KNOWN_FIRMWARE`, date de revue `TABLE_DATE`. |
| Statut | `up_to_date` : version = dernière connue ; `update_available` : version ∈ {`0x0044`, `0x0046`} (anciennes versions dont l'updater Apple de 2009 est documenté) ; `unknown` : modèle absent de la table, version non lue, version plus récente que la table ou autre version plus ancienne (la table ne sait pas conclure et le dit). |
| Aucun flash | Aucune fonction d'écriture n'existe (`registry::check_write` refuse tout). « Mise à jour disponible » signifie « Apple a publié une version plus récente pour ce modèle ». |

## Où c'est exposé

* D-Bus (racine et objet de chaque clavier) : `FirmwareVersion` (`0x0050`, vide = pas encore lue), `FirmwareLatestKnown`
  (vide = modèle hors table), `FirmwareStatus` (`up_to_date` / `update_available` / `unknown`, jamais vide).
* JSON (`akmctl status --json`, propriété `Json`) : `firmware.version_hex`, `firmware.latest_known`, `firmware.status`,
  `firmware.source`, `firmware.table_date`.
* `akmctl status` (ligne `Firmware:`), `akmctl firmware` (version, dernière connue, statut, source, date de la table),
  `akmctl firmware --json`.
* Fenêtre : « Firmware : 0x0050 — à jour (dernière version publique connue d'Apple) » (en anglais si la locale n'est pas
  française), source et date de la table dessous. Infobulle du tray : une ligne. Widget Plasma : ligne dans la vue complète
  et dans l'infobulle.

## D'où vient chaque entrée de la table

| PID | Dernière version | Source |
|---|---|---|
| `0x0255`, `0x0256`, `0x0257` (A1314 ISO/ANSI/JIS) | `0x0050` | **[mesuré]** `0x4F` = `4f 50 00` sur un A1314 ISO `05AC:0256` le 2026-10-01 (docs/HARDWARE-RAPPORTS-HID.md §2). **Aucun programme de mise à jour public connu pour ces PID** (docs/RE-PILOTES-ANCIENS.md §6, #218) : « dernière connue » = la version observée sur le matériel, pas une version qu'Apple distribuerait. |
| `0x0239`, `0x023A`, `0x023B` (A1255, 2007-2009) | `0x0050` | **[source publique]** « 2009 Aluminum Keyboard Firmware Update » (support.apple.com/en-us/106755) : `Parameters.plist` `FWVersion = 80` (= `0x50`), cible 0x239-0x23B (docs/RE-COMMANDES-VENDEUR.md §4.3, RE-PILOTES-ANCIENS.md §6). Versions antérieures : `0x44` / `0x46`. |

Les autres modèles (A1255 `0x022C`-`0x022E`, Magic Keyboard…) sont `unknown` : la table ne les connaît pas, et le démon ne
lit d'ailleurs aucun rapport vendeur sur les familles hors BCM2042.

## Mettre la table à jour

1. Obtenir la preuve : une version lue sur un clavier réel (`akmctl info` après connexion) ou la page de support Apple /
   `Parameters.plist` d'un updater. Une version vue une seule fois sur un exemplaire n'est pas « la dernière publique ».
2. Ajouter ou modifier l'entrée dans `KNOWN_FIRMWARE` (`akm-core/src/firmware.rs`) avec une `source` précise ; ajouter une
   version antérieure documentée à `OLDER_WITH_UPDATE` seulement si l'updater d'Apple est établi pour ce PID.
3. Mettre à jour `TABLE_DATE` (date ISO) et le tableau ci-dessus.
4. `cargo test -p akm-core firmware::` : `pids_are_unique_across_the_table`, `current_firmware_of_every_known_pid_is_up_to_date`.
   Ajouter le cas dans les tests si une nouvelle règle de statut apparaît.

## Registre des rapports (résumé)

`akmctl info` liste la table complète. Classes de sécurité : `SafeRead` (`0x47`, `0x46`, `0x49`), `OncePerConnection`
(`0x4F`, `0x60`, `0x51`-`0x54` ; le démon ne lit que `0x4F` et `0x60`), `PassiveInput` (`0x04` `0x05` `0x30` `0x13` `0x11`
`0x12`), `ManualOnly` (lisibles mais jamais demandés par le démon : `0xFF`, `0x5A`, `0xEB`, `0x5B`, `0xF4`, `0xF5`, `0xEA`,
`0x09`, `0x5C`, `0x5D`), `NeverRead` (`0xFE`, `0x4C`, Input `0x01`, `0x34`, `0x35`), `NeverWrite` (`0x40` `0x41` `0x43`
`0x44` `0x45` `0x4A` `0x50` `0x55`, `0xD0` `0xD4` `0xD5` `0xFA` `0xFB`, et tout `0xDx`/`0xFx` inconnu), `Unknown`. La liste blanche
de lecture est **générée** de la table ; `hid_read_feature` (seul appel `ioctl` vers le clavier) refuse tout id dont la classe
ne permet pas la lecture, avant l'ioctl. Aucune écriture HID n'existe : `0x44` (oubli de tous les hôtes, #217) et `0x4A`
(notification SCO, #216) restent des écritures soumises à l'accord explicite du gérant et ne sont pas implémentées.

Seuils `0x60` (Full / Low / Critical / Empty, mV, 4 × u16 BE, lus une fois par connexion) : exposés en JSON
(`battery.thresholds`, `threshold_level`, `threshold_margins_mv`) et affichés avec la marge restante avant `Low` et `Critical`
(#215). Ils ne remplacent pas l'estimation par chimie (#178) : ce sont les seuils du firmware, l'estimation reste celle de la
chimie déclarée.
