# Rapports HID Feature du clavier Apple A1314 (BCM2042) — carte de rétro-ingénierie

Appareil étudié : Apple Wireless Keyboard A1314 ISO « Clavier de alice #1 », `AA:BB:CC:DD:EE:F1`,
`0005:05AC:0256`, bcdDevice `0x0050`, pilote noyau `apple`, `/dev/hidraw7`.
Piles neuves posées le 2026-10-01 vers 03:00 (clavier hors ligne de 02:55 à 03:06, capacité noyau 90 % → 100 %).

Méthode : **lecture seule** uniquement (`HIDIOCGFEATURE` = GET_REPORT, nœud ouvert en `O_RDONLY`),
aucun SET_REPORT, pas de sudo, au plus une salve de lectures par 5 minutes.
Outils : `tests/live/re/sample_reports.py`, `tests/live/re/idle_timeout.py`.
Données : `tests/fixtures/a1314_iso/reports_scan_20261001.json` (balayage 0x00-0xFF),
`tests/fixtures/a1314_iso/reports_timeseries_20261001.json` (série temporelle).

Niveaux de preuve :

* **[mesuré]** : observé sur le matériel pendant cette étude ;
* **[source]** : document ou code public cité ;
* **[hypothèse]** : interprétation plausible, non prouvée.

> ⚠️ Le rapport `0x4C` contient un secret (19 octets, forte entropie). Il n'est jamais recopié en clair :
> les fichiers ne contiennent que sa longueur, son premier octet et une empreinte SHA-256 tronquée.

## 1. Ce que déclare le descripteur HID

Descripteur de 224 octets (`/sys/class/hidraw/hidraw7/device/report_descriptor`, identique à
`tests/fixtures/a1314_iso/report_descriptor.bin`) décodé **[mesuré]** :

| Report ID | Type | Contenu déclaré |
|---|---|---|
| `0x01` | Input/Output | clavier de démarrage : 8 modificateurs, 1 octet réservé, 6 touches ; 5 LED en sortie |
| `0x47` | **Input** | page `0x06` *Generic Device Controls* / usage `0x20` Battery Strength, 8 bits, 0-255 (dans une collection Consumer `0x0C:0x01` → Generic Desktop Keyboard) |
| `0x11` | Input | 3 bits de bourrage, Consumer Eject `0xB8`, vendeur `0x00FF:0x03` (= touche Fn), 3 bits de bourrage |
| `0x12` | Input | Consumer Play/Pause `0xCD`, Fast Forward `0xB3`, Rewind `0xB4`, Scan Next `0xB5`, Scan Previous `0xB6` + 3 bits de bourrage (pas de Stop `0xB7`) |
| `0x13` | Input | vendeur `0xFF01:0x0A` (1 bit, `Input 0x02`), `0xFF01:0x0C` (1 bit, `Input 0x22` = Data,Var,**Abs**,No Preferred — pas relatif), 6 bits de bourrage |
| `0x09` | **Feature** | vendeur `0xFF01:0x0B`, 1 octet de données + 2 octets constants (bourrage) |

**Seul `0x09` est déclaré en Feature.** `0x47` est déclaré en Input, mais le noyau le lit en Feature :
`hid-input.c` classe les claviers Apple aluminium sans fil dans `hid_battery_quirks` avec
`HID_BATTERY_QUIRK_PERCENT | HID_BATTERY_QUIRK_FEATURE` (« ask for feature report ») **[source]**
([hid-input.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-input.c)).
Tous les autres IDs répondent au GET_REPORT sans être déclarés : ce sont des registres internes
du firmware Broadcom, exposés par le canal de contrôle HIDP **[mesuré]**.

`hid-apple.c` n'envoie aucun rapport Feature au 0x0256 : ses lectures/écritures (`0xBF`, `0xB0`, minuterie
batterie `APPLE_RDESC_BATTERY`) visent le rétroéclairage et les Magic Keyboard USB ; pour
`USB_DEVICE_ID_APPLE_ALU_WIRELESS_2011_ISO` il ne pose que `APPLE_NUMLOCK_EMULATION | APPLE_HAS_FN |
APPLE_ISO_TILDE_QUIRK` **[source]** ([hid-apple.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-apple.c)).
Aucun projet public trouvé ne documente les IDs vendeur du BCM2042 (recherches WebSearch 2026-10-01) ;
le reste de ce document repose donc sur la mesure.

## 2. Balayage complet 0x00-0xFF

27 IDs répondent (les autres renvoient une erreur). Longueur = octets rendus par l'ioctl, ID compris.
Les IDs `0x54`, `0x5C`, `0x5D`, `0xD1`, `0xD8` n'étaient pas dans l'inventaire initial.

| ID | Long. | Brut (2026-10-01) | Décodage | Preuve |
|---|---|---|---|---|
| `0x09` | 4 | `09 01 00 00` | octet 1 = `0x01` (vendeur `FF01:0B`), octets 2-3 = bourrage | descripteur [mesuré] ; sens [hypothèse] |
| `0x46` | 3 | `46 aa 0b` / `46 af 0b` | **u16 LE = tension piles en mV** : 2986 / 2991 mV | [mesuré], voir §3 |
| `0x47` | 2 | `47 63` | u8 = 99 % (Battery Strength) | [source] descripteur + quirk noyau ; = `power_supply/capacity` [mesuré] |
| `0x49` | 3 | `49 89 0b` → `49 86 0b` | u16 LE = 2953 puis 2950 mV, varie lentement | grandeur vivante [mesuré] ; tension filtrée [hypothèse], voir §4 |
| `0x4A` | 2 | `4a 12` | u8 = 18 | inconnu |
| `0x4B` | 3 | `4b 00 08` | `00 08` | inconnu |
| `0x4C` | 20 | `4c 03` + 18 octets masqués | 1 octet `0x03` + **adresse BD_ADDR de l'hôte appairé** (6 o, LE) + 12 octets secrets | adresse [mesuré, §2bis] ; 12 o restants SENSIBLES [hypothèse] fragment de clé de lien |
| `0x4F` | 3 | `4f 50 00` | u16 LE = `0x0050` = version firmware/bcdDevice | = `Modalias usb:v05ACp0256d0050` et « HID v0.50 » du noyau [mesuré] |
| `0x51` | 9 | `51` + `"Clavier "` | nom, fragment 1/4 (8 o ASCII) | [mesuré] |
| `0x52` | 9 | `52` + `"de alice"` | nom, fragment 2/4 | [mesuré] |
| `0x53` | 9 | `53` + `" #1"` + NUL | nom, fragment 3/4 | [mesuré] |
| `0x54` | 9 | `54` + 8 × NUL | nom, fragment 4/4 (vide ici) | [mesuré] ; nom ≤ 32 octets |
| `0x5A` | 9 | `5a 0b8a 09ca 0964 0806` | 4 × u16 BE : 2954, 2506, 2404, 2054 mV | valeurs [mesuré] ; seuils de décharge [hypothèse forte] |
| `0x5B` | 9 | `5b 06cc 0384 00000000` | concaténation de `0xF4` et `0xF5` + 4 zéros | [mesuré] |
| `0x5C` | 9 | 8 × `00` | vide | [mesuré] |
| `0x5D` | 9 | 8 × `00` | vide | [mesuré] |
| `0x60` | 9 | = `0x5A` | copie de `0x5A` | [mesuré] |
| `0xD1` | 2 | `d1 00` | u8 = 0 | inconnu |
| `0xD8` | 2 | `d8 00` | u8 = 0 | inconnu |
| `0xEA` | 2 | `ea 62` (une fois `ea 00`) | u8 = 98, 0 transitoire | [hypothèse] second estimateur de % ; 0 isolé à ignorer [mesuré] |
| `0xEB` | 9 | = `0x5A` | copie de `0x5A` | [mesuré] |
| `0xF4` | 3 | `f4 06 cc` | u16 BE = 1740 | constant ; [hypothèse] voir §4 |
| `0xF5` | 3 | `f5 03 84` | u16 BE = 900 | **constant à travers un changement de piles : ce n'est pas la tension** [mesuré] |
| `0xF6` | 3 | `f6 00 04` | 4 | inconnu |
| `0xF7` | 3 | `f7 00 04` | 4 | inconnu |
| `0xFE` | 9 | 8 × `00` | vide sauf une fois | [mesuré] `fe 00 04` au `--dump` de 03:57:18, puis zéros dans les 16 lectures exactes commitées (scan, tours A/B, 13 salves). **Pas un artefact de l'outil** : la version d'alors du `--dump` (avant 2219efa) allouait un tampon neuf par ID et ne listait un rapport que si un octet utile était non nul ; le `04` vient donc de la réponse [contre-audit]. Sens inconnu ; **ne plus lire** (#175) |
| `0xFF` | 4 | `ff 0b aa 01` / `ff 0b af 01` | **u16 BE = même tension que `0x46`** + octet `0x01` | [mesuré], voir §3 |

Endianness : `0x46`, `0x49`, `0x4F` sont en petit-boutiste ; `0x5A`/`0x5B`/`0xF4`/`0xF5`/`0xFF` en gros-boutiste.
Le firmware mélange donc deux conventions ; `0x46` (LE) et `0xFF` (BE) donnent la même grandeur.

### 2bis. Structure de `0x4C` (sans divulgation)

* **[mesuré]** octets 2 à 7 = `aa:bb:cc:dd:ee:f2` lu à l'envers, c'est-à-dire l'adresse de l'adaptateur
  Bluetooth du PC (`HID_PHYS`), pas celle du clavier. Vérifié par comparaison booléenne, sans afficher
  la suite.
* **[mesuré]** 12 octets restants : 12 valeurs distinctes sur 12 (forte entropie), empreinte SHA-256
  identique sur toutes les lectures de la journée (registre stable).
* **[hypothèse]** enregistrement d'appairage : octet `0x03` (type ou numéro d'emplacement),
  hôte lié, puis matériau de clé (12 octets ne font pas une clé de lien complète de 16 octets).
* Correction : ce n'est **pas** une IRK (concept BLE ; le A1314 est en BR/EDR). L'intitulé
  « identity key » du code et de #123 est inexact, mais la consigne de masquage reste entière.
* Le CLI Python (`read_all_reports`) publie `d[2:16]` (14 octets) : l'adresse de l'hôte **et 8 des
  12 octets secrets** ; le démon Rust publie le rapport entier (#123).

## 3. Tension réelle des piles : `0x46` et `0xFF`, pas `0xF5`

* **[mesuré]** Dans une même salve de lectures, `0x46` lu en LE et `0xFF[1..3]` lu en BE donnent la même
  valeur (balayage de 03:57:29 : 2986/2986 ; tour A de 03:58:24-33 : 2991/2991). Le dump initial de l'inventaire
  (03:57:18, lectures non simultanées) montrait 2991 (`0x46`) et 2982 (`0xFF` = `0b a6`) : la grandeur bouge entre deux
  lectures, ce qu'une constante de configuration ne fait pas.
* **[mesuré]** 2,99 V pour deux piles AA alcalines neuves (≈ 1,50 V/élément) : cohérent physiquement.
* **[mesuré]** `0xF5` vaut `0x0384` (900) **avant et après le changement de piles** du 2026-10-01
  (historique `~/.config/apple-kb-monitor/history.jsonl` : « voltage » 2,9032 V = 900 × 3,3 / 1023 à 90 %
  à 02:33 avec les anciennes piles, puis à 100 % à 03:07 avec les neuves). Une tension de piles
  ne peut pas rester identique au millivolt entre un jeu usé (90 %) et un jeu neuf.
  En avril 2026 l'historique donnait 924 (2,98 V) pendant 2 jours sans la moindre variation.
* Conséquence : la « tension » calculée comme `adc_raw * 3.3 / 1023` (`0xF5`) est fausse. Le « numéro de build »
  `0x0BAA` est en fait 2986 mV. **État au contre-audit (c54c502)** : `akm-core` est corrigé (02fad76, tension = `0x46`) ;
  le CLI Python garde les anciens décodages (#200).

## 4. Seuils, courbes et pourcentages

* `0x5A` = `0x60` = `0xEB` (trois copies identiques **[mesuré]**) : 2954 / 2506 / 2404 / 2054 mV,
  strictement décroissants, cohérents avec la courbe d'une paire d'alcalines (1,48 / 1,25 / 1,20 / 1,03 V
  par élément). Le contrat du projet (`calibration.rs`) les associe à 100/75/50/25 % **[source projet,
  non vérifié]**. Valeurs propres à l'unité (le défaut du code est 2900/2450/2350/2000).
* Contrôle de cohérence **[mesuré]** : avec 2986-2991 mV (> 2954), la courbe 100/75/50/25 donnerait 100 %,
  or `0x47` = 99 et `0xEA` = 98. En revanche `0x49` = 2953 mV, interpolé sur la même courbe, donne 99,94 %
  → 99 en troncature = `0x47`. **[hypothèse]** `0x49` est la tension filtrée (ou mesurée sous charge radio)
  dont le firmware tire le pourcentage, `0x46`/`0xFF` la mesure instantanée.
* `0xEA` (98) ≠ `0x47` (99) : **[hypothèse]** second estimateur (autre filtre ou autre arrondi) ;
  l'ancienne interprétation « pourcentage avant arrondi » est incompatible avec 98 < 99.
* `0xF4` = 1740, `0xF5` = 900 (dupliqués dans `0x5B`), constants **[mesuré]**. Hypothèses concurrentes :
  (a) 1740 mV = tension de coupure (0,87 V/élément, en dessous du dernier seuil 2054) ;
  (b) 900 = délai d'inactivité en secondes (15 min) avant veille — testé au §5 ;
  (c) paramètres radio (sniff) en slots de 0,625 ms (1087,5 ms / 562,5 ms).

## 5. Observation temporelle (lecture seule)

Série principale : 13 salves de 27 GET_REPORT, une toutes les 5 min, 2026-10-01 04:15:26 → 05:15:38,
aucune absence ni erreur d'E/S (`reports_timeseries_20261001.json`). Complétée par la capture de
l'audit voisin (`tests/live/re/capture-2026-10-01.jsonl`, branche `audit/decodage-hid`, 03:57-04:00).

| Heure | `0x46` mV | `0xFF` mV | `0x49` mV | `0x47` % | `0xEA` | noyau % |
|---|---|---|---|---|---|---|
| 03:57:18 (voisin) | 2991 | 2982 | 2953 | 99 | 98 | 99 |
| 03:58:24 (tour A) | 2991 | 2991 | 2953 | 99 | 98 | 99 |
| 04:15:26 | 2986 | 2986 | 2950 | 99 | 98 | 99 |
| 04:25:27 | 2991 | 2986 | 2950 | 99 | 98 | 99 |
| 04:40:31 | 2986 | 2986 | 2950 | 99 | **0** | 99 |
| 05:05:36 | 2991 | 2991 | 2950 | 99 | 98 | 99 |
| 05:15:38 | 2991 | 2991 | 2950 | 99 | 98 | 99 |

Constats **[mesuré]** :

1. **Constants sur toute la journée** (balayage, série, capture voisine) : `0x09`, `0x4A`, `0x4B`, `0x4C`
   (empreinte identique), `0x4F`, `0x51-0x54`, `0x5A`/`0x60`/`0xEB`, `0x5B`, `0x5C`/`0x5D`, `0xD1`/`0xD8`,
   `0xF4`, `0xF5`, `0xF6`/`0xF7`, `0xFE`.
2. **`0x46` et `0xFF`** oscillent entre 2982, 2986 et 2991 mV : écarts de 4 à 5 mV (pas de quantification probable ;
   la résolution de l'ADC n'est publiée nulle part [hypothèse]), bruit de ± 1 pas. Les deux registres sont lus à ~10 s d'écart dans une salve, d'où des écarts d'un pas.
3. **`0x49`** passe de 2953 (03:57) à 2950 (dès 04:15) puis reste fixe une heure : grandeur lissée,
   sans le bruit de `0x46` → appuie « tension filtrée ».
4. **`0xEA`** vaut 98 sauf **une lecture à 0** (04:40:31, `ea00`, longueur correcte) alors que `0x47` et le
   noyau restent à 99 : valeur transitoire (recalcul en cours ?). Tout consommateur doit ignorer un 0 isolé.
   L'historique d'avril 2026 montre aussi des 0 % ponctuels (bug #78, cause différente : lecture en échec).
5. `0x4C` désigne toujours l'hôte `aa:bb:cc:dd:ee:f2`.
6. Batterie : 99 % noyau/`0x47` sur toute l'heure ; une heure de piles neuves ne suffit pas à voir
   bouger le pourcentage, seule la tension renseigne (cf. #83/#96 pour l'historique long).

Veille et déconnexion :

* La capture voisine montre la perte du lien en pleine salve à 04:00:13 (`errno 5` après ~3,5 s par
  rapport), BlueZ journalise « keyboard disconnected » à 04:00:33 puis `Host is down` : le clavier s'est
  endormi. Aucun SET_REPORT n'a été émis ; la cause (veille d'inactivité ou effet des lectures) n'est pas
  tranchée.
* De 04:15 à 05:15, avec une lecture toutes les 5 min, le lien est resté établi.
* **[hypothèse]** `0xF5` = 900 s = délai d'inactivité avant veille. Test proposé (passif, sans requête) :
  `tests/live/re/idle_timeout.py` horodate les rapports d'entrée (contenu jamais conservé) et la
  disparition du nœud ; un écart dernier appui → déconnexion ≈ 900 s sur plusieurs cycles, sans aucun
  GET_REPORT pendant ce temps, confirmerait. Non exécuté jusqu'au bout ici (lectures suspendues sur
  consigne pendant la déconnexion).

> **Contre-audit (docs/CONTRE-AUDIT.md)** : `0xEA` « second estimateur » et la relation `0x47` = tronc(interp(`0x49`))
> sont mises en défaut à 2945 mV (#179, VERIF-BATTERIE §1.2). Noms Apple de plusieurs IDs (`0x30` BatteryState,
> `0x40` WillShutdown, `0x41` RecantConnection, `0x44`/`0x45` FactoryDefault, `0x50` DeviceNameChange, `0x51-0x54`
> DeviceName1..4, `0x55` LongDeviceName) : RE-PILOTE-MACOS §3. `0x09` : drapeau probable du délai Verr. Maj interne
> (RE-PILOTE-MACOS §8, déduction).

## 6. Synthèse : décodé / décodable / inconnu

| Statut | Rapports |
|---|---|
| **Décodé** (preuve mesurée) | `0x46` tension instantanée mV (LE) · `0xFF` même tension (BE) + octet `0x01` · `0x47` % (= noyau) · `0x4F` version `0x0050` · `0x51-0x53` nom ASCII · `0x4C` octets 2-7 = hôte appairé · `0x5A`/`0x60`/`0xEB` table de 4 tensions (3 copies) · `0x5B` = `0xF4`‖`0xF5` · `0x5C`/`0x5D`/`0xFE` vides · `0xF5` n'est **pas** une tension |
| **Décodable** (hypothèse testable sans écriture) | `0x49` tension lissée (vivante, sans bruit) · `0x5A` = seuils 100/75/50/25 % (cohérent avec `0x47` via `0x49`) · `0xEA` second estimateur %, 0 transitoire · `0xF5` = 900 s de délai de veille (protocole §5) · `0xF4` = 1740 mV de coupure · `0x54` 4ᵉ fragment de nom (nom ≤ 32 o ; troncature à 24 o du code non démontrée, nom actuel de 19 o) · octet 3 de `0xFF` (drapeau, à surveiller en fin de piles) · `0x4C` octet `0x03` |
| **Inconnu** (constant, aucune corrélation possible en lecture) | `0x09` (`FF01:0B` = 1, seul Feature déclaré) · `0x4A` (18) · `0x4B` (`00 08`) · `0xD1`/`0xD8` (0) · `0xF6`/`0xF7` (4) · 12 derniers octets de `0x4C` (secrets, non étudiés) |

Les rapports constants ne peuvent être élucidés qu'en écrivant (SET_REPORT), ce qui est exclu, ou en
comparant plusieurs claviers / firmwares (autre A1314, A1255), ou en observant une fin de vie de piles
(`0xFF` octet 3, `0xEA`, `0x49` sous 2054 mV).

## 7. Fonctions utilisateur rendues possibles et suivi Gitea

| Fonction | Rapports | Suivi |
|---|---|---|
| Corriger la tension affichée (`0xF5` × 3,3/1023 → `0x46` mV) et les décodages faux (`0xFF` « build », `0x46`/`0x49` « paramètres BT », `0xEA` « pré-arrondi ») | `0x46`, `0xFF`, `0x49`, `0xEA` | bugs #131, #132, #136 (preuve du changement de piles ajoutée sur #136 ; #138 fermé en doublon) |
| Tension réelle + pourcentage fin 0,1 % sur la courbe d'usine de l'unité | `0x46`, `0x49`, `0x5A` | **#139** (F35) |
| Hôte appairé lu dans le clavier, alerte de ré-appairage ailleurs | `0x4C` octets 2-7 seulement | **#140** (F36) ; décodage #133, exposition #123 |
| Détection du remplacement de piles par saut de tension | `0x49`/`0x46` | commentaire sur #85 |
| Santé / type de piles (alcaline vs NiMH), écart instantané − lissé | `0x46`, `0x49`, `0x5A` | commentaire sur #108 |
| Prévision d'autonomie plus précoce (la tension bouge avant le % entier) | `0x49` | #83, via #139 |
| Contrôle d'intégrité de la table d'étalonnage (3 copies) | `0x5A`, `0x60`, `0xEB` | déjà dans le code (`calib_mirror_*`) |
| Délai de veille affiché / prédit | `0xF5` | non ouvert : hypothèse à confirmer (§5) |
| État de charge, mode d'alimentation | aucun : le A1314 est à piles, aucun rapport ne varie avec une alimentation | sans objet |

## 8. Reproduire

```bash
python3 tests/live/re/sample_reports.py --scan-once                    # balayage 0x00-0xFF, une fois
python3 tests/live/re/sample_reports.py --interval 300 --count 13 --out s.jsonl
python3 tests/live/re/sample_reports.py --analyze s.jsonl              # résumé, sans matériel
python3 tests/live/re/idle_timeout.py --duration 7200 --out idle.jsonl # passif, horodatage seul
```

Sources : [hid-input.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-input.c) (quirks batterie Apple),
[hid-apple.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-apple.c) (quirks 0x0256),
[BCM2042 (fiche produit)](https://www.alldatasheet.com/html-pdf/175090/BOARDCOM/BCM2042/384/1/BCM2042.html) (puce HID BT avec interface batterie, sans table de registres publique).
