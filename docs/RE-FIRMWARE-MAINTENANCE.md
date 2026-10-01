# RE — Fonctions cachées, maintenance et diagnostic (BCM2042 / claviers Apple sans fil)

Recherche **documentaire uniquement**, sans accès au matériel. Objet : tout ce qui est
publiquement connu ou déductible des fonctions cachées, de maintenance et de diagnostic
de la puce Broadcom BCM2042 (et parents BCM207xx) et des claviers Apple sans fil, en
particulier le **A1314 ISO, firmware `0x0050` (« HID v0.50 »)**.

Convention de preuve, appliquée à chaque ligne :
- **[source publique]** — attesté par une source citée (datasheet, kernel, brevet, exposé, doc Apple).
- **[déduction]** — tiré logiquement d'une ou plusieurs sources publiques + de nos mesures.
- **[spéculation]** — hypothèse plausible, non prouvée ; à ne jamais présenter comme un fait.

> ⚠️ **Aucune fonction d'écriture (SET_REPORT, flash, bootloader) n'a été testée ni ne doit
> l'être sans sauvegarde préalable.** Voir §6 (risque de brique). Ce document est exploitable
> en **lecture seule** ; les procédures d'écriture n'y figurent qu'à titre d'analyse de risque.

---

## 0. La puce — ce que la datasheet dit vraiment

Source : Broadcom *BCM2042 Single-Chip Bluetooth Mouse and Keyboard — Product Brief*
(réf. `2042-PB03-R`, 11/08/06) et datasheet AllDataSheet/DigChip. **[source publique]**

| Élément | Valeur attestée | Preuve |
|---|---|---|
| Cœur CPU | **8051** 8 bits, « on-board 8051 processor » | [source publique] product brief |
| Mémoire | **108 KB ROM + 22 KB RAM + 20 KB Boot ROM** (bloc fonctionnel) | [source publique] block diagram (revérifié) |
| Conception | **ROM-based**, « eliminates external flash » ; « Flash option offered to support feature development » | [source publique] |
| Config externe | EEPROM série I²C (module BM2042 : « built-in 128K serial EEPROM » = **128 kbit**, 16 Ko ; A1255 : EEPROM/flash STMicro) | [source publique] teardown iFixit + brief module BM2042 |
| Matrice clavier | **jusqu'à 8 × 20 touches**, hot-keys personnalisables | [source publique] |
| Entrées souris | décodeur quadrature 3 axes (ball/optique) | [source publique] |
| Sorties | interface directe **LED et LCD**, GPIO LED/alim | [source publique] |
| Alimentation | LDO intégré + régulateur à découpage (boost) ; le schéma bloc porte « Battery 2.7V to 3.3V ». La fiche du module BM2042 (même puce) donne **VBAT 1,7-3,6 V** (broche) / 1,8-3,6 V (conditions recommandées) et « dual output 1.5-1.8 V or 2.7-3.3 V » : la plage 2,7-3,3 V est vraisemblablement une **sortie** de régulateur, pas la plage des piles [contre-audit] | [source publique] |
| Radio | BT **2.0** (le *brief* ne mentionne **pas** l'EDR ; le module BM2042 dit « 2.0+EDR compatible »), AFH, fast connect, +4 dBm (classe 2), −85 dBm | [source publique] |
| Firmware | de la couche link control jusqu'à HCI exécuté sur le 8051 | [source publique] |

> ⚠️ **Contradiction de nomenclature à trancher.** Le brief ci-dessus et le forum geekhack
> décrivent le BCM2042 comme un **8051**. Le prompt de mission et le README public du projet
> (`litteulapi/apple-kb-monitor`) parlent d'un **ARM7TDMI**. **[déduction]** : l'ARM7TDMI est
> l'architecture des BCM207xx/BCM2070x plus récents (ère Magic Keyboard / BRCM "patchram") ;
> le BCM2042 d'origine (A1016/A1255/A1314 2009) est un **8051**. La famille a migré 8051 → ARM
> au fil des générations. Les fonctions bootloader décrites au §1 (issues de l'exposé Chen 2009)
> concernent la génération **USB Cypress enCoRe**, pas directement le 8051 BCM2042 — voir §1.3.
> **À vérifier** par dump/désassemblage réel ; aucune source ne confirme l'ARM7TDMI pour le A1314.

**Conséquence maintenance** : sur un design *ROM-based*, le firmware applicatif réside en ROM
masquée **non réinscriptible**. Seules la config EEPROM et une éventuelle *patch RAM* sont
modifiables. Ceci borne fortement ce que « mise à jour firmware » peut signifier pour un A1314
(voir §1.4). **[déduction]**

---

## 1. Mise à jour du firmware des claviers Apple sans fil

### 1.1 Chaîne logicielle macOS **[source publique]**

- Les mises à jour héritées sont livrées en paquets `.pkg` : *Aluminum Keyboard Firmware
  Update 1.0 / 1.1*, *2009 Aluminum Keyboard Firmware Update 1.0*, *Apple Wireless Keyboard
  Update 2.0* (2009-11-10). Elles **déposent un updater dans `/Applications/Utilities`** et le
  lancent. Pré-requis : clavier **appairé/connecté**, Mac sur **secteur** (« power cord must be
  connected »), macOS ≥ 10.5.8/10.6.2.
  Sources : support.apple.com (106755, 106678, dl997), macworld.
- **Outil bas niveau** : `HIDFirmwareUpdaterTool` (sans symboles), appelé deux fois :
  `-parse kbd_0x0069_0x0220.irrxfw` puis `-progress -pid 0x220 kbd_0x0069_0x0220.irrxfw`.
  Vérifie la version du clavier (`cmpw $0x0220,%ax`) et l'OS (`SystemVersion.plist`).
  Les images sont nommées `kbd_<ver>_<pid>.irrxfw` dans le bundle de l'updater.
  Source : exposé **K. Chen, « Reversing and Exploiting Apple Firmware Updates », Black Hat USA 2009**.
- **Magic Keyboard moderne** : plus de `.pkg`. Les mises à jour sont **poussées en arrière-plan,
  sans fil**, tant que le clavier est appairé à un appareil macOS/iOS/iPadOS/tvOS (ex. *Magic
  Keyboard Firmware Update 2.0.6*, 2024-01-09). Source : support.apple.com/120303.

### 1.2 Format de fichier `.irrxfw` et transport **[source publique]** (Chen, BH USA 2009)

- **Magic number `0xCAFEBABE`** (≠ Java). VID `0x05ac`, PID ciblés **`0x220`, `0x221`,
  `0x222`, `0x228`** (génération 2007–2009).
- **Obfuscation** (pas chiffrement) : fichier lu par blocs de **83 octets** ; bloc *i* XOR
  complément-à-1 d'un vecteur `A` (83 o), puis chaque octet XOR `B[(i+16) mod 53]` (vecteur
  `B` de 53 o). Dé-obfuscable par l'outil lui-même (ou en dumpant la RAM sous gdb).
- **Pas de signature cryptographique** pour l'image du clavier **filaire** étudié (seules des sommes de contrôle ;
  la citation exacte n'a pas été retrouvée dans le *paper*, à chercher dans les *slides*). Seules des sommes de contrôle protègent l'intégrité (voir 1.3).

### 1.3 Protocole bootloader (génération 2007–2009) **[source publique]** (Chen)

Le clavier **n'a pas d'endpoint interrupt OUT** → tout passe par l'**endpoint de contrôle**,
en HID **Set_Report** :

```
Entrée en bootloader  : bmRequestType=0x21 bRequest=0x09 (SET_REPORT)
                        wValue=0x030a (type Feature 0x03, report ID 0x0a)
                        wIndex=0x0000  wLength=0x0001  data=0x0a
```

Paquets de 64 octets, préfixe `ff` + opcode :

| Commande | Opcode | Rôle |
|---|---|---|
| `ff 38` | enter bootload mode | entrée mode flash (avec mot de passe constant intégré) |
| `ff 39` | write to flash memory | écriture flash |
| `ff 3a` | verify flash memory | vérification |
| `ff 3b` | exit bootloader | sortie |

- **Mot de passe bootloader** : constant, intégré au paquet. **[source publique]**
- **Checksum de paquet** : `53 = (ff+38+01+...+07) mod 0x100` ; checksum final sur toute l'image
  (`0x4e41b mod 0x10000 = 0xe41b`). **[source publique]**
- **Codes retour** : `0x08` flash protection error, `0x10` communication checksum error,
  `0x20` no error, `0x80` invalid command. **[source publique]**

> ⚠️ **Portée réelle.** La *cible désassemblée par Chen est le clavier USB filaire* à cœur
> **Cypress CY7C63923** (8 bits Harvard, 256 o RAM, 8 Ko flash) — cité nommément dans l'exposé.
> Le mécanisme `.irrxfw` + `HIDFirmwareUpdaterTool` est **commun** à la gamme 2007–2009 (PID
> 0x220–0x228), mais les détails *bootloader/flash* ci-dessus valent pour le Cypress, **pas
> prouvés identiques sur le BCM2042 ROM-based**. **[déduction]** Sur BCM2042 *ROM-based*,
> « write to flash » ne peut viser que la zone config/patch, pas la ROM applicative. **À vérifier.**

### 1.4 Ce que cela implique pour notre A1314 (`0x0050`)

- **Notre** A1314 ISO a le PID `0x0256` (ALU_WIRELESS_**2011**_ISO, `hid-ids.h:169`) ; `0x023a` est le A1314 2009 ISO.
  L'appairage BT expose la version `0x0050` (notre `0x4F` décodé). **[source publique + mesuré]** (corrigé au contre-audit).
- **[déduction]** Les updaters `.pkg` 2009–2011 restent le seul canal officiel ; ils exigent
  **macOS ancien + clavier appairé**. Aucun canal Linux/Windows public de flash n'existe.
- README `apple-kb-monitor` (avant contre-audit) : « Signed Apple firmware (not flashable from
  Linux) ». **Statut de signature inconnu pour le A1314** : Chen (2009) a analysé l'updater du clavier **filaire USB**
  (A1243, Cypress CY7C63923), pas celui du clavier Bluetooth (`bfu`, RE-COMMANDES-VENDEUR §4.3). Ni « signé » ni « non
  signé » n'est démontré pour le A1314 ; README corrigé.

### 1.5 Identification du bootloader / vérification de signature — synthèse

| Génération | Bootloader | Signature | Flashable hors macOS ancien |
|---|---|---|---|
| 2007–2009 USB (Cypress) | `ff 38/39/3a/3b`, mdp constant, checksum | **Aucune** [source publique] | Techniquement oui (Chen l'a fait), risque de brique |
| 2009–2011 BT (BCM2042, A1314) | updater `bfu` sur L2CAP (RE-COMMANDES-VENDEUR §4.3) ; ROM-based | **inconnue** (ni démontrée ni réfutée) | **Non** (canal `.pkg` macOS uniquement) [déduction] |
| Magic Keyboard (BCM207xx ARM) | OTA sans fil Apple | **Oui** (chaîne moderne) | Non |

---

## 2. Modes test usine / diagnostic / appairage / alimentation

### 2.1 Reset d'appairage & mode découvrable (A1314) **[source publique]** (forums Apple)

- **Extinction** : maintenir le bouton power ~3 s jusqu'à extinction de la LED verte.
- **Entrée en mode appairage/découvrable** : **maintenir le bouton power** ; la **LED verte
  clignote** → le clavier est découvrable. Garder le bouton enfoncé pendant toute la séquence
  côté hôte (clic *Pair*, saisie du code PIN, Entrée).
- **Reset « batterie »** : retirer les piles ≥ 12 h, réinsérer → démarrage auto en mode appairage.
- **[déduction]** Aucun « reset d'usine » logiciel n'est documenté publiquement pour effacer la clé de lien (BR/EDR ;
  « IRK » est une notion LE) côté clavier ; le désappairage se fait côté hôte (BlueZ `remove`). Le pilote macOS déclare
  toutefois `FactoryDefault` (`0x45`) et `FullFactoryDefault` (`0x44`) en écriture, jamais envoyés (RE-PILOTE-MACOS §3).

### 2.2 « Pairing cable » / appairage par câble

- **A1314** : **pas de port de données**. L'appairage ne peut passer que par la radio BT.
  Il n'y a **pas** de « pairing cable » pour cette génération. **[déduction]**
- **Magic Keyboard** : l'appairage **par Lightning/USB** existe — brancher sur le Mac crée le
  bond ; le Mac **envoie la link key au clavier via USB** (HID). C'est précisément le vecteur de
  **CVE-2024-0230** (Marc Newlin, *Hi, My Name Is Keyboard*, ShmooCon 2024) : la link key reste
  en RAM et est lisible via le port Lightning **ou** via un service HID BT non authentifié ;
  rapport HID **`0x35`** utilisé pour les données d'appairage OOB. **[source publique]**
  → **Ne concerne pas le A1314/BCM2042** (pas de port données, pas de ce service). Noté pour
  complétude sécurité ; aucune action côté notre projet.

### 2.3 Modes d'alimentation / veille **[source publique]** (datasheet) + **[déduction]**

- La datasheet annonce « lowest power consumption », « greater than six-month battery life »,
  fast connect — donc **sleep/deep-sleep automatiques** gérés par le firmware sur inactivité.
  Pas de valeur de délai publiée. **[source publique]** (qualitatif)
- **[déduction]** Le « sniff »/park BT et la coupure radio sur inactivité sont internes au
  firmware 8051 ; non pilotables par l'hôte autrement qu'en gardant le lien actif
  (ce que fait UPower en interrogeant la batterie — cf. nos issues #145/#146).

### 2.4 Rétroéclairage / capteurs

- **A1314** : **aucun rétroéclairage, aucun capteur** (ni ALS ni Touch ID). La datasheet prévoit
  une « interface directe LED/LCD » mais le A1314 ne câble que la LED d'état verte. **[déduction]**
- **Rétroéclairage** : n'existe que sur **Magic Keyboard** récents ; piloté par rapports
  **Output `0xB0`** (set backlight : version, brightness, rate) et **Feature `0xBF`**
  (config : off/on_min/on_max). **[source publique]** kernel `hid-apple.c`.
  → Sans objet pour le A1314 ; ne pas exposer d'option LED rétroéclairage pour ce modèle.

### 2.5 Modes test usine

- **Aucun mode test usine public** pour le A1314/BCM2042. Les puces BT ont un *HCI test mode*
  (DUT mode, `HCI_Enable_Device_Under_Test_Mode`) accessible seulement si le firmware expose HCI
  — ce que le BCM2042 fait en interne mais **n'expose pas** à l'hôte sur un clavier HID appairé.
  **[spéculation]** Un accès HCI direct nécessiterait l'EEPROM/UART physique (geekhack : dump
  I²C de l'EEPROM à la pointe de fer + microscope). Hors périmètre logiciel, non exploitable sans
  démontage. **[source publique]** (geekhack) + **[spéculation]**

---

## 3. Rapports vendeur Apple connus et leur usage OS

### 3.1 Noyau Linux `hid-apple.c` / `hid-input.c` (master, lu le 2026-10-01) **[source publique]**

- **Quirks batterie BT** (`hid_battery_quirks[]`) pour A1314 et parents :
  `ALU_WIRELESS_2009_ANSI/ISO (0x0239/0x023a)`, `ALU_WIRELESS_2011_ANSI/ISO (0x0255/0x0256)`,
  `ALU_WIRELESS_ANSI (0x022c)` → `HID_BATTERY_QUIRK_PERCENT | HID_BATTERY_QUIRK_FEATURE`.
  C.-à-d. : le noyau **lit la batterie en Feature report** et la traite **directement en %**.
- `apple_fetch_battery()` : envoie un **GET_REPORT** sur le report batterie ; **s'abstient si
  `capacity == max`** ; cadencé par `battery_timer` (`APPLE_BATTERY_TIMEOUT_SEC` = **60 s**). **Inactif pour le 0x0256** :
  il exige le quirk `APPLE_RDESC_BATTERY`, que le A1314 n'a pas (`hid-apple.c:1087`). Les lectures toutes les 30 s
  observées viennent d'UPower (#146), pas de ce minuteur [corrigé au contre-audit].
- `hidinput_query_battery_capacity()` lit l'octet à `offset = 1 + report_offset/8` et met à
  l'échelle `min..max → 0..100`. **[source publique]**
- **Magic Keyboard** (différent du A1314) : fixup du **descripteur 83 octets** sous
  `APPLE_RDESC_BATTERY` (ne se déclenche que pour le descripteur USB de 83 o) ; rapports
  d'alim/luminosité `APPLE_MAGIC_REPORT_ID_POWER=3`, `..._BRIGHTNESS=1`. GET_REPORT batterie BT
  **`0x90`** sur Magic Keyboard (confirmé par la série de patchs kernel juillet 2026 :
  l'entrée BT manquait le quirk `APPLE_RDESC_BATTERY`, d'où power_supply bloqué à 0 %).
  **[source publique]**
- Modules : `fnmode` (défaut **3** = auto : 4 si `APPLE_DISABLE_FKEYS`, 2 pour un clavier non Apple, **1 sinon** —
  donc 1 pour le A1314, `hid-apple.c:438-446`), `iso_layout`, `swap_opt_cmd`.
  → cohérent avec nos issues #126/#151 (remappage F-row, fnmode).

### 3.2 macOS (IOKit / AppleBluetoothHIDKeyboard) **[source publique partielle]**

- Le canal de contrôle **HIDP** expose des Feature reports vendeur (`0xFF00`/`0xFF01`) sur le
  BCM2042 ; macOS lit `BatteryPercent`, `ProductID` via `AppleBluetoothHIDKeyboard` /
  `IOHIDFamily`. Le `HIDFirmwareUpdaterTool` lit aussi la version via GET_REPORT. **[source publique]** (Chen)
- Rapports `0x47` (battery strength), `0x90` (Magic KB battery) = ceux cités par la doc et les
  patchs kernel. **[source publique]**

### 3.3 Windows (Boot Camp / AppleBluetoothHID) **[spéculation]**

- Le pilote Boot Camp `AppleKeyboardMagic`/`AppleBTKbd` gère le F-row et probablement la batterie
  via les mêmes Feature reports, mais **aucune source publique détaillée** sur les report IDs.
  **[spéculation]** — à ne pas affirmer.

### 3.4 Table de synthèse des rapports vendeur (recoupée avec nos mesures)

| Report | Sens public | Usage OS | Preuve |
|---|---|---|---|
| `0x47` | Battery Strength (0–255 → %) | Linux quirk FEATURE/PERCENT, = `power_supply/capacity` | [source publique] kernel + [mesuré] nous |
| `0x4F` | version firmware/bcdDevice (`0x0050`) | = `Modalias ...d0050`, « HID v0.50 » | [source publique] kernel + [mesuré] |
| `0x90` | battery input report (Magic KB) | GET_REPORT BT macOS/Linux | [source publique] kernel/patchs |
| `0x35` | données appairage OOB (Magic KB) | vecteur CVE-2024-0230 | [source publique] Newlin |
| `0xB0`/`0xBF` | backlight set / config (Magic KB) | SET/GET_REPORT Linux | [source publique] kernel |
| `0x030a` (SET) | entrée bootloader (gén. 2007–09) | `HIDFirmwareUpdaterTool` | [source publique] Chen |
| `0x09` (`FF01:0B`) | Feature vendeur déclaré | lu par noyau/hôte | [source publique] descripteur + [mesuré] |
| `0x13` (`FF01:0A/0C`) | Fn/éject/réveil (bits) | décodage entrées | [mesuré] nous (cf. #129/#130) |
| `0xEA`, `0xF4`, `0xF5`, `0x5A/0x60/0xEB`, `0x5B`, `0x46`, `0x49`, `0x4C` | **non documentés publiquement** | — | [mesuré] nous uniquement (voir §5) |

---

## 4. Projets de RE existants (BCM2042 / BCM207xx / Apple HID)

| Projet / source | Contenu | Pertinence A1314 |
|---|---|---|
| **K. Chen, Black Hat USA 2009** | dé-obfuscation `.irrxfw`, bootloader `ff 38/39/3a/3b`, pas de signature, cible Cypress CY7C63923 | ⭐⭐⭐ mécanisme de MAJ, mais cœur Cypress ≠ BCM2042 |
| **marcnewlin/hi_my_name_is_keyboard** (CVE-2024-0230) | extraction link key Magic KB via Lightning / BT non authentifié, rapport `0x35` | ⭐ sécurité, **ne vise pas** le A1314 |
| **geekhack « Wireless Model M »** | dump I²C EEPROM FM24C32 (4 Ko) d'un module BCM, 8051, recherche matrice de scan | ⭐⭐ confirme 8051 + EEPROM, voie matérielle |
| **litteulapi/apple-kb-monitor** (notre projet, miroir GitHub) | 21 Feature reports RE, tension/%, courbe de décharge | ⭐⭐⭐ — mais README répète des décodages **réfutés** (voir §5) |
| **sethk/A1243Dvorak** | firmware clavier **USB** A1243 (Dvorak), patch table scancodes + checksum | ⭐ méthode de patch scancode (USB only) |
| **acidanthera/BrcmPatchRAM** | injection *patchram* sur contrôleurs **hôte** Broadcom (BCM20702…) | ⭐ concept patch RAM Broadcom, pas le clavier |
| **NoaHimesaka1873/apple-bcm-firmware** | firmwares BT/Wi-Fi des **Mac** (hôte), pas des claviers | ✗ hors sujet clavier |

**[déduction]** : aucun dump ni désassemblage **public du firmware BCM2042 du A1314** n'existe.
La connaissance fine des reports `0x46/0x49/0x5A/0xEA/0xF4/0xF5/0x4C` est **propre à notre
projet** (mesures), pas sourçable ailleurs — d'où l'importance de la rigueur [mesuré]/[hypothèse].

---

## 5. Vérification de nos affirmations contre les sources

| Affirmation de nos docs / du README | Verdict externe | Preuve |
|---|---|---|
| `0xF5` = `0x0384` = **900 s délai de veille** | **Non confirmable publiquement.** Aucune source ne documente `0xF5`. Reste une **[hypothèse]** à tester en lecture seule (§5 de `HARDWARE-RAPPORTS-HID.md`, script `tests/live/re/idle_timeout.py`). La datasheet confirme seulement *qu'un* mécanisme de veille auto existe, sans valeur. | [source publique] datasheet (qualitatif) + [mesuré] constance de `0xF5` |
| README public : `0xF5` = « tension batterie ADC brut » | **Réfuté par nos mesures** : constant à travers un changement de piles ⇒ ce n'est pas la tension. La vraie tension est `0x46` (LE) / `0xFF` (BE). | [mesuré] (bugs #131/#132/#136/#138) |
| README public : `0xEA` = « % pré-arrondi ADC » | **Non prouvé** ; `0xEA`(98) ≠ `0x47`(99). Au mieux **[hypothèse]** second estimateur. | [mesuré] |
| `0x4F` = version firmware `0x0050` | **Confirmé** : = `bcdDevice d0050`, « HID v0.50 » du noyau. | [source publique] kernel + [mesuré] |
| `0x47` = % batterie = source noyau | **Confirmé** : quirk `PERCENT|FEATURE`, = `power_supply/capacity`. | [source publique] kernel + [mesuré] |
| `0x4C` = adresse de l'hôte appairé (pas une « identity_key ») | **Cohérent** : le lien contient l'adresse de l'hôte ; la vraie clé (link key) est ce que Newlin extrait sur Magic KB, pas exposé ici. | [source publique] Newlin + [mesuré] (#133/#140) |
| « firmware signé, non flashable » | **Non démontré** : Chen ne couvre que le clavier filaire USB ; la signature de l'updater BT du A1314 est inconnue. Non flashable depuis Linux = **vrai en pratique** (pas de canal public). | [source publique] Chen + [déduction] |

**Bilan** : nos docs internes (`HARDWARE-RAPPORTS-HID.md`) sont **plus justes** que le README
public, qui propage encore les décodages erronés `0xF5`/`0xEA`. Le seul point nouveau à corriger
dans notre discours : « firmware **signé** » n'est pas démontré pour le A1314 (dire plutôt « pas de canal
de flash public hors updater macOS ancien, signature inconnue »).

---

## 6. Fonctions nécessitant une ÉCRITURE — risque de brique & retour arrière

> **Aucune de ces opérations ne doit être tentée sans prérequis de sauvegarde explicite.**

| Fonction | Écriture | Risque brique | Retour arrière | Prérequis sauvegarde |
|---|---|---|---|---|
| Entrée bootloader `SET 0x030a`, `ff 38/39/3a` | SET_REPORT / flash | **Élevé** : une image/checksum erroné laisse le clavier en bootloader ⇒ brique si pas de réécriture valide | Seulement re-flasher une image valide via le même outil | Dump EEPROM I²C **avant** (voie matérielle), image `.irrxfw` d'origine |
| Réécriture EEPROM config (I²C) | écriture I²C physique | **Élevé** : matrice/param corrompus ⇒ clavier muet | Restaurer le dump | Dump binaire intégral de l'EEPROM FM24C32/STMicro |
| `SET_REPORT` sur reports vendeur inconnus (`0xF5`…) | SET_REPORT | **Inconnu/dangereux** : effet non caractérisé, peut écrire en NVRAM | Aucun garanti | Ne pas faire : caractériser d'abord en lecture seule |
| Backlight `0xB0` (Magic KB) | Output report | Faible (volatile) | renvoyer une valeur | — (sans objet A1314) |

**[déduction]** Pour le A1314 *ROM-based*, il n'existe **pas de retour arrière logiciel fiable**
si la config EEPROM est corrompue sans dump préalable. ⇒ **politique projet : lecture seule.**

---

## 7. Catalogue — fonctions cachées : preuve, risque, intérêt utilisateur

| Fonction | Niveau de preuve | Risque | Intérêt utilisateur | Exploitable en lecture seule |
|---|---|---|---|---|
| Lire version firmware `0x4F` (`0x0050`) | [source publique]+[mesuré] | nul | diagnostic / inventaire | ✅ (déjà fait) |
| Lire % batterie `0x47` (= noyau) | [source publique]+[mesuré] | nul | batterie fiable | ✅ (déjà fait) |
| Lire tension `0x46`/`0xFF`, % fin via `0x49`/`0x5A` | [mesuré] | nul | santé piles, % précis | ✅ (#139) |
| Lire hôte appairé `0x4C` (adresse) | [mesuré]+[source publique] | nul | alerte ré-appairage | ✅ (#140) |
| Décoder Fn/éject/réveil `0x13` | [mesuré] | nul | exposer entrées | ✅ (#130) |
| **Vérifier `0xF5` = 900 s délai de veille (passif)** | [hypothèse] | **nul (lecture seule)** | prédire/afficher la veille | ✅ **issue à ouvrir** (§8) |
| Reset appairage (bouton power) | [source publique] | nul (manuel) | dépannage liaison | ✅ doc utilisateur (lié #142/#147) |
| Mise à jour firmware `.irrxfw` | [source publique] (gén. 2009) | **élevé** | ~nul (pas de bénéfice) | ❌ écriture, macOS ancien requis |
| Bootloader `ff 38/39` | [source publique] (Cypress) | **très élevé** | nul | ❌ risque brique |
| HCI test/DUT mode | [spéculation] | élevé (matériel) | nul | ❌ non exposé |
| Rétroéclairage `0xB0`/`0xBF` | [source publique] | faible | nul sur A1314 | ❌ sans objet (pas de LED rétro) |

---

## 8. Issues Gitea recommandées (exploitables sans risque, sans doublon)

- **À ouvrir** : « RE — Confirmer `0xF5` = 900 s = délai d'inactivité avant veille (mesure
  passive, lecture seule) » — s'appuie sur `tests/live/re/idle_timeout.py` déjà présent ;
  aucun GET/SET, seulement horodatage des nœuds hidraw. Label `documentation`/`suivi`.
  (Non couvert par #142 qui porte sur *corriger* la reconnexion, ni #145 veille système.)
- **Déjà couvert, ne pas dupliquer** : #139 (tension/% fin), #140 (hôte appairé `0x4C`),
  #130 (entrées `0x13`), #131/#132/#136 (corrections décodage), #142/#145/#147
  (reconnexion/veille/doctor).
- **Note sécurité (pas d'action)** : CVE-2024-0230 ne concerne **que** les Magic Keyboard
  (port Lightning / rapport `0x35`), **pas** le A1314 BCM2042 — à mentionner dans la doc pour
  couper court à toute confusion, sans ouvrir d'issue.

---

## Sources

- Broadcom *BCM2042 Product Brief* `2042-PB03-R` (alldatasheet / digchip).
- K. Chen, *Reversing and Exploiting Apple Firmware Updates*, Black Hat USA 2009 (slides PDF).
- Linux kernel `drivers/hid/hid-apple.c`, `hid-input.c`, `hid-ids.h`, `hid-quirks.c` (master, 2026-10-01).
- Marc Newlin, *Hi, My Name Is Keyboard* (CVE-2024-0230), ShmooCon/Nullcon 2024 ; dépôt GitHub `marcnewlin/hi_my_name_is_keyboard`.
- Apple Support : 106755, 106678, dl997, 120303 ; macworld (keyboard firmware).
- iFixit *Apple Wireless Keyboard (A1255) Teardown* ; geekhack *Wireless Model M* (dump EEPROM BCM).
- README `litteulapi/apple-kb-monitor` (miroir GitHub du projet).
