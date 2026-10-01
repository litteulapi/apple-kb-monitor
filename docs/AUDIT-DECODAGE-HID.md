# Audit du décodage HID — A1314 ISO (BCM2042)

Date : 2026-10-01. Périmètre : tout le code qui interroge le clavier et décode
ses réponses.

- `apihub-app/akm-core/src/decode.rs`, `calibration.rs`, `hidraw.rs`, `power.rs`,
  `model.rs`, `report.rs` ;
- les consommateurs `apihub-app/src/main.rs` (onglet Clavier) et
  `apple-kb-monitord/src/actor.rs` ;
- le script Python `apple-kb-monitor` (`hid_get_feature`, `read_all_reports`,
  `dump_all_reports`, tables de modèles).

Matériel : A1314 ISO, `HID_ID=0005:05AC:0256`, AA:BB:CC:DD:EE:F1, `/dev/hidraw7`,
noyau 7.1.13. Accès en **lecture seule** : `HIDIOCGFEATURE` uniquement, sans
`SET_REPORT`/`SET_FEATURE`, sans écriture, sans sudo, avec 0,4 s entre deux requêtes.

Marquage : **[mesuré]** = observé sur le clavier réel ou dans le code exécuté ;
**[source]** = documenté dans une référence externe citée ; **[hypothèse]** = non
démontré.

> **État au contre-audit (c54c502)** : les défauts #131-#134 et #136 sont corrigés dans `akm-core` (02fad76) ; le CLI
> Python les conserve (#200). Les numéros de ligne ci-dessous datent de l'audit.

## 1. Données de référence

### 1.1 Descripteur de rapport (sysfs `report_descriptor`, 224 octets, lecture statique)

| ID | Type | Usage | Taille |
|----|------|-------|--------|
| 0x01 | Input/Output | clavier standard, LED | 8+8+48 bits |
| 0x11, 0x12 | Input | Consumer (éjection, multimédia) | 8 bits |
| 0x13 | Input | page vendeur 0xFF01, usages 0x0A/0x0C | 8 bits |
| **0x47** | **Input** | Generic Device Controls 0x06 / **Battery Strength 0x20**, plage logique 0..255 | 8 bits |
| **0x09** | **Feature** | page vendeur 0xFF01 usage 0x0B, plage logique 0..1 + 16 bits de bourrage | 24 bits |

**[mesuré]** Le seul rapport *Feature* déclaré est **0x09**. Les 20 autres ID
de cet inventaire (0x46, 0x49, 0x4A, 0x4B, 0x4C, 0x4F, 0x51-0x53, 0x5A, 0x5B, 0x60,
0xEA, 0xEB, 0xF4-0xF7, 0xFE, 0xFF) ne sont **pas déclarés** (0x47 est déclaré en Input).
*Contre-audit* : le balayage complet ultérieur trouve 27 ID qui répondent, soit 25 non déclarés
(s'ajoutent 0x54, 0x5C, 0x5D, 0xD1, 0xD8 : HARDWARE-RAPPORTS-HID §2). Le descripteur ne
documente donc ni leur sens ni leurs unités : tout leur décodage relève de la
rétro-ingénierie du projet. Aucune référence publique ne décrit ces ID
(recherche du 2026-10-01 ; le seul travail public sur le microgiciel des claviers
Apple est K. Chen, Black Hat 2009, sur le modèle filaire).

### 1.2 Noyau [source]

`drivers/hid/hid-input.c`, tableau `hid_battery_quirks[]` : 0x0239/0x023A
(2009) et 0x0255/0x0256 (2011), ainsi que 0x022C, sont marqués
`HID_BATTERY_QUIRK_PERCENT | HID_BATTERY_QUIRK_FEATURE`. Le noyau lit donc 0x47
**en GET_REPORT Feature** (bien que le rapport soit déclaré Input), avec min=0 et
max=100. `drivers/hid/hid-ids.h` : 0x022c/d/e = ALU_WIRELESS ANSI/ISO/JIS,
0x0239/a/b = 2009, 0x0255/6/7 = 2011, 0x0267 = MAGIC_KEYBOARD_2015,
0x026c = NUMPAD_2015, 0x029a/c/f = 2021, 0x0320/1/2 = 2024, `BT_VENDOR_ID_APPLE` = 0x004c.

### 1.3 Transport [mesuré]

`bluetoothctl info` : `BREDR.Connected: yes`, Class 0x002540, UUID HID
**0x1124** (HID classique), pas d'UUID 0x1812 (HOGP). Le lien est en
**BR/EDR**, pas en LE. Le nœud hidraw passe par uhid (`/sys/devices/virtual/misc/uhid/…`) :
BlueZ relaie les GET_REPORT, et `profiles/input/device.c` impose son propre
délai d'expiration (`hidp_report_req_timeout`).

### 1.4 Les 22 rapports de l'inventaire initial (27 au balayage complet)

Captures du 2026-10-01 entre 03:57 et 04:00, conservées dans
`tests/live/re/capture-2026-10-01.jsonl` (0x4C caviardé) et
`tests/live/re/a1314_iso_frames.hex`. La longueur indiquée est la valeur de
retour exacte de l'ioctl (outil `tests/live/re/sample_reports.py`). Pendant la
même période, le `capacity` noyau valait **99**.

| ID | Longueur | Trame (tour A) | Variation observée sur 3 ou 4 lectures |
|----|---------:|----------------|-----------------------------|
| 0x09 | 4 | `09 01 00 00` | stable |
| 0x46 | 3 | `46 af 0b` | stable |
| 0x47 | 2 | `47 63` (99) | stable, **= noyau 99** |
| 0x49 | 3 | `49 89 0b` | stable |
| 0x4A | 2 | `4a 12` | stable |
| 0x4B | 3 | `4b 00 08` | stable |
| 0x4C | 20 | `4c 03 f2 ee dd cc bb aa` + 12 octets | — |
| 0x4F | 3 | `4f 50 00` | stable |
| 0x51/52/53 | 9 | « Clavier » / « de alice » / « #1 » | — |
| 0x5A / 0x60 / 0xEB | 9 | `0b8a 09ca 0964 0806` | identiques entre eux |
| 0x5B | 9 | `06cc 0384 00 00 00 00` | = 0xF4 ‖ 0xF5 |
| 0xEA | 2 | `ea 62` (98) | stable |
| 0xF4 | 3 | `f4 06 cc` (1740) | stable |
| 0xF5 | 3 | `f5 03 84` (900) | stable |
| 0xF6 / 0xF7 | 3 | `00 04` | stable |
| 0xFE | 9 | `00…00` | **`fe 00 04`** au dump de 03:57, des zéros à 03:58, puis aucune réponse à 04:00 |
| 0xFF | 4 | `ff 0b af 01` | **`0b a6` (2982) à 03:57, puis `0b af` (2991)** |

**Incident [mesuré]** : à 04:00:16, pendant le 3ᵉ tour d'échantillonnage, la
requête 0xFE (10ᵉ du tour) a expiré, puis 0xFF, 0x5B et 0x09 aussi : `errno=5`
(EIO) après 3,39 à 3,60 s chacune, avec `hidp_report_req_timeout` dans
`bluetoothd`. Le clavier a ensuite quitté le lien (`Host is down`, BlueZ ne
parvenait plus à se reconnecter). Le journal montre que le même clavier était
déjà « Host is down » à 02:55, avant tout accès de l'audit. *Contre-audit* : 02:55-03:06 est la fenêtre du
**changement de piles** (HARDWARE-RAPPORTS-HID en-tête, VERIF-BATTERIE) ; cet argument ne plaide donc **pas** pour
une mise en veille. La cause reste ouverte (#175 : la dernière requête était `0xFE`). Le lien de cause à effet
avec les lectures n'est pas établi, et l'échantillonnage a été arrêté.

## 2. Tableau champ par champ

Verdict : ✅ correct · ❌ faux · ⚠️ non prouvé (présenté comme certain).

| Champ exposé | Code | Décodage du code | Valeur réelle décodée | Verdict | Preuve | Correction proposée |
|---|---|---|---|---|---|---|
| `battery.percentage` (noyau) | `power.rs:128-139`, `decode.rs:272-275` | `capacity` power_supply | 99 | ✅ | [mesuré] `capacity`=99 = 0x47=0x63 ; [source] quirk PERCENT\|FEATURE | — |
| `battery.percentage` (0x47, sans noyau) | `decode.rs:146-150`, Py `289-291` | `buf[1]` | 99 | ✅ | identique au noyau [mesuré] | — |
| `battery.percentage_fine` (0xEA) | `decode.rs:142-144`, puis écrasé `decode.rs:254-257`, `actor.rs:192` | `buf[1]` « pre-rounding value » (`decode.rs:18`, Py `14`) | 98, puis **remplacé par 99 (noyau)** | ❌ sémantique / ⚠️ | 0xEA=98 et 0x47=99 de façon stable : un entier 98 « avant arrondi » ne peut pas donner 99, l'affirmation est **réfutée** [mesuré]. Avec le noyau, la valeur 0xEA n'est jamais exposée côté Rust (le champ vaut le noyau), alors que Python expose 98 sous le même nom | Renommer `raw_0xEA` et ne plus l'écraser ; retirer « pre-rounding » ; `battery_pct()` (`report.rs:79-85`) ne doit pas préférer 0xEA à 0x47 → #136 |
| `battery.adc_raw` (0xF5) | `decode.rs:152-158`, Py `299-302` | u16 **big-endian** `buf[1..3]` | 900 | ✅ (octets) | 0x0384=900 ; même valeur dans 0x5B[3..5] (BE) [mesuré] | — |
| `battery.voltage` | `calibration.rs:14-24`, Py `122-125,303-304` | `adc × 3,3 / 1023` (« 10-bit, 3.3 V reference ») | **2,903 V** (2,9032) | ⚠️ | Échelle jamais démontrée. 0x46 (LE)=2991, 0x49 (LE)=2953 et 0xFF (BE)=2982→2991 tombent tous dans 2,95–2,99 si on les lit en mV, ce qui est cohérent avec 2 piles AA neuves et un noyau à 99 %, mais à 50–90 mV de 2,903 [hypothèse]. `adc_ref`=1740 n'entre pas dans la formule. 900×1740 ne donne aucune tension plausible, et 900/1023×3,3 n'est qu'une hypothèse | Marquer « estimation » ; corréler 0xF5, 0x46, 0x49, 0xFF et le noyau sur une décharge (historique) avant d'afficher une tension à 3 décimales → #136 |
| `firmware.adc_ref` (0xF4) | `decode.rs:208-212`, Py `307-309` | u16 BE | 1740 | ✅ (octets) / ⚠️ (sens) | = 0x5B[1..3] [mesuré] ; « factory calibration constant » non démontré | Libellé « 0xF4 (brut) » |
| Courbe 0x5A | `calibration.rs:31-42`, Py `312-320` | 4 × u16 BE, mV, [100, 75, 50, 25 %] | 2954, 2506, 2404, 2054 (valide) | ✅ octets / ⚠️ sens | Strictement décroissante ; = 0x60 = 0xEB [mesuré]. Seuils = défaut + 54/56/54/54 mV. L'unité mV et l'association aux niveaux 100/75/50/25 % restent une [hypothèse] | Documenter comme hypothèse |
| `DEFAULT_CALIBRATION_MV` | `calibration.rs:19`, Py `275` | [2900, 2450, 2350, 2000] | — | ⚠️ | Aucune source. La courbe réelle diffère de +54 mV | Citer l'origine, ou n'utiliser aucun repli (pas d'interpolation sans courbe lue) |
| `battery.percentage_interpolated` | `decode.rs:160-165`, `calibration.rs:45-76`, Py `405-430` | linéaire par segments, 0 mV = 0 % | **97** | ✅ arithmétique / ⚠️ modèle | (2903−2506)/(2954−2506)=0,8862 → 75+22,15=97,15 → 97 en Rust **et** en Python [mesuré, rejoué sur les trames]. Sous 25 %, la décroissance linéaire jusqu'à 0 mV n'a pas de sens physique : à 1,8 V on afficherait 21,9 % alors que le clavier est hors service [hypothèse] | Borne basse = tension de coupure, pas 0 mV → #136 |
| Clamp « LOW » (0x09) | `decode.rs:221-229` | `buf[1]==0` et interpolé > 15 → 10 % | non déclenché (`01`) | ⚠️ | 0x09 est le seul Feature déclaré (usage FF01:0x0B, 0..1) [mesuré]. « 0 = LOW » n'est pas démontré, et la valeur 10 % est arbitraire | Exposer `state_flag` brut, ne pas réécrire un pourcentage |
| `detect_battery_type` | `calibration.rs:79-93`, `main.rs:171-173` | seuils de tension → chimie | « Alkaline (fresh) » | ⚠️ | Seuils sans source, appliqués à une tension elle-même non prouvée | Retirer de l'UI ou marquer « estimation » (#108) |
| `firmware.version` (0x4F) | `decode.rs:167-171`, Py `329-332` | quartets de `buf[1]` → « 5.0 » | « 5.0 » | ⚠️ | Trame réelle `4f 50 00` (3 octets) = u16 LE **0x0050** = version DID du Modalias BlueZ `usb:v05ACp0256d0050` [mesuré]. La lecture en quartets « 5.0 » est une [hypothèse], et le 2ᵉ octet est ignoré | Afficher `0x0050` (u16 LE), comme le DID → #136 |
| `firmware.build` (0xFF) | `decode.rs:173-177`, Py `335-338`, UI `main.rs:341-345` | u16 BE = « build number » | **2991** (2982 une minute plus tôt) | ❌ | La valeur **change entre deux lectures** (0x0BA6 puis 0x0BAF) et égale le u16 LE de 0x46 au même instant [mesuré]. Un numéro de build ne varie pas | Ne plus exposer comme build ; champ brut `raw_0xFF` → **#131** |
| `bluetooth.conn_interval_ms` / `slave_latency` (0x46) | `decode.rs:193-199`, Py `361-364`, UI `main.rs:270-278` | `buf[1]×1,25 ms`, `buf[2]` = latence (paramètres LE) | 218,75 ms, latence 11, UI « eff=2625 ms » | ❌ | Lien **BR/EDR** (§1.3) : l'intervalle de connexion en 1,25 ms et la latence esclave sont des paramètres de la couche liaison **LE** (Core Spec Vol 6 Part B), absents en BR/EDR [source]. Les 2 octets `af 0b` lus en u16 LE donnent 2991, la valeur de 0xFF [mesuré] | Supprimer l'interprétation LE ; champ brut → **#132** |
| `bluetooth.supervision_timeout_s` (0x49) | `decode.rs:201-206`, Py `366-370` | Rust : u16 LE × 10 ms ; Python : `d[1]` seul | Rust **29,53 s**, Python **137** | ❌ | Unités LE sur un lien BR/EDR [source]. Rust et Python décodent différemment la même trame [mesuré]. 0x0B89=2953 ≈ 1er seuil de 0x5A (2954) [hypothèse mV] | Idem → **#132** |
| `device.name` (0x51-0x53) | `decode.rs:179-191`, Py `341-347` | 3 blocs de 8 octets, coupés au NUL | « Clavier de alice #1 » | ✅ | Identique à `HID_NAME` et à BlueZ [mesuré]. Limite de 24 octets ; décodage `from_utf8_lossy` (Rust) contre `ascii, replace` (Python) | Python : décoder en UTF-8 |
| `bluetooth.identity_key` (0x4C) | `decode.rs:40-41,214-219`, Py `349-353`, UI `main.rs:317-321` | « BCM2042 internal identity key (NOT the BT MAC) » ; Rust = 19 octets dont l'octet de type ; Python = type + `d[2:16]` | Rust 19 octets ; Python **14 octets sur 18** | ❌ | Les octets 2-7 `f2 ee dd cc bb aa`, inversés, donnent **aa:bb:cc:dd:ee:f2 = `HID_PHYS`, l'adresse de l'adaptateur de l'hôte appairé** [mesuré]. Ce n'est pas une clé d'identité de l'appareil, ni une IRK (notion propre au LE). Les 12 octets restants ne sont pas identifiés [hypothèse : élément d'appairage]. La troncature Python a été rejouée [mesuré] | Exposer `paired_host` (adresse) et ne plus publier le reste (#123) → **#133** |
| `device.model` / `chip` (Rust) | `model.rs:42-145` | table PID | « A1314, aluminum, ISO », BCM2042 | ✅ PID / ⚠️ puce | PID conformes à `hid-ids.h` [source]. Puce BCM2042 pour l'A1314 de 2011 : [hypothèse], sans démontage cité | Citer la source |
| `device.model` / `chip` (Python) | Py `142-165,176-180,226` | table PID + plages de puces | — | ❌ | 0x0220 = ALU filaire (pas « A1016 ») ; 0x0229 = GEYSER4 ; 0x022C = ANSI (pas JIS) ; 0x024F/0x0250 = ALU_REVB filaire (pas « Magic Keyboard A1644 ») ; 0x0267/0x026C = Magic Keyboard 2015 et 2015 pavé numérique (pas « Touch ID A2449 ») ; 0x022D/E et 0x0239-B absents ; VID 0x004C refusé [source hid-ids.h]. Le correctif #64 n'a pas été porté | Aligner sur `model.rs` → **#135** |
| `device.mac` | `model.rs:190-197` | `HID_UNIQ` | AA:BB:CC:DD:EE:F1 | ✅ | [mesuré] | — |
| `battery_pct()` | `report.rs:79-85` | fine > interpolé > 0x47, puis `filter(is_finite)` **après** les `or` | 98 sans noyau (contre 99 pour 0x47 et le noyau) | ❌ mineur | Un NaN en tête ne passe pas au champ suivant (test `report.rs:102-105`). La priorité donnée à 0xEA repose sur l'affirmation réfutée | Ordre 0x47 > 0xEA ; filtrer chaque terme → #136 |

## 3. Couche d'accès (ioctl, fd, délais d'expiration)

| Point | Code | Verdict | Preuve |
|---|---|---|---|
| Constante `HIDIOCGFEATURE` | `hidraw.rs:19` = 0xC1004807 ; Py `113` | ✅ | `_IOC(RW=3, 'H'=0x48, 7, 256)` = 0xC0000000\|0x01000000\|0x4800\|0x07 = 0xC1004807. Python : 0xC0004807\|(len<<16) est équivalent pour len < 16384 |
| Tampon et `unsafe` | `hidraw.rs:22-28` | ✅ | tampon de 256 octets = taille encodée ; `buf[..ret]` borné, `ret ≤ 256` garanti par le noyau |
| `ret == 0` | `hidraw.rs:27,35` | ❌ mineur | `None` puis `io::Error::last_os_error()` : l'errno est **périmé** (sans rapport avec l'appel) → #134 |
| EINTR | `hidraw.rs:26` | ⚠️ | aucune relance ; un EINTR sur la sonde 0xEA ferme le fd et rend « clavier injoignable » → #134 |
| **Échec après la sonde** | `decode.rs:138-233` | ❌ | Seule la sonde 0xEA interrompt la séquence. Mesuré : 4 expirations consécutives de 3,39 à 3,60 s (EIO). Pire cas après une sonde réussie : 13 × ~3,6 s ≈ **47 s** de blocage du thread de l'acteur (`actor.rs:151`) et 13 requêtes HIDP sans réponse vers un clavier qui s'endort → **#134** |
| fd persistant | `hidraw.rs:117-147` | ✅ | `O_CLOEXEC`, fermeture sur échec de la sonde ; `F_GETFD` ne détecte pas un nœud retiré, mais l'ioctl suivant échoue (ENODEV) puis le fd est rouvert |
| Python `hid_get_feature` | Py `259-272` | ❌ | ouvre et ferme le nœud **à chaque rapport** ; ignore la longueur renvoyée par l'ioctl : un rapport court est complété par des zéros et décodé comme une valeur → **#137** |
| Python `--dump` | Py `508-534` | ❌ | ignore la longueur ; **masque** un rapport dont toute la charge vaut 0 (`any(buf[1:])`) : un 0x09=0 « LOW » serait invisible ; supprime les zéros de fin : 0x09 (4 octets) affiché `0901`, 0x4F (3) affiché `4f50`, 0xFE (9 octets nuls à 03:58) absent [mesuré] → **#137** |
| Balayage des 256 ID | Py `519` | ✅ | 3,6 s pour 256 ID, clavier éveillé : les ID non déclarés sont refusés vite, sans expiration [mesuré] |

## 4. Défauts confirmés (issues)

| # | Titre |
|---|-------|
| #131 | 0xFF décodé comme « firmware build » : la valeur varie entre deux lectures |
| #132 | 0x46/0x49 décodés en paramètres LE sur un lien BR/EDR ; Rust et Python divergent sur 0x49 |
| #133 | 0x4C « identity_key » = adresse de l'hôte appairé + 12 octets ; troncature Python |
| #134 | `decode_bcm2042` continue après une expiration : jusqu'à ~47 s de blocage ; errno périmé, pas de relance sur EINTR |
| #135 | Python : table modèles/puces fausse (correctif #64 non porté) |
| #136 | Sémantique réfutée ou non prouvée présentée comme certaine (0xEA « pré-arrondi », tension, 0x4F, priorité de `battery_pct`) |
| #137 | Python : longueur de l'ioctl ignorée ; `--dump` masque ou tronque des rapports |

## 5. Outils ajoutés (lecture seule)

- `tests/live/re/sample_reports.py` : échantillonne les 22 ID par HIDIOCGFEATURE
  uniquement, enregistre la longueur exacte, l'errno et la latence, espace les
  requêtes, et caviarde 0x4C par défaut.
- `tests/live/re/replay_python_decode.py` : rejoue des trames enregistrées dans
  `read_all_reports` du script Python, sans matériel.
- `tests/live/re/a1314_iso_frames.hex` : trames réelles (0x4C remis à zéro) au
  format `Fixture::from_hex_dump`, à brancher dans un test Rust.
- `tests/live/re/capture-2026-10-01.jsonl` : les captures brutes de l'audit.
