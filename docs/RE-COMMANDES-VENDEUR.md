# RE — Commandes vendeur, registres en écriture seule et rapports cachés (A1314 ISO / BCM2042)

* Corrigé le 2026-10-02 (issue #193) : §1.1 et §1.4, `0x55` n'est pas un « registre de configuration vendeur » mais `LongDeviceName` (source : `RE-PILOTE-MACOS.md` §3).
* Corrigé le 2026-10-02 (issue #194) : §6 point 4, `MVLT` n'est pas une tension lue par macOS 26.5 pour calculer le % (source : `RE-PILOTE-MACOS.md` §6, `RE-PILOTES-ANCIENS.md` §4).

Recherche **documentaire uniquement**, **sans aucun accès au clavier** (il a décroché trois fois sous
des rafales de lecture — voir #175). Objet : identifier, à partir de sources publiques, ce que sont
les 11 identifiants Feature refusés en lecture, les rapports d'entrée non déclarés `0x04`/`0x05`/`0x30`,
et les commandes de contrôle HID/HCI que les périphériques HID Apple/Broadcom exposent — afin de savoir
ce qui est **connu**, sans jamais spéculer sur le contenu d'une écriture.

Clavier : A1314 ISO, `05AC:0256`, bcdDevice `0x0050`, puce **Broadcom BCM2042**.

Convention de preuve :
- **[source publique]** — attesté par une source citée (datasheet, noyau, brevet, exposé, binaire Apple public).
- **[déduction]** — tiré logiquement d'une ou plusieurs sources publiques + de nos mesures.
- **[spéculation]** — hypothèse plausible, non prouvée ; jamais présentée comme un fait.

> ⚠️ **Politique lecture seule.** Aucune écriture (SET_REPORT, bootloader, flash) n'est testée ni
> recommandée ici. Ce document établit ce que chaque registre est *probablement*, d'où le danger
> viendrait, et comment valider **sans écrire**. Il complète `RE-HID-EXHAUSTIF.md` (§0.3, §6),
> `RE-FIRMWARE-MAINTENANCE.md` et `RE-LIAISON-BLUETOOTH.md`.

---

> **Contre-audit (docs/CONTRE-AUDIT.md)** : les §1.1 et §1.2 ci-dessous sont **remplacés** par la carte
> `ExtendedFeatures` du pilote macOS pour le PID 598 (RE-PILOTE-MACOS §3, niveau [plist]) : `0x40` = WillShutdown,
> `0x41` = RecantConnection, `0x44` = FullFactoryDefault, `0x45` = FactoryDefault, `0x50` = DeviceNameChange,
> `0x55` = **LongDeviceName (64 o)** — pas un « registre de configuration ». Les déductions « famille batterie/identité »
> et « config vendeur » sont donc fausses pour ces six IDs. Seuls `0xD0 0xD4 0xD5 0xFA 0xFB` restent inconnus.
> Au §2.1, le SDK WICED d'Infineon vise des puces récentes (CYW207xx) : c'est une **analogie**, pas une « preuve directe »
> pour le micrologiciel Apple du BCM2042 ; `0x04`/`0x05` restent non confirmés (inconnus de macOS).

## 1. Les 11 IDs Feature refusés en lecture (`ERR_UNSUPPORTED_REQUEST` 0x03)

`0x40 0x41 0x44 0x45 0x50 0x55 0xD0 0xD4 0xD5 0xFA 0xFB` — le micrologiciel les **connaît** (code
`0x03`, distinct de `0x02` « ID inexistant ») mais refuse le GET **[mesuré]** (`RE-HID-EXHAUSTIF.md` §2.1,
#181). Un rapport qui existe mais refuse la **lecture** est, en HID, un rapport **à écriture
prédominante** : SET_REPORT accepté, GET_REPORT non [source publique : Bluetooth HID Profile 1.1.1 §7.4.1 ;
USB HID 1.11 §7.2.1, un rapport peut être déclaré avec accès asymétrique].

### 1.1 `0x55` et `0x50` — famille « registre vendeur de configuration » (preuve externe forte)

> **Corrigé (#193)** : sur l'A1314, `0x55` n'est **pas** un registre de configuration vendeur. C'est **`LongDeviceName`**
> (Feature, 64 o, écriture seule), et `0x50` est `DeviceNameChange` **[plist]** (`RE-PILOTE-MACOS.md` §3). Le descripteur du
> Magic Keyboard cité ci-dessous ne vaut que pour ce produit ; la déduction et la spéculation de cette section sont caduques.

Le **descripteur HID du Magic Keyboard BT** (`05AC:029C`, dump public, xloc, 2024-02-01) déclare un
**rapport Feature vendeur `Report ID 0x55`** :

```
0x06, 0x02, 0xFF,  //   Usage Page (Vendor Defined 0xFF02)
0x09, 0x55,        //   Usage (0x55)
0x85, 0x55,        //   Report ID (85 = 0x55)
...
0xB1, 0xA2,        //   Feature (Data,Var,Abs,...,Volatile)    ← 64 octets, volatile
```

- **[source publique]** Chez Apple, sur une puce Broadcom plus récente, `0x55` est exactement un
  **rapport Feature vendeur de 64 octets, marqué `Volatile`** — c'est-à-dire un registre de
  configuration en écriture (gist `xloc/9f1ecca90ca29a9039c2a2468af70763`).
- **[déduction]** Sur le A1314, `0x55` est très probablement le **même registre de configuration
  vendeur**, exposé mais en écriture seule (GET refusé). `0x50` lui est adjacent (famille `0x5x` =
  identité/nom `0x51-0x54`, cf. `HARDWARE-RAPPORTS-HID.md` §2) : **[spéculation]** commande de
  validation/commit du nom ou second registre de config. On ne spécule pas sur ce qu'une écriture y ferait.

### 1.2 `0x40 0x41 0x44 0x45` — famille batterie/identité

- **[mesuré]** Groupés juste avant le bloc lisible `0x46`(mV) `0x47`(%) `0x49`(mV lissés) `0x4A 0x4B`
  `0x4C`(hôte appairé) `0x4F`(version). Le bloc `0x4x` est le **bloc batterie + identité**.
- **[déduction]** `0x40/0x41/0x44/0x45` sont les **registres d'écriture du même bloc** : calibration
  d'étalonnage batterie, seuils, ou déclencheurs d'acquisition ADC. Aucune source publique ne donne
  leur contenu exact ; **[spéculation]** et donc **non écrits**.

### 1.3 `0xD0 0xD4 0xD5` et `0xFA 0xFB` — familles état-radio et alimentation/config

- **[mesuré]** `0xD0/0xD4/0xD5` encadrent `0xD1 0xD8` (lus, valeur 0, sens inconnu) ; `0xFA/0xFB`
  encadrent le bloc `0xF4-0xFF` (tension mini `0xF4`=1740 mV, `0xF5`=900, `0xF6/0xF7`, `0xFE` dangereux).
- **[déduction]** Ce sont les **écritures des blocs « état radio » (0xDx) et « alimentation/veille »
  (0xFx)**. Le bloc `0xFx` étant celui où `0xF5`=900 est l'hypothèse « délai de veille » (#173), `0xFA`
  ou `0xFB` pourraient être la **porte d'écriture du délai de veille** — **[spéculation]**, à ne pas écrire.

### 1.4 Verdict §1

Les 11 IDs sont, de façon cohérente avec le comportement Apple/Broadcom public, des **registres de
configuration/commande en écriture** regroupés par famille autour des registres lisibles correspondants.
**Corrigé (#193)** : six des onze sont nommés par la personnalité Apple du PID 598 **[plist]** (`RE-PILOTE-MACOS.md` §3) :
`0x40` `WillShutdown`, `0x41` `RecantConnection`, `0x44` `FullFactoryDefault`, `0x45` `FactoryDefault`, `0x50`
`DeviceNameChange` et `0x55` **`LongDeviceName`** (64 o), qui n'est donc pas un « registre Feature vendeur de configuration ».
Seuls `0xD0 0xD4 0xD5 0xFA 0xFB` n'ont aucun nom dans les personnalités Apple. **Aucun ne doit être écrit** tant
que #175 n'est pas résolu (voir table §5).

---

## 2. Les rapports d'entrée non déclarés `0x04`, `0x05`, `0x30`

Ces trois IDs répondent au GET Input alors qu'ils **ne sont pas dans le descripteur** **[mesuré]**
(`RE-HID-EXHAUSTIF.md` §2.2 : `0x04`→`04 00`, `0x05`→`05 02`, `0x30`→`30 00`). Correspondance publique
trouvée :

### 2.1 `0x04` = SLEEP, `0x05` = FUNC_LOCK ? (firmware HID de référence Broadcom/Infineon — analogie, non confirmée)

Le **BTSDK « HID dual-mode keyboard »** d'Infineon (ex-Broadcom/Cypress WICED, le SDK officiel des puces
BT HID Broadcom successeurs du BCM2042) définit publiquement ses **report IDs d'entrée** ainsi
(`app.h`, dépôt `Infineon/mtb-example-btsdk-hid-dual-mode-keyboard`) :

```c
RPT_ID_IN_STD_KEY    = 0x01,   // = notre clavier de démarrage 0x01
RPT_ID_IN_BIT_MAPPED = 0x02,
RPT_ID_IN_BATTERY    = 0x03,
RPT_ID_IN_SLEEP      = 0x04,   // <-- notre 0x04
RPT_ID_IN_FUNC_LOCK  = 0x05,   // <-- notre 0x05
RPT_ID_IN_SCROLL     = 0x06,
RPT_ID_IN_PIN        = 0x07,
RPT_ID_IN_CNT_CTL    = 0xcc,   // Connection Control (feature aussi)
```

- **[source publique]** Dans le firmware HID de référence de la famille Broadcom, **`0x04` est le
  rapport SLEEP** (notification de mise en veille) et **`0x05` le rapport FUNC_LOCK** (état du verrou Fn).
- **[spéculation]** (rétrogradé au contre-audit : aucune source Apple ; macOS ignore ces IDs) Nos valeurs collent : `0x05`=`05 02` = octet d'état du verrou de fonction
  (`02` = un état de bascule), `0x04`=`04 00` = pas d'évènement de veille en cours. Ce sont des
  **rapports d'entrée internes** que le A1314 **n'émet pas en interruption** (pas de touche Fn-lock
  physique sur ce modèle), mais que le cœur Broadcom expose quand même au GET.
- **Conséquence projet** : le démon peut les surveiller **passivement** (lecture du nœud `/dev/hidraw`,
  jamais de GET actif) pour détecter veille/Fn — **sans aucun risque**, c'est de la lecture.

### 2.2 `0x30` = BATT_STAT (état batterie, convention souris/trackpad Apple)

- **[source publique]** FreeBSD `bthidd` déclare pour les périphériques HID Apple :
  `#define BATT_STAT_REPORT_ID 0x30` et `#define BATT_STRENGTH_REPORT_ID 0x47`
  (`usr.sbin/bluetooth/bthidd/hid.c`, « *Inoffical and unannounced report ids for Apple Mice and
  trackpad* »). Le même `0x47` est notre registre de pourcentage batterie **[mesuré + source]**.
- **[déduction]** `0x30` est le **rapport d'état batterie** (OK / bas / critique) de la convention
  HID Apple, pendant du `0x47` (strength). Notre `30 00` = état « OK / rien à signaler ». À surveiller
  passivement comme `0x04/0x05` ; **ne jamais le poller activement** (c'est une entrée, pas un registre
  à GET en rafale).

### 2.3 Ce qui N'est PAS applicable au A1314

- **`0x90`** (Input, Power Page) = batterie des **Magic Keyboard / Magic Mouse 2 / Trackpad 2** lue par
  GET_REPORT sur BT (noyau `hid-magicmouse.c`, série de patchs juillet 2026). **Absent** du A1314 :
  notre batterie est en Feature `0x47`. [source publique + mesuré : `0x90` refusé `0x02` chez nous]
- **`0x35`** (Feature) = **données d'appairage OOB + clé de lien** des Magic Keyboard (CVE-2024-0230,
  Newlin) — voir §3.3. **Absent** du A1314.
- **`0x34`** (Feature) = adresse BT + nom produit du Magic Keyboard (Newlin). Chez nous, l'adresse de
  l'hôte est dans `0x4C` et le nom dans `0x51-0x54` [mesuré] ; `0x34` est refusé `0x02`.

---

## 3. Commandes de contrôle HID/HCI des périphériques HID Apple/Broadcom

### 3.1 « Magic report » d'activation (souris/trackpad) — SET_FEATURE, inoffensif par analogie, hors sujet clavier

- **[source publique]** Pour réveiller le mode multitouch, le pilote envoie un **SET_REPORT Feature** :
  Linux `hid-magicmouse.c` → `feature_mt[] = { 0xD7, 0x01 }` (Magic Mouse 1), `{ 0xF1, 0x02, 0x01 }`
  (Mouse 2 / Trackpad 2) ; FreeBSD `bthidd` → `{ 0x53, 0xD7, 0x01 }` écrit sur le canal de contrôle
  L2CAP (`0x53` = en-tête HIDP SET_REPORT Feature, puis report `0xD7` donnée `0x01`).
- **[déduction]** C'est la **forme générale d'une commande vendeur Apple** : `SET_FEATURE <id> <data>`
  sur le canal de contrôle (PSM 0x11), exactement le canal que nous observons (`RE-LIAISON-BLUETOOTH.md`
  §2). Un clavier n'a **pas** de mode multitouch ; `0xD7` est refusé `0x02` chez nous. Noté comme
  **modèle de preuve** : nos 11 registres `0x03` se piloteraient de la même manière (`0x53 <id> <data>`).

### 3.2 Reset / mode découvrable / « reset usine » du A1314

- **[source publique]** Reset d'appairage et mode découvrable = **manipulations physiques** du bouton
  power (maintien jusqu'au clignotement vert), ou retrait des piles (forums Apple ; `RE-FIRMWARE-MAINTENANCE.md`
  §2.1). Pas de « reset usine » **logiciel** documenté.
- **[déduction]** Il n'existe **pas** de commande HID publique de reset/factory sur le A1314 ; le
  désappairage se fait côté hôte (BlueZ `remove`). Les registres `0xD0/0xD4/0xD5` ou `0x40/0x41`
  pourraient en théorie en porter une [spéculation], mais rien ne l'atteste — **ne pas écrire**.

### 3.3 CVE-2024-0230 (`0x35`) — Magic Keyboard uniquement, PAS le A1314

- **[source publique]** Newlin (`hi_my_name_is_keyboard`, ShmooCon 2024). Séquence de lecture BT :
  `c17.send([0x53, 0xFF, id])` puis `c17.send([0x43, 0xF0|flag])` sur L2CAP 17 (contrôle), pour lire
  `0x34` (adresse+nom clavier) et `0x35` (adresse du Mac + **clé de lien 16 o**, format
  `35 01 01 <6o adresse Mac> <16o clé>`).
- **Pourquoi hors sujet** : l'attaque exploite que le **Magic Keyboard a un port Lightning** et que le
  Mac lui **pousse la clé de lien par USB** (SET_REPORT `0x35`), clé qui reste lisible en RAM par BT
  non authentifié. Le **A1314 n'a aucun port de données** : pas de handoff USB↔BT, pas de `0x34/0x35`
  (refusés `0x02` chez nous). **[déduction]** CVE-2024-0230 **ne concerne pas** le A1314. Mentionné
  pour couper court à la confusion, aucune action.
- ⚠️ À noter tout de même : notre `0x4C` **contient l'adresse de l'hôte appairé** [mesuré, #133/#140].
  La *vraie* clé de lien n'y est pas exposée en clair comme sur `0x35` (c'est ce que Newlin extrayait),
  mais `0x4C` reste sensible : **ne jamais logguer ses 12 octets** (`RE-HID-EXHAUSTIF.md` §0.3).

### 3.4 Rétro-éclairage (`0xB0`/`0xBF`) et batterie `0x90` — Magic Keyboard, sans objet A1314

- **[source publique]** Noyau `hid-apple.c` : `0xB0` = SET backlight (`apple_backlight_set_report`,
  `hid_hw_raw_request(hdev, 0xB0u, ...)`), `0xBF` = GET/SET config backlight
  (`apple_backlight_config_report`). Activés **seulement** par le quirk `APPLE_BACKLIGHT_CTL`, attaché
  aux **MacBook T2 (WELLSPRINGT2\_\*)**, et `APPLE_MAGIC_BACKLIGHT` au **Touch Bar**. Le 0x0256 (notre
  modèle) ne reçoit que `APPLE_NUMLOCK_EMULATION | APPLE_HAS_FN | APPLE_ISO_TILDE_QUIRK`.
- **[déduction]** Le A1314 **n'a pas de rétro-éclairage** (une seule LED d'état verte). `0xB0/0xBF`
  sont refusés `0x02` chez nous. Aucune option LED à exposer pour ce modèle.

### 3.5 HCI test / DUT / mode usine

- **[source publique + spéculation]** `RE-FIRMWARE-MAINTENANCE.md` §2.5 : le BCM2042 a un HCI interne
  (jusqu'au DUT mode) mais **ne l'expose pas** à l'hôte sur un clavier HID appairé. Les opcodes vendeur
  Broadcom HCI (`0xFC01` set bdaddr, `0xFC2E` download minidriver, `0xFC4C` write RAM, `0xFC4E` launch
  RAM — `btbcm.c`, `hciattach_bcm43xx.c`) s'adressent au **contrôleur hôte**, pas à un périphérique HID
  distant. **[déduction]** Inaccessibles sans UART/EEPROM physique (démontage). Hors périmètre logiciel.

---

## 4. Le bootloader HID de Chen (`ff 38/39/3a/3b`) — s'applique-t-il au A1314 ?

### 4.1 Ce que Chen a réellement désassemblé (Black Hat USA 2009)

- **[source publique]** (paper + slides, blackhat.com/presentations/bh-usa-09/CHEN) : la cible est le
  **clavier filaire USB A1243/A1242** (`05AC:0220`), à cœur **Cypress CY7C63923 (enCoRe II, 8 bits
  Harvard, 256 o RAM, 8 Ko flash)** + EEPROM Microchip 25LC040A + hub Cypress. **Pas** un BCM2042.
- Entrée bootloader : `bmRequestType=0x21 bRequest=0x09` (SET_REPORT), `wValue=0x030A` (Feature,
  report ID `0x0A`), `data=0x0A`. Paquets de 64 o, préfixe `ff` + opcode :
  `ff 38` enter bootload, `ff 39` write flash, `ff 3a` verify, `ff 3b` exit.
- Le premier paquet est `ff 38 00 01 02 03 04 05 06 07 00 … 53 …`. Le « mot de passe constant » est en
  réalité la **suite d'octets `00 01 02 03 04 05 06 07`** passée en paramètre de `ff 38`, et `0x53` est
  la **somme de contrôle** du paquet (`0xff+0x38+0x01+…+0x07 = 0x153 mod 0x100 = 0x53`). Codes retour :
  `0x00` pas de réponse, `0x08` protection flash, `0x10` checksum, `0x20` OK, `0x80` commande invalide.
- **[source publique]** Ce schéma `0x38/0x39/0x3B` **est le bootloader USBFS générique de Cypress/PSoC**
  (`ENTER=0x38`, `WRITE=0x39`, `EXIT=0x3B` ; gist `kylemanna/4047794`, famille PSoC1). C'est une
  **convention Cypress**, pas Broadcom.

### 4.2 Application au A1314 : NON

- **[déduction]** Le A1314 est **BCM2042, ROM-based** (datasheet : 108 Ko ROM masquée + 20 Ko Boot ROM,
  « eliminates external flash »). Le bootloader Cypress `ff 38/39/3a/3b` et son « mot de passe »
  **ne s'appliquent pas** : architecture, jeu d'opcodes et transport différents.
- **[mesuré]** Cohérent : `0x0A` est refusé `0x02` (ID inexistant) chez nous — le A1314 n'a **pas** le
  report bootloader `0x0A` de Chen. Les opcodes `0x38/0x39/0x3B` ne correspondent à aucun de nos IDs.
- **Conclusion** : la menace Chen **ne transpose pas** au A1314. Le canal de mise à jour du A1314 est
  différent — voir §4.3.

### 4.3 Le vrai canal de mise à jour du A1314 : l'updater Bluetooth 2009 d'Apple (constat public)

Le paquet officiel **`WirelessKybdFirmwareUpdate.dmg`** (Apple, « 2009 Aluminum Keyboard Firmware
Update », toujours téléchargeable depuis `updates.cdn-apple.com`) contient l'updater bas niveau
`.../WLKBFU/2/bfu` (Mach-O i386+ppc). Examen **statique non destructif** (métadonnées publiques :
chaînes, méthodes Objective-C, `Parameters.plist`) :

- **[source publique]** `Parameters.plist` : **`FWVersion = 80`** (= **0x50**), `PageTimeout = 32768`.
  → la cible de cet updater est exactement notre **firmware `0x0050`**. C'est **le** canal officiel du A1314.
- **[source publique]** L'updater lie `IOBluetooth.framework` et ouvre un **canal L2CAP Bluetooth**
  (`IOBluetoothDeviceOpenL2CAPChannelSync`, `IOBluetoothL2CAPChannelWrite`), pilote le driver
  `IOAppleBluetoothHIDDriver` / `IOBluetoothHIDDriver`, et expose des méthodes
  `sendCommand:withAck:param:pLength:`, `sendRecord`, `calculateFirmwareChecksum`, `prepareDevice`,
  `ReleaseAllChannelsWithSleepForHIDUpdate`, `FWDecrypt`/`decryptFWData:`, avec un `config.hex`
  chiffré/obfusqué joint.
- **[déduction]** La mise à jour du A1314 passe donc par **le canal de contrôle HIDP Bluetooth**
  (celui-là même que nous observons), par une **séquence de commandes vendeur avec accusé** — et non
  par le bootloader USB Cypress. Les **registres en écriture seule du §1** (familles `0x4x`, `0x5x`,
  `0xDx`, `0xFx`) sont les **candidats naturels** pour ces commandes de maintenance (enter-update,
  write-record, checksum, reset). **[spéculation]** quant au mapping exact ; **aucune écriture n'est
  tentée**. Ce point corrige `RE-FIRMWARE-MAINTENANCE.md` §1.4 : il **existe** un canal de flash hors
  « bootloader Cypress », mais il reste **réservé à l'updater macOS** et dangereux.
- **[déduction]** Le `.irrxfw` et `HIDFirmwareUpdaterTool` de Chen (gén. USB 0x220) **ne sont pas** ce
  que le A1314 utilise : ici c'est `bfu` + `config.hex` + L2CAP. Deux générations, deux mécanismes.

> ⚠️ Rien de ce §4.3 n'est une recette d'écriture : c'est un constat sur un binaire public, destiné à
> expliquer **pourquoi** certains registres refusent la lecture (ce sont des portes de commande/maintenance)
> et **pourquoi il ne faut pas y écrire à l'aveugle** (perte de l'appairage, voire brique — pas de
> retour arrière sans dump préalable, cf. `RE-FIRMWARE-MAINTENANCE.md` §6).

---

## 5. Table de synthèse — registre → fonction probable → preuve → risque d'écriture → test sûr

| Registre | Type | Fonction probable | Preuve | Risque d'écriture | Test SÛR (sans écrire) |
|---|---|---|---|---|---|
| **`0x55`** | Feature (WO) | **LongDeviceName** (64 o) | [plist] RE-PILOTE-MACOS §3 (remplace la déduction « config vendeur ») | moyen (NVRAM) | aucune écriture |
| **`0x50`** | Feature (WO) | **DeviceNameChange** (validation du nom) | [plist] RE-PILOTE-MACOS §3 | **moyen** | aucune écriture |
| **`0x40 0x41 0x44 0x45`** | Feature (WO) | **WillShutdown, RecantConnection, FullFactoryDefault, FactoryDefault** | [plist] RE-PILOTE-MACOS §3 (remplace la déduction « bloc batterie ») | `0x40` faible (macOS l'envoie) ; `0x41` élevé ; `0x44`/`0x45` **INTERDIT** (remise à zéro) | aucune écriture |
| **`0xD0 0xD4 0xD5`** | Feature (WO) | écriture bloc « état radio » | [déduction] (famille `0xDx`) | **inconnu** | lire `0xD1/0xD8` (passif) |
| **`0xFA 0xFB`** | Feature (WO) | écriture bloc alim/veille (p.ex. délai `0xF5`) | [déduction] (famille `0xFx`) + [spéculation] | **élevé** (alim/radio) | mesure passive du délai de veille (#173) |
| `0x04` | Input non décl. | SLEEP ? | [analogie] WICED `RPT_ID_IN_SLEEP=0x04` ; inconnu de macOS | n/a (entrée) | **lecture passive** du nœud hidraw |
| `0x05` | Input non décl. | FUNC_LOCK ? | [analogie] WICED `RPT_ID_IN_FUNC_LOCK=0x05` + [mesuré] `05 02` ; inconnu de macOS | n/a (entrée) | lecture passive |
| `0x30` | Input non décl. | **BatteryState** (0 normal, 1 bas, 2-3 critique) | [source] bthidd `BATT_STAT_REPORT_ID=0x30` + [plist/désassemblage] RE-PILOTE-MACOS | n/a (entrée) | lecture passive |
| `0x47` | Input→Feature | battery strength (%) | [source] noyau quirk + [mesuré] | — (déjà lu) | déjà en prod |
| `0x35` | Feature | **N/A A1314** (clé de lien Magic KB, CVE-2024-0230) | [source] Newlin | — | ne pas chercher (refusé `0x02`) |
| `0x90` | Input | **N/A A1314** (batterie Magic KB/Mouse2) | [source] hid-magicmouse | — | — |
| `0xB0`/`0xBF` | Output/Feature | **N/A A1314** (rétro-éclairage Magic KB) | [source] hid-apple | — | — |
| `0x0A` + `ff 38/39/3a/3b` | — | **N/A A1314** (bootloader Cypress USB) | [source] Chen 2009 | — | ne pas tenter (refusé `0x02`) |
| `0xFE` | Feature (RO) | boîte réponse/journal, **fige le firmware** | [mesuré] 2 coupures sur ~27 lectures (#175) | — | **NE JAMAIS LIRE** (déjà exclu) |
| `0x4C` | Feature (RO) | adresse hôte appairé + 12 o sensibles | [mesuré] + [source] | — | lire ≤ 1×/connexion, **ne pas logguer les 12 o** |

### 5.1 À NE JAMAIS ÉCRIRE (ligne rouge)

- **Les 11 registres `0x03`** (`0x40 0x41 0x44 0x45 0x50 0x55 0xD0 0xD4 0xD5 0xFA 0xFB`) : commandes/
  config vendeur de sens non prouvé. Une écriture peut toucher l'appairage, l'alimentation, la radio,
  ou déclencher une séquence de maintenance (§4.3). **Pas de retour arrière logiciel garanti** sur un
  BCM2042 ROM-based sans dump préalable (impossible sans démontage). → `RE-FIRMWARE-MAINTENANCE.md` §6,
  plan #182 palier **W7 « déconseillé »**.
- **`0x4C`** : écriture = perte probable de l'appairage.
- **Tout ce qui ressemble au canal de maintenance `bfu`/L2CAP** (§4.3) : réservé à l'updater macOS.

### 5.2 Exploitable SANS RISQUE, tout de suite (lecture passive seule)

- Surveiller `0x04` (veille), `0x05` (Fn-lock), `0x30` (état batterie) **en lecture passive** du nœud
  `/dev/hidraw` (rapports d'interruption), **jamais par GET actif**. Zéro trafic radio provoqué.
- Confirmer `0xF5 = 900 s` par mesure **passive** du délai de veille (#173, déjà ouverte).

---

## 6. Méthode d'analyse sûre pour valider chaque hypothèse SANS écrire

1. **Dumps de descripteurs croisés** [sans risque] : comparer notre descripteur 224 o aux descripteurs
   publics (Magic KB `029C`, Magic Mouse `030D`) pour confirmer le rôle de `0x55`, `0x90`, `0x30`.
   Fait ici pour `0x55` (§1.1), `0x04/0x05` (§2.1), `0x30` (§2.2).
2. **Lecture passive du nœud hidraw** [sans risque] : écouter les rapports d'interruption `0x04/0x05/0x30`
   sans aucun GET. Valide leur sémantique (veille/Fn/batterie) par corrélation temporelle.
3. **Captures macOS publiques** [sans risque] : une capture **PacketLogger** (`.pklg`) ou Wireshark d'un
   Mac pilotant un A1314, ou un **btmon** de `apple_fetch_battery`, montrerait quels SET_REPORT/GET_REPORT
   macOS/IOKit émet réellement — y compris vers les registres `0x55`/`0x5x`. **Aucune publique trouvée**
   pour le A1314 à ce jour ; à solliciter plutôt que de tester en écriture.
4. **Journaux IOKit / ioreg** [sans risque] : `AppleBluetoothHIDKeyboard` expose `BatteryPercent`,
   `BatteryLow`, `BatteryPanic` et un blob `"Battery" = <"MVLT…` (managingosx, 2014). **Corrigé (#194)** :
   `MVLT` se lit `MV{LT}` = `MeasuredVoltages.Latched`, la tension de `0x49` (et non `0x46`) que publiait le pilote de Lion
   10.7.5 (`RE-PILOTES-ANCIENS.md` §4). macOS 26.5 ne lit plus aucune tension, et aucune version n'a tiré le % d'une tension :
   le pilote recopie `0x47`, borné à 100 (`RE-PILOTE-MACOS.md` §6).
5. **Analyse statique de binaires publics** [sans risque, non destructif] : comme au §4.3 sur `bfu`
   (chaînes, méthodes ObjC, plist) — jamais l'exécuter contre le clavier.
6. **Interdits** : tout SET_REPORT, tout GET `0xFE`, tout balayage actif, toute exécution de `bfu`/
   `HIDFirmwareUpdaterTool` contre le clavier. (`RE-HID-EXHAUSTIF.md` §0.3, #175, #177.)

---

## 7. Corrections apportées aux docs existantes

- `RE-FIRMWARE-MAINTENANCE.md` §1.4 disait « aucun canal Linux/Windows public de flash n'existe » et
  « bootloader non prouvé public » : **précision** — il existe un **canal de flash Bluetooth officiel**
  (`WirelessKybdFirmwareUpdate.dmg` → `bfu` → L2CAP/HIDP, `FWVersion=80=0x50`), mais il reste **réservé
  à l'updater macOS** et **non transposable** au bootloader Cypress de Chen. (§4.3)
- La mention « firmware signé » reste à éviter pour le A1314 (Chen : pas de signature gén. 2009 ;
  `config.hex` ici est **obfusqué/chiffré** via `FWDecrypt`, pas nécessairement signé — **[spéculation]**,
  non vérifié sans exécuter la routine, ce qui n'est pas fait).

---

## Sources

- Linux kernel `drivers/hid/{hid-apple.c,hid-magicmouse.c,hid-input.c,hid-ids.h}` et `drivers/bluetooth/btbcm.c` (master, lu 2026-10-01).
- FreeBSD `usr.sbin/bluetooth/bthidd/hid.c` (report IDs Apple `0x29/0x30/0x47`, magic report `53 d7 01`).
- Infineon/Broadcom WICED BTSDK : `Infineon/mtb-example-btsdk-hid-dual-mode-keyboard`, `app.h` (RPT_ID_IN_SLEEP=0x04, FUNC_LOCK=0x05, CNT_CTL=0xcc).
- Magic Keyboard BT HID descriptor : gist `xloc/9f1ecca90ca29a9039c2a2468af70763` (Feature `0xFF02:0x55` 64 o volatile ; Input `0x90` Power).
- K. Chen, *Reversing and Exploiting an Apple Firmware Update*, Black Hat USA 2009 (paper + slides) — bootloader Cypress `ff 38/39/3a/3b`, `wValue 0x030A`, checksum, cible CY7C63923.
- Cypress/PSoC USBFS bootloader : gist `kylemanna/4047794` (ENTER 0x38 / WRITE 0x39 / EXIT 0x3B).
- Marc Newlin, *Hi, My Name Is Keyboard* (CVE-2024-0230), ShmooCon 2024 ; `marcnewlin/hi_my_name_is_keyboard` (`0x34`/`0x35`, séquence `53 FF id` / `43 F0`).
- Apple « 2009 Aluminum Keyboard Firmware Update » — `support.apple.com/en-us/106755`, DMG `WirelessKybdFirmwareUpdate.dmg` (métadonnées publiques de `bfu` + `Parameters.plist`, `FWVersion=80`).
- Broadcom *BCM2042 Product Brief* `2042-PB03-R` ; iFixit Magic Mouse teardown (BCM2042A4KFBGH).
- managingosx.wordpress.com (2014) — `AppleBluetoothHIDKeyboard` `BatteryPercent`/`BatteryLow`/`BatteryPanic`, blob `MVLT`.
- Mesures internes : `RE-HID-EXHAUSTIF.md`, `RE-LIAISON-BLUETOOTH.md`, `HARDWARE-RAPPORTS-HID.md`, issues #133/#140/#175/#177/#181/#182.
