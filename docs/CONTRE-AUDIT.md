# Contre-audit des documents de rétro-ingénierie — A1314 ISO (BCM2042)

Date : 2026-10-01. Base : `main` @ `c54c502`, branche `docs/contre-audit`.
Périmètre : README.md, docs/FEATURES.md, HARDWARE-RAPPORTS-HID, RE-HID-EXHAUSTIF, RE-LIAISON-BLUETOOTH,
RE-COMMANDES-VENDEUR, RE-FIRMWARE-MAINTENANCE, RE-FIRMWARE-EXTRACTION, RE-PILOTE-MACOS, VERIF-BATTERIE,
AUDIT-DECODAGE-HID, RECONNEXION-PAIRAGE, wiki Gitea (9 pages).

**Méthode.** Aucun accès au clavier, aucun sudo, aucun service touché. Chaque affirmation a été refaite
indépendamment à partir :

* des données brutes commitées : `tests/fixtures/a1314_iso/` (descripteur 224 o, `reports_scan_20261001.json`,
  `reports_timeseries_20261001.json`, `exhaustive_{feature,input,handshake}_20261001.json`,
  `link_btmon_stats_20261001{,_b}.json`, `link_sdp_decoded.txt`, `link_mgmt_sysconfig.txt`, `link_conn_info.txt`),
  `tests/live/re/capture-2026-10-01.jsonl`, `a1314_iso_frames.hex`, `verif_series_20261001.jsonl` ;
* d'un décodeur de descripteur HID écrit pour l'occasion (item par item) et de recalculs Python
  (endianness, interpolations, moyennes, horodatages epoch → heure locale) ;
* des sources publiques re-téléchargées le 2026-10-01 : `hid-input.c`, `hid-apple.c`, `hid-ids.h`, `uhid.c`,
  `hidraw.c` (torvalds/linux master), BlueZ `profiles/input/device.c`, FreeBSD `bthidd/hid.c`, Infineon BTSDK
  `app.h`, product brief Broadcom `2042-PB03-R` (digchip), fiche Sunitec BM2042, paper Chen BH 2009, iFixit 216345,
  managingosx 2014, elatov 2012, AUR RPC ;
* du code : rejeu de `_read_all_reports_fd` (CLI Python) sur les trames réelles, `cargo test -p akm-core`
  (191 passés, 1 ignoré), `pytest tests/` (128 passés).

`0x4C` : seuls la longueur, l'octet 1 et l'adresse de l'hôte (déjà publique, `HID_PHYS`) ont été manipulés ;
les 12 octets restants n'existent masqués que sous forme d'empreinte dans les fixtures et n'ont pas été lus.

Statuts : **✅ confirmé** · **❌ faux (corrigé)** · **⚠️ surqualifié (étiquette corrigée)** · **❓ invérifiable ici**.

## 1. Bilan

| | Nombre |
|---|---|
| Affirmations factuelles vérifiées | **118** |
| Confirmées | 64 |
| Fausses (corrigées dans les docs, ou issue de code) | 40 |
| Surqualifiées (niveau de preuve abaissé ou précisé) | 10 |
| Invérifiables depuis le dépôt | 4 |
| Documents corrigés (1 commit chacun) | 12 + 4 pages wiki |
| Issues de code ouvertes | 2 (#200, #201) |

## 2. Tableau des affirmations

### 2.1 Descripteur HID et carte des rapports

| # | Affirmation (document) | Statut | Preuve | Correction |
|---|---|---|---|---|
| 1 | Descripteur de 224 o, identique hidraw/SDP (HARDWARE §1, RE-LIAISON §1.1) | ✅ | `report_descriptor.bin` = `.hex` = attribut SDP 0x0206, comparaison octet à octet | — |
| 2 | Seul `0x09` est déclaré en Feature ; `0x47` en Input (HARDWARE §1, AUDIT §1.1) | ✅ | décodage des items : un seul `0xB1` (après `85 09`) | — |
| 3 | `0x47` = « Battery System `0x06` » (HARDWARE §1) | ❌ | `05 06 09 20` : page 0x06 = *Generic Device Controls* ; Battery System = page 0x85 | corrigé |
| 4 | `0x12` = Play/Pause, Next, Previous, **Stop** (HARDWARE §1) | ❌ | usages `0xCD 0xB3 0xB4 0xB5 0xB6` ; pas de `0xB7` | corrigé (FF, Rewind, Scan Next/Prev) |
| 5 | `0x13` usage `0x0C` « 1 bit relatif » (HARDWARE §1) | ❌ | `81 22` = Data,Var,**Abs**,No Preferred | corrigé |
| 6 | `0x11` : Eject + vendeur `0xFF:0x03` = Fn (HARDWARE §1, RE-PILOTE §2.2) | ✅ | `09 b8 81 02 06 ff 00 09 03 81 02` ; = `FnModifierUsagePage/Usage` macOS | précision : 3 + 3 bits de bourrage |
| 7 | `0x01` : 8 modificateurs, 1 octet réservé, 5 LED, 6 touches | ✅ | items décodés | — |
| 8 | 27 IDs Feature répondent, 218 `0x02`, 11 `0x03` (RE-HID §2.1) | ✅ | `exhaustive_handshake` : 27+11+218 = 256 | — |
| 9 | Input : 5 IDs répondent (`01 04 05 11 30`), 251 refusés | ✅ | `exhaustive_input` + handshake | — |
| 10 | « 21 rapports non documentés » (README) | ❌ | 27 répondent dont 25 non déclarés | corrigé |
| 11 | « Les 21 autres ID » (AUDIT §1.1) | ❌ | la liste en compte 20 ; balayage complet : 25 | corrigé |
| 12 | `0x46` u16 LE = 2986/2991 mV ; `0x49` = 2953 → 2950 | ✅ | `aa0b`=2986, `af0b`=2991, `890b`=2953, `860b`=2950 | — |
| 13 | `0xFF[1..3]` u16 BE = `0x46` au même instant | ✅ | scan 2986/2986 ; tour A 2991/2991 ; série : écarts ≤ 1 pas | — |
| 14 | Dump 03:57:18 : `0x46` = 2991, `0xFF` = **2986** (HARDWARE §3) | ❌ | `ff0ba601` → 2982 | corrigé (le tableau §5 du même doc disait déjà 2982) |
| 15 | Échantillon « 03:57:53 » (HARDWARE §3, §5) | ❌ | aucun horodatage commité ; tour A = epoch 1790819904-913 = 03:58:24-33 | corrigé |
| 16 | `0x5A` = 2954/2506/2404/2054 mV BE ; = `0x60` = `0xEB` | ✅ | `0b8a 09ca 0964 0806` ; 3 copies identiques sur 13 salves + scans | — |
| 17 | Par élément : 1,48/1,25/1,20/1,03 V | ✅ | /2 = 1,477/1,253/1,202/1,027 | — |
| 18 | `0x5B` = `0xF4` ‖ `0xF5` + 4 zéros | ✅ | `5b06cc038400000000` | — |
| 19 | `0xF4` = 1740, `0xF5` = 900, constants | ✅ | 13 salves + scan + passe 12:02, une seule valeur | — |
| 20 | `0xF5` × 3,3/1023 = 2,9032 V ; 924 → 2,98 V | ✅ | 900×3,3/1023 = 2,9032 ; 924 → 2,9806 | — |
| 21 | `0xF5` = « 10-bit ADC, 3.3 V ref » (README, wiki) | ❌ | constant à travers un changement de piles ; aucune source ne publie l'ADC | corrigé |
| 22 | `0x4F` u16 LE = `0x0050` = Version SDP | ✅ | `4f 50 00` ; SDP 0x0203 = 0x0050 | — |
| 23 | `0x4F` = « 5.0 » (CLI Python) | ❌ | rejeu : `fw_version "5.0"` | issue #200 |
| 24 | Nom : `"Clavier "` + `"de alice"` + `" #1"` = 19 o, 4 × 8 o | ✅ | hex des 4 trames | — |
| 25 | `0x4C` : 20 o, octet 1 = `03`, octets 2-7 = hôte inversé | ✅ | `capture…jsonl` `4c03f2eeddccbb…` ; `bonded_host` de la série | — |
| 26 | `0x4C` = « 128-bit internal key » (README) / « identity key » (CLI) | ❌ | 1 + 6 + 12 o ; adresse de l'hôte | corrigé ; CLI : #200 |
| 27 | `0x4C` stable (empreinte identique) | ✅ | `f34626564fca9676` sur 13 salves + scan + passe 12:02 | — |
| 28 | CLI Python publie `d[2:16]` = 14 o (adresse + 8 secrets) | ✅ | rejeu : `identity_key` de 14 o ; l. 468 | issue #200 (les #123/#133 fermés ne couvraient que Rust) |
| 29 | `0xEA` = « pré-arrondi » (README) | ❌ | 98 quand `0x47` = 99 | corrigé |
| 30 | `0xEA` = 0 une fois à 04:40:31 | ✅ | salve 6 : `ea00` | — |
| 31 | `0x09` = « 1=OK, 0=LOW » (README) | ❌ | aucune preuve ; macOS : drapeau de délai Verr. Maj (déduction RE-PILOTE) | corrigé |
| 32 | `0x46` = intervalle + latence, `0x49` = supervision, `0x4A` = alimentation, `0x4B` = classe (README) | ❌ | `0x46`/`0x49` = mV ; `0x4A`/`0x4B` inconnus | corrigé |
| 33 | `0xFF` = « build number » (README, wiki) | ❌ | varie entre lectures, = mV | corrigé |
| 34 | 13 salves de 27 GET_REPORT, aucune erreur (HARDWARE §5) | ✅ | 13 × 27 clés, aucun champ d'erreur | — |
| 35 | Tableau §5 (04:15 → 05:15) | ✅ | recalcul ligne à ligne identique | — |
| 36 | `0xFE` `fe0004` = « probable artefact de l'outil » (HARDWARE §2, VERIF §5) | ❌ | `--dump` d'avant 2219efa : tampon neuf par ID, filtre `any(buf[1:])` → le `04` vient de la réponse | corrigé dans les deux docs ; RE-HID avait raison |
| 37 | `0xFE` « non reproduit en 14 lectures exactes » | ❌ | 16 lectures exactes commitées (scan, tours A et B, 13 salves) | corrigé |
| 38 | Pas de 0x46/0xFF ≈ 4,5 mV = « un pas d'ADC » | ⚠️ | écarts 4-5 mV mesurés ; la résolution n'est publiée nulle part | requalifié en hypothèse |

### 2.2 Batterie et pourcentage

| # | Affirmation | Statut | Preuve | Correction |
|---|---|---|---|---|
| 39 | Interp(2953) = 99,94 ; (2950) = 99,78 ; (2945) = 99,50 ; (2935) = 98,94 | ✅ | 75 + 25·(v−2506)/448 | — |
| 40 | 1 point = 17,9 mV par paire (segment 100-75) | ✅ | 448/25 = 17,92 | — |
| 41 | À 12:02 : `0x46` 2974-2982, `0xFF` 2969-2978, `0x49` 2945, `0x47` 98 | ✅ | `exhaustive_feature` (tampons ≥ 3) | — |
| 42 | `0x49` « stable sur 11 lectures » (RE-HID §2.1) | ❌ | 9 lectures de 3 o (tampon 1 refusé, tampon 2 tronqué) | corrigé |
| 43 | `0x46`/`0xFF` « en baisse de 2991 à 2982 » (RE-HID §4) | ❌ | plages 2974-2982 / 2969-2978 | corrigé |
| 44 | Moyenne `0x46` 04:15-05:15 = 2988,3 (σ 2,5) ; `0xFF` 2987,5 ; écart à `0x49` 38 mV | ✅ | 6×2991 + 7×2986 ; 4×2991 + 9×2986 | — |
| 45 | Série 12:30-13:21 : 11 salves, `0x47` = 96, `0x49` = 2935, `0x46` 2978 → 2986 | ✅ | `verif_series_20261001.jsonl` (12 lignes dont 1 arrêt) | — |
| 46 | 96 % ↔ 2882-2900 mV ; 90 % ↔ 2775-2793 mV | ✅ | inversion de l'interpolation | — |
| 47 | Tableau tension → % par chimie | ⚠️ | reproduit par `verif_battery_model.py` ; courbes lues « à l'œil » (le doc le dit) | — (étiquette [modèle] déjà présente) |
| 48 | 2 piles AA, 2,5 Ah à 1,9 mA ≈ 55 jours (RE-LIAISON §3.3) | ✅ | 2500/1,88 = 1330 h | — |
| 49 | UPower : 2 GET `0x47` / 30 s ; 137 réponses `0x47` en 1 528 s | ✅ | `link_btmon_stats` ; silences de 30,0 s | — |
| 50 | Noyau : chaque lecture de `capacity` = GET_REPORT ; limite 30 s seulement dans `hidinput_update_battery` (VERIF §1.3) | ✅ | `hid-input.c` : `ratelimit_time` + 30 000 ms ; `avoid_query` | — |
| 51 | `hid-input.c` « lecture limitée à une toutes les 30 s » (RE-HID §3) | ❌ | voir #50 | corrigé |
| 52 | Tampon noyau `max(len, 4)` | ✅ | `hidinput_query_battery_capacity` | — |
| 53 | Quirk `PERCENT \| FEATURE` pour 0x0239/0x023A/0x0255/0x0256/0x022C | ✅ | `hid_battery_quirks[]` | — |
| 54 | `apple_fetch_battery` « confirme les observations UPower » (MAINTENANCE §3.1) | ❌ | exige `APPLE_RDESC_BATTERY`, absent pour 0x0256 ; minuteur 60 s | corrigé |
| 55 | Courbe « typique » 2900/2450/2350/2000 (README) | ❌ | défaut du code sans source ; mesuré 2954/2506/2404/2054 | corrigé |
| 56 | `0x47` = tronc(interp(`0x49`)) (HARDWARE §4) | ⚠️ | vrai à 2953/2950, faux à 2945 (98 au lieu de 99) et 2935 (96 au lieu de 98) | renvoi #179 ajouté |
| 57 | Voltage de l'historique = constante `0xF5` (VERIF §4, #180) | ✅ | 2,9032/2,9777/2,9806 V = 900/924/923 × 3,3/1023 | — |

### 2.3 Liaison Bluetooth

| # | Affirmation | Statut | Preuve | Correction |
|---|---|---|---|---|
| 58 | SDP : HID 1.0, ParserVersion 0x0111, pays 13, sous-classe 0x40, booléens | ✅ | `link_sdp_decoded.txt` | — |
| 59 | `HIDSupervisionTimeout` 0x1F40 = 8000 slots = 5 s | ✅ | 8000 × 0,625 ms | — |
| 60 | Supervision effective ≈ 20 s = 0x7D00 | ✅ | 12:13:08.4 → 12:13:28.8 (20,4 s) ; 32000 × 0,625 ms | — |
| 61 | Page scan 0x0800 = 1,28 s, fenêtre 0x0012 = 11,25 ms (0,9 %) ; page timeout 5,12 s | ✅ | `link_mgmt_sysconfig.txt` (LE) ; 11,25/1280 = 0,88 % | — |
| 62 | Sniff 20 slots = 12,5 ms, 260 entrées ; host sniff_min/max 80/800 | ✅ | stats ; sysconfig 0x0008/0x0009 = 0x0050/0x0320 | — |
| 63 | Actif 0,65 % (9,9 s) ; séjours actifs 25-55 ms, médiane 40 | ✅ | `sniff.sejours_actif` | — |
| 64 | Réveil : < 0,5 s n = 208, 6-246 ms ; 0,5-25 s n = 20, 540-1016 ms (RE-LIAISON §3.3) | ❌ | n = 207, 6-53 ms ; n = 21, 246-1016 ms | corrigé |
| 65 | ≥ 25 s : n = 31, ≈ 1 000 ms | ✅ | médiane 998 ms | — |
| 66 | 812 GET ; 353 DATA + 459 HANDSHAKE (448 `0x02` + 11 `0x03`) ; 1 121 rapports d'entrée | ✅ | stats | — |
| 67 | 1 081 en sniff + 2 actif + 38 avant le 1er Mode Change = 1 121 | ✅ | somme | — |
| 68 | Écart moyen à la grille 0,53 ms vs 3,125 attendu | ✅ (attendu) / ❓ (0,53) | 12,5/4 = 3,125 ; 0,53 non présent dans le JSON commité | — |
| 69 | RSSI 0 ×30, −1 ×3, −2 ×2, −3 ×2 (37 lectures) ; TX hôte 8 dBm | ✅ | 37 `Read RSSI` ; `link_conn_info.txt` | — |
| 70 | RSSI BR/EDR = écart à la Golden Receive Power Range, pas des dBm | ✅ | Core Vol 4 Part E §7.5.4 | README « 0 dBm » corrigé |
| 71 | Features LMP de référence `bc 02 04 38 08 00 00 00`, LMP 2.0, subversion 0x31C, Apple (76) | ✅ | page elatov revérifiée ; décodage bit à bit refait | — |
| 72 | BlueZ GET_REPORT = en-tête + ID, sans BufferSize ; `REPORT_REQ_TIMEOUT 3` (device.c:647) | ✅ | `hidp_send_get_report`, ligne 647 | — |
| 73 | uhid : 5 s, `min3(...)` ; hidraw `count < 2` → « too short » | ✅ | uhid.c l. 198, 266 ; hidraw.c l. 136/220 | — |
| 74 | 2 816 ioctl par passe, 2 560 transmises, 512 « too short », 4 800 refus, 5 632 au total | ✅ | 256 × 11 ; 256 × 10 ; 256 × 2 ; (229 + 251) × 10 | — |
| 75 | Probabilité « (1/27)² = 0,14 % » (RE-HID §0.2) | ❌ | séquences de 14 et ~27 IDs → 1/14 × 1/27 ≈ 0,26 % | corrigé, modèle marqué hypothèse |
| 76 | Coupure 04:00 : `0xFE` → EIO après 3 393 ms, puis `0xFF`, `0x5B`, `0x09` à ~3,6 s | ✅ | `capture…jsonl` | — |
| 77 | « Host is down à 02:55 avant tout accès → veille probable » (AUDIT §1.4) | ❌ | 02:55-03:06 = changement de piles | corrigé |
| 78 | Un clavier qui s'endort envoie un LMP detach (RECONNEXION §3.6) | ⚠️ | jamais observé sur ce clavier | étiqueté hypothèse |
| 79 | 9 pages sans réponse (RECONNEXION) vs 7 (RE-LIAISON) | ✅ | fenêtres différentes (12:16:34 vs fin de copie 12:15:34, 7 `Connect Complete` dans `stats_b`) | précisé |
| 80 | Coût UPower ≈ 0,05 mA ; 16 req./15 min démon = 21 % | ✅ | 1,6 mA·s / 30 s ; 16/(16+60) | — |

### 2.4 Puce, firmware, sources externes

| # | Affirmation | Statut | Preuve | Correction |
|---|---|---|---|---|
| 81 | BCM2042 = **ARM7TDMI** (README) | ❌ | brief 2042-PB03-R : « On-board 8051 processor » | corrigé (README, wiki) |
| 82 | BCM2042 = 8051 (MAINTENANCE, LIAISON, EXTRACTION) | ✅ | idem | — |
| 83 | 108 KB ROM + 22 KB RAM + 20 KB Boot ROM ; ROM-based ; Flash option | ✅ | schéma bloc du brief | « ROM masquée » marqué déduction |
| 84 | Bluetooth « 2.0 + EDR » [brief] (MAINTENANCE §0, README) | ❌ | le brief dit « version 2.0 compliant » ; seul le module BM2042 dit « 2.0+EDR compatible » ; features LMP de référence sans EDR | corrigé |
| 85 | Alimentation « 2,7 V – 3,3 V » (MAINTENANCE, VERIF §2) | ⚠️ | étiquette du schéma ; module BM2042 : VBAT 1,7-3,6 V, « dual output … 2.7-3.3 V » | précisé dans les deux docs |
| 86 | Module BM2042 : sniff 10 ms 2,35 mA … 1,28 s 0,018 mA ; sommeil 50 µA ; profond 16 µA ; 0 dBm typ, +4 max | ✅ | PDF Sunitec | — |
| 87 | « 128K serial EEPROM » | ⚠️ | fiche : « EEPROM Size 128Kbit » | précisé (16 Ko) |
| 88 | Pas de mode test / debug public (EXTRACTION §1.3, §3.4) | ❌ | fiche BM2042 : UART de debug, Boot-ROM « waits for download » si UP_RX = 1 | corrigé |
| 89 | Bank switching P1[3:2] dans la datasheet (EXTRACTION §3.1) | ❓ | absent du brief ; datasheet alldatasheet en 403 | marqué non revérifié |
| 90 | iFixit : BCM2042 + EEPROM STMicro dans l'A1255 | ✅ | page 216345 revérifiée | — |
| 91 | Puce de l'A1314 0x0256 = BCM2042 | ⚠️ | aucun démontage d'A1314 cité ; LMP fabricant « Apple (76) » | README : « identifié sur l'A1255 » |
| 92 | Magic Keyboard = BCM20733 (README, wiki) | ❓ | aucune source citée ni trouvée | remplacé par « non vérifiée » |
| 93 | Firmware « signé » (README) | ❌ (non démontré) | Chen 2009 = clavier filaire USB Cypress ; updater BT `bfu` non analysé pour la signature | README : signature inconnue |
| 94 | « Gén. 2009 sans signature » appliqué au A1314 (MAINTENANCE §1.4, §5) | ⚠️ | même raison | corrigé |
| 95 | Chen : 0xCAFEBABE, PIDs 0x220/221/222/228, blocs de 83 o, `wValue 0x030a`, `ff 38 00 01…07`, CY7C63923 | ✅ | paper PDF revérifié | — |
| 96 | Citation « No cryptographic signature of the firmware » | ❓ | absente du *paper* ; peut-être dans les *slides* | signalé |
| 97 | Notre A1314 ISO = PID `0x023a` (MAINTENANCE §1.4) | ❌ | HID_ID `0005:05AC:0256` ; `0x023a` = 2009 ISO | corrigé |
| 98 | `hid-ids.h:169` = 0x0256 ; `hid-apple.c:1087` = entrée 2011_ISO, `NUMLOCK_EMULATION \| HAS_FN \| ISO_TILDE_QUIRK` | ✅ | fichiers master | — |
| 99 | `fnmode` 3 = auto : « 4 sur MBP14/16, 2 sinon » (MAINTENANCE §3.1) | ❌ | `hid-apple.c:438-446` : 4 si `DISABLE_FKEYS`, 2 si non-Apple, **1 sinon** | corrigé |
| 100 | « Effacer l'IRK/bond » (MAINTENANCE §2.1) | ❌ | IRK = LE ; lien BR/EDR = clé de lien | corrigé |
| 101 | PIDs hid-ids.h des 17 modèles, `BT_VENDOR_ID_APPLE` 0x004c | ✅ | hid-ids.h l. 98, 129-131, 165-178 | — |
| 102 | `APPLE_MODELS` dans `apihub-app/src/keyboard.rs` (README, wiki) | ❌ | `apihub-app/akm-core/src/model.rs`, 17 entrées | corrigé |
| 103 | bthidd : `BATT_STAT_REPORT_ID 0x30`, `BATT_STRENGTH 0x47`, `{0x53, 0xd7, 0x01}` | ✅ | FreeBSD main | — |
| 104 | WICED `RPT_ID_IN_SLEEP 0x04`, `FUNC_LOCK 0x05`, `CNT_CTL 0xcc` | ✅ | app.h l. 111-129 | — |
| 105 | … = « preuve directe » que `0x04`/`0x05` du A1314 sont SLEEP/FUNC_LOCK (COMMANDES §2.1) | ⚠️ | SDK de puces CYW récentes ; IDs inconnus de macOS | rétrogradé en analogie |
| 106 | `0x55` = « registre de configuration vendeur » ; `0x40/41/44/45` = « bloc batterie » (COMMANDES §1) | ❌ | plist macOS PID 598 : LongDeviceName, WillShutdown, RecantConnection, FullFactoryDefault, FactoryDefault | corrigé |
| 107 | managingosx : `BatteryPercent`, `BatteryLow`, `BatteryPanic`, `"Battery" = <"MVLT…` | ✅ | page revérifiée | — |
| 108 | « MVLT réfuté » (RE-PILOTE §6) | ⚠️ | seule la conversion mV → % par macOS est réfutée ; le bloc existe (2014), origine inconnue | précisé |
| 109 | Carte `ExtendedFeatures`, cadence 4 h, SUSPEND, `WillShutdown` (RE-PILOTE) | ❓ | analyse sur le Mac du gérant, aucun extrait commité ; cohérente avec nos mesures (refus `0x03` des 6 IDs, `0x43` refusé `0x02`) | signalé non reproductible |

### 2.5 README, FEATURES, wiki : fonctions annoncées

| # | Affirmation | Statut | Preuve | Correction |
|---|---|---|---|---|
| 110 | Paquet AUR `yay -S apple-kb-monitor` + badge | ❌ | AUR RPC : `resultcount 0` | retiré |
| 111 | Modules `power.rs`, `history.rs`, `bluez.rs`, `rssi.rs` dans `apihub-app/src` (FEATURES, wiki Architecture) | ❌ | `apihub-app/src` : fnmode_diag, instance, keyboard, main, portal, rename, source, tray, view ; le reste dans `akm-core` / `apple-kb-monitord` | corrigé |
| 112 | `kde/DeviceItem.qml` patch Bluedevil ; `post_install` patche Bluedevil (FEATURES, wiki Installation) | ❌ | dossier `kde/` absent ; `.install` sans Bluedevil | retiré |
| 113 | `dbus/com.agenceapi.AppleKbMonitor.conf` (FEATURES) | ❌ | absent ; `dbus/` = 2 fichiers `.service` d'activation | corrigé |
| 114 | Widget Plasma lit `apple-kb-monitor --json` (FEATURES) | ❌ | `plasma/…/DaemonLink.qml` : D-Bus `AppleKbMonitor1` | corrigé |
| 115 | Wiki Installation : « démon CLI optionnel apple-kb-monitor.service » | ❌ | daemon principal = `apple-kb-monitord.service` ; le Python est en `Conflicts=` | corrigé |
| 116 | 2 onglets Keyboard et Diag ; RSSI délai 1,5 s, cache 10 s | ✅ | `main.rs:145-146` ; `rssi.rs:23-24` | — |
| 117 | Exemple `--once` « 2.981V » (README) | ❌ | tension issue de `0xF5` (constante) | remplacé par un avertissement |
| 118 | Politique de lecture du démon : `0x47/0x46/0x49`, fenêtre 60 s, 250 ms, 2 s, verrou (RECONNEXION §5.2) | ✅ | `read_policy.rs` (`ALLOWED`, `ACTIVE_WINDOW`, `MIN_GAP`, `BUDGET`) | — |

## 3. Contradictions entre documents : résolution

| Sujet | Versions en présence | Tranché | Pourquoi |
|---|---|---|---|
| Cœur du BCM2042 | ARM7TDMI (README) / 8051 (MAINTENANCE, LIAISON, EXTRACTION) | **8051** | brief Broadcom 2042-PB03-R, revérifié |
| `0xF5` | « ADC brut 10 bits » (README, CLI) / « constant » (HARDWARE) | **constante de sens inconnu** | 900 avant et après un changement de piles ; 13 salves identiques |
| `0x4C` | « clé d'identité 128 bits » (README, CLI) / « adresse de l'hôte + 12 o » (HARDWARE, #133) | **adresse de l'hôte + 12 o sensibles** | octets 2-7 inversés = `HID_PHYS` ; 1 + 6 + 12 o |
| `0xFF` | « build » (README) / « tension » (HARDWARE) | **tension (BE, mV) + octet 0x01** | varie entre lectures ; = `0x46` |
| Firmware signé | « Signed » (README) / « sans signature » (MAINTENANCE) | **inconnu** | Chen = autre produit (filaire USB) |
| RSSI | « 0 dBm » (README, CLI) / relatif (LIAISON, #174) | **relatif (dB)** | Core Spec Vol 4 Part E §7.5.4 |
| `0xFE` `fe0004` | « artefact » (HARDWARE, VERIF) / « pas un artefact » (RE-HID) | **pas un artefact de l'outil** | code du `--dump` de l'époque relu |
| `0xFE` cause des coupures | « peut figer le firmware » (RE-HID) / « état atteint pendant une rafale, pas un rapport tueur » (RECONNEXION) | **non tranché** (compatibles : déclencheur non déterministe) | voir §4 |
| `0x55`, `0x40-0x45` | config vendeur / bloc batterie (COMMANDES) / noms Apple (RE-PILOTE) | **noms Apple** | plist de la personnalité PID 598, et refus `0x03` cohérent |
| `0x04`/`0x05` | SLEEP/FUNC_LOCK « preuve directe » (COMMANDES) / inconnus (RE-PILOTE) | **inconnus** | WICED = analogie |
| MVLT | « macOS lit des mV » (RE-HID) / « réfuté » (RE-PILOTE) | **conversion réfutée, bloc inexpliqué** | désassemblage 2026 vs ioreg 2014 |
| `0x47` = f(`0x49`) | troncature exacte (HARDWARE §4) / mise en défaut (VERIF, #179) | **mise en défaut** | 2945 → 99 attendu, 98 lu ; 2935 → 98 attendu, 96 lu |
| Supervision | 5 s (SDP) / 20 s (LIAISON) | **20 s effectifs** | coupure mesurée ; aucun `Write Link Supervision Timeout` |
| EDR | « 2.0+EDR » (README, MAINTENANCE) / « 2.0 » | **2.0 attesté, EDR non prouvé** | brief sans EDR ; features LMP de référence sans EDR |
| PID de notre clavier | 0x023a (MAINTENANCE) / 0x0256 (partout ailleurs) | **0x0256** | `hid_device.uevent`, SDP |
| fnmode auto | « 4 ou 2 » (MAINTENANCE) / `fnmode=1` (packaging) | **1 pour un clavier Apple** | `hid-apple.c` |

## 4. Inconnues honnêtement non tranchées

1. **Cause des coupures** (#175) : `0xFE` (2 blocages sur ~27 lectures, toujours après `0xF7`) ou état transitoire du
   micrologiciel pendant une rafale. Aucune lecture n'a été faite ici ; le protocole E8 / RE-HID §0.2 reste à exécuter
   avec accord du gérant.
2. Sens de `0x09`, `0x4A` (18), `0x4B` (`00 08`), `0xD1`/`0xD8`, `0xEA`, `0xF4` (1740), `0xF5` (900), `0xF6`/`0xF7` (4),
   octet 3 de `0xFF`, Input `0x04`/`0x05`, Feature refusés `0xD0 0xD4 0xD5 0xFA 0xFB`, 12 derniers octets de `0x4C`.
3. Fonction exacte `0x47` = f(tension) et moment de son recalcul (hypothèse : à la reconnexion).
4. Signature (ou non) de l'image de l'updater Bluetooth Apple du A1314.
5. Résolution de l'ADC du BCM2042 ; plage d'alimentation réelle de la puce (2,7-3,3 V du brief vs 1,7-3,6 V du module).
6. Puce réelle de l'A1314 0x0256 (pas de démontage cité) et des Magic Keyboard.
7. Bank switching du 8051 (P1[3:2]) : datasheet complète inaccessible.
8. Faits macOS (RE-PILOTE-MACOS) : non reproductibles depuis le dépôt.
9. Statistiques sans donnée brute commitée : écart à la grille sniff (0,53 ms), captures btmon brutes supprimées
   (contiennent des frappes) ; seules les agrégations JSON sont conservées.
10. Comportement de veille propre du clavier (délai, LMP detach) : jamais observé, UPower le maintenant éveillé.

## 5. Erreurs de code découvertes (issues ouvertes)

| Issue | Objet |
|---|---|
| [#200](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/200) (bug, sécurité) | CLI Python : #131/#132/#133/#136/#174 fermés mais corrigés seulement dans `akm-core` ; le CLI installé publie 8 octets sensibles de `0x4C`, une tension constante, un « build », des paramètres LE, « 5.0 », des dBm ; tests Python qui figent ces valeurs |
| [#201](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/201) (bug) | CLI Python : 14 rapports vendeur toutes les 300 s sans verrou ni liste blanche (#177) ; `--dump` lit `0xFE` |

Aucun autre défaut de code nouveau : les décodages de `akm-core` (`decode.rs`, `read_policy.rs`, `signal.rs`)
sont conformes aux mesures, et leurs tests sur les trames réelles passent.

## 6. Commits de ce contre-audit

Un commit par document (branche `docs/contre-audit`, non poussée) : README, FEATURES, HARDWARE-RAPPORTS-HID,
RE-HID-EXHAUSTIF, RE-LIAISON-BLUETOOTH, RE-COMMANDES-VENDEUR, RE-FIRMWARE-MAINTENANCE, RE-FIRMWARE-EXTRACTION,
VERIF-BATTERIE, AUDIT-DECODAGE-HID, RECONNEXION-PAIRAGE, RE-PILOTE-MACOS, puis ce fichier.
Wiki Gitea corrigé par l'API : Fonctionnalités, Matériel supporté, Architecture, Installation.
