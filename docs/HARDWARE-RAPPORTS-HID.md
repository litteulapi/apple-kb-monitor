# Rapports HID Feature du clavier Apple A1314 (BCM2042) — carte de rétro-ingénierie

Appareil étudié : Apple Wireless Keyboard A1314 ISO « Clavier de maria #1 », `04:DB:56:CA:42:EE`,
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
| `0x47` | **Input** | Battery System `0x06` / Battery Strength `0x20`, 8 bits, 0-255 |
| `0x11` | Input | Consumer Eject `0xB8`, vendeur Apple `0xFF:0x03` (= touche Fn) |
| `0x12` | Input | Consumer Play/Pause, Next, Previous, Stop… |
| `0x13` | Input | vendeur `0xFF01:0x0A` (1 bit), `0xFF01:0x0C` (1 bit relatif) |
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
| `0x49` | 3 | `49 89 0b` | u16 LE = 2953 (mV ?) constant | [hypothèse] tension filtrée/de référence, voir §4 |
| `0x4A` | 2 | `4a 12` | u8 = 18 | inconnu |
| `0x4B` | 3 | `4b 00 08` | `00 08` | inconnu |
| `0x4C` | 20 | `4c 03` + 18 octets masqués | 1 octet type (`0x03`) + 18 octets à forte entropie | SENSIBLE ; [hypothèse] matériau de clé |
| `0x4F` | 3 | `4f 50 00` | u16 LE = `0x0050` = version firmware/bcdDevice | = `Modalias usb:v05ACp0256d0050` et « HID v0.50 » du noyau [mesuré] |
| `0x51` | 9 | `51` + `"Clavier "` | nom, fragment 1/4 (8 o ASCII) | [mesuré] |
| `0x52` | 9 | `52` + `"de maria"` | nom, fragment 2/4 | [mesuré] |
| `0x53` | 9 | `53` + `" #1"` + NUL | nom, fragment 3/4 | [mesuré] |
| `0x54` | 9 | `54` + 8 × NUL | nom, fragment 4/4 (vide ici) | [mesuré] ; nom ≤ 32 octets |
| `0x5A` | 9 | `5a 0b8a 09ca 0964 0806` | 4 × u16 BE : 2954, 2506, 2404, 2054 mV | valeurs [mesuré] ; seuils de décharge [hypothèse forte] |
| `0x5B` | 9 | `5b 06cc 0384 00000000` | concaténation de `0xF4` et `0xF5` + 4 zéros | [mesuré] |
| `0x5C` | 9 | 8 × `00` | vide | [mesuré] |
| `0x5D` | 9 | 8 × `00` | vide | [mesuré] |
| `0x60` | 9 | = `0x5A` | copie de `0x5A` | [mesuré] |
| `0xD1` | 2 | `d1 00` | u8 = 0 | inconnu |
| `0xD8` | 2 | `d8 00` | u8 = 0 | inconnu |
| `0xEA` | 2 | `ea 62` | u8 = 98 | [hypothèse] pourcentage interne non arrondi/autre filtre |
| `0xEB` | 9 | = `0x5A` | copie de `0x5A` | [mesuré] |
| `0xF4` | 3 | `f4 06 cc` | u16 BE = 1740 | constant ; [hypothèse] voir §4 |
| `0xF5` | 3 | `f5 03 84` | u16 BE = 900 | **constant à travers un changement de piles : ce n'est pas la tension** [mesuré] |
| `0xF6` | 3 | `f6 00 04` | 4 | inconnu |
| `0xF7` | 3 | `f7 00 04` | 4 | inconnu |
| `0xFE` | 9 | 8 × `00` | vide | [mesuré] |
| `0xFF` | 4 | `ff 0b aa 01` / `ff 0b af 01` | **u16 BE = même tension que `0x46`** + octet `0x01` | [mesuré], voir §3 |

Endianness : `0x46`, `0x49`, `0x4F` sont en petit-boutiste ; `0x5A`/`0x5B`/`0xF4`/`0xF5`/`0xFF` en gros-boutiste.
Le firmware mélange donc deux conventions ; `0x46` (LE) et `0xFF` (BE) donnent la même grandeur.

## 3. Tension réelle des piles : `0x46` et `0xFF`, pas `0xF5`

* **[mesuré]** Dans une même salve de lectures, `0x46` lu en LE et `0xFF[1..3]` lu en BE donnent la même
  valeur (balayage : 2986/2986 ; échantillon 03:57:53 : 2991/2991). Le dump initial de l'inventaire
  (lectures non simultanées) montrait 2991 (`0x46`) et 2986 (`0xFF`) : la grandeur bouge entre deux
  lectures, ce qu'une constante de configuration ne fait pas.
* **[mesuré]** 2,99 V pour deux piles AA alcalines neuves (≈ 1,50 V/élément) : cohérent physiquement.
* **[mesuré]** `0xF5` vaut `0x0384` (900) **avant et après le changement de piles** du 2026-10-01
  (historique `~/.config/apple-kb-monitor/history.jsonl` : « voltage » 2,9032 V = 900 × 3,3 / 1023 à 90 %
  à 02:33 avec les anciennes piles, puis à 100 % à 03:07 avec les neuves). Une tension de piles
  ne peut pas rester identique au millivolt entre un jeu usé (90 %) et un jeu neuf.
  En avril 2026 l'historique donnait 924 (2,98 V) pendant 2 jours sans la moindre variation.
* Conséquence : la « tension » affichée aujourd'hui par le CLI Python (`adc_raw * 3.3 / 1023`) et par
  `akm-core` (`calibration.rs`, `decode.rs` : `0xF5` = « ADC raw », `0x46` = « connection params »,
  `0xFF` = « firmware build ») est fausse. Le « numéro de build » `0x0BAA` est en fait 2986 mV.

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

SECTION_SERIE_TEMPORELLE

## 6. Synthèse : décodé / décodable / inconnu

| Statut | Rapports |
|---|---|
| **Décodé** (preuve mesurée) | `0x46` tension mV LE · `0xFF` tension mV BE + octet `0x01` · `0x47` % · `0x4F` version `0x0050` · `0x51-0x54` nom 32 o · `0x5A`/`0x60`/`0xEB` table de 4 tensions (copies) · `0x5B` = `0xF4`‖`0xF5` · `0x5C`/`0x5D`/`0xFE` vides |
| **Décodable** (hypothèse testable sans écriture) | `0x49` tension filtrée · `0xEA` second estimateur % · `0xF5` délai de veille · `0xF4` coupure · octet 3 de `0xFF` (drapeau d'état, à corréler avec pile faible) · `0x4C` type `0x03` + 18 o |
| **Inconnu** (constant, aucune corrélation possible en lecture) | `0x09` (`FF01:0B` = 1) · `0x4A` (18) · `0x4B` (`00 08`) · `0xD1`/`0xD8` (0) · `0xF6`/`0xF7` (4) |

Les rapports constants ne peuvent être élucidés qu'en écrivant (SET_REPORT), ce qui est exclu, ou en
comparant plusieurs claviers / firmwares (A1314 d'une autre révision, A1255).

## 7. Fonctions utilisateur rendues possibles

| Fonction | Rapports | Suivi |
|---|---|---|
| Tension réelle des piles (mV) au lieu d'une valeur figée | `0x46` / `0xFF` | bug à corriger (CLI + akm-core) |
| Détection fiable du remplacement de piles : saut de tension ≥ 150 mV à la reconnexion, indépendant du % | `0x46` | complète #85 |
| Santé / type de piles : tension sous charge (`0x46`) vs filtrée (`0x49`), écart ↔ résistance interne | `0x46`, `0x49` | complète #108 |
| Pourcentage fin (0,1 %) calculé sur la courbe d'usine de l'unité | `0x46` + `0x5A` | nouvelle issue |
| Contrôle d'intégrité de la table d'étalonnage (3 copies identiques) et alerte si divergence | `0x5A`, `0x60`, `0xEB` | déjà dans le code (`calib_mirror_*`) |
| Nom interne complet (32 o) : le code ne lit que `0x51-0x53` (24 o) → nom tronqué au-delà | `0x51-0x54` | nouvelle issue |
| Inventaire matériel : version firmware `0x4F`, révision | `0x4F` | existant |
| Prédiction de mise en veille / délai d'inactivité affiché | `0xF5` (si confirmé) | selon §5 |
| Mode d'alimentation / état de charge | aucun rapport ne varie avec l'alimentation : le A1314 est à piles, pas d'état de charge | sans objet |
