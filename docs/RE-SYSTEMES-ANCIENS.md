# RE — Systèmes Apple publiés de 10.2.8 à 10.6.8 : qui appelle quels registres du A1314

Clavier visé : Apple Wireless Keyboard A1314 « 2009 B », BCM2042, PID `0x0256`, firmware `0x0050`.
Complète `RE-PILOTES-ANCIENS.md` (2009, 10.7.5) et `RE-MACOS-SILICON.md` / `RE-PILOTE-MACOS.md` (macOS 26.5).
Question du gérant : « il y a forcément des appels aux registres dans le système Apple publié ». Ce document
répond registre par registre, avec la version d'OS où chaque appel apparaît.

**Cadre.** Analyse statique de logiciels Apple publiquement distribués, pour l'interopérabilité. Aucun accès au clavier,
aucune écriture HID. Aucun binaire Apple ni code décompilé n'est commité : seulement des identifiants, tailles, formats
et noms de symboles. Aucune clé, aucun déchiffrement de firmware.

**Niveaux de preuve** : **[plist]** lu dans un `Info.plist` ; **[décompilé]** fonction décompilée par Ghidra 12.1.2 ;
**[chaîne]** symbole ou chaîne présents dans le binaire ; **[mesuré]** mesure sur notre clavier (documents existants) ;
**[déduction]** raisonnement, à confirmer.

## 0. Réponse courte

1. **Il y a bien des appels, mais par nom** : de Jaguar (10.2.8, 2003) à Snow Leopard (10.6.8, 2011), le pilote noyau
   (`IOAppleBluetoothHIDDriver`) n'émet **aucun** ID de rapport codé en dur. Il lit un dictionnaire `ExtendedFeatures`
   dans le `Info.plist` (`nom → id, type, taille, bornes`) et appelle `getExtendedReport("BatteryPercent")`,
   `setExtendedReport("WillShutdown", …)`, etc. **[décompilé]**. La liste des registres qu'Apple appelle est donc
   exactement l'union de ces dictionnaires, plus les ID codés en dur dans le framework utilisateur. Les deux ont été
   relevés dans les **huit** paquets examinés.
2. **Un seul ID codé en dur dans tout l'espace utilisateur** : `0x4A`, écrit par
   `-[AppleBluetoothHIDDevice setFeatureWithReportID:0x4A value:n]`, avec `n` = 1 à 4. Il apparaît avec **Bluetooth 1.5**
   (février 2004, support des casques) et reste inchangé jusqu'à Lion **[décompilé 10.4.11 PPC et 10.5.8 i386, chaîne 1.5 et 10.3.9]**.
3. **`0xD0 0xD4 0xD5 0xFA 0xFB 0x4B 0xD1 0xD8 0xF6 0xF7` et les Input `0x04` `0x05` n'apparaissent dans aucun** des
   systèmes, pilotes, frameworks, démons, panneaux de préférences et updaters examinés de 10.2.8 à 10.6.8. Toutes les
   occurrences de ces constantes trouvées par Ghidra sont des décalages de structure, des octets d'UUID ou des délais
   (détail §4). La réponse logicielle publique est donc **négative et maintenant exhaustive de 2003 à 2012**.
4. Les octets `0xD1-0xDC` existent bien chez Apple, mais comme **opcodes du protocole de mise à jour** sur un canal
   L2CAP vendeur (PSM `0xF30D`), pas comme ID de rapport HID (§5). Ce protocole est le même en 2007 et en 2009.

## 1. Paquets examinés (serveurs Apple publics)

| Paquet | Source officielle | Taille | Intégrité | Bluetooth / HID trouvés |
|---|---|---|---|---|
| Mac OS X 10.2.8 Combo (`061-0807`) | `https://download.info.apple.com/Mac_OS_X/061-0807.20031003.AbT8b/2Z/MacOSXUpdateCombo10.2.8.dmg` | 101 800 834 o | sha256 `fe1b0e1c31aab370…` | IOBluetoothFamily 1.3.2f2, IOBluetoothHIDDriver 1.1f6 (plist seul, binaire absent du combo), IOBluetooth.framework |
| Bluetooth Update 1.5 (`061-0975`, support.apple.com/106464) | `https://download.info.apple.com/Mac_OS_X/061-0975.20040205.tb453/2Z/BluetoothUpdate1.5.dmg` | 5 410 157 o | sha256 `56f59bd674350dfd…` | IOBluetoothFamily 1.5f12, IOBluetooth.framework |
| Mac OS X 10.3.9 Combo (`061-1570`) | `https://download.info.apple.com/Mac_OS_X/061-1570.20050415.39CbC/MacOSXUpdateCombo10.3.9.dmg` | 118 917 398 o | sha256 `6fe0fe18cb0e44227b15fc2f60c82176091e46607a1409bbcae61e76f157e581` | IOBluetoothFamily 1.5.4f6, IOBluetoothHIDDriver 1.1.2f1, framework, `blued` |
| Mac OS X 10.4.11 Combo PPC (`061-3465`) | `http://swcdn.apple.com/content/downloads/43/02/zzz061-3465/fTqCvXWszd59dvPcX54s22WLy8bNg72MZr/MacOSXUpdCombo10.4.11PPC.tar` | 195 368 960 o | taille = catalogue ; tar → pkg → `Archive.pax.gz` lu en flux | IOBluetoothFamily 1.9.5f4, IOBluetoothHIDDriver 1.7.4f1, AppleHIDKeyboard 1.0.2f2 (ppc) |
| Mac OS X 10.4.11 Combo Intel (`061-4049`) | `http://swcdn.apple.com/content/downloads/19/05/zzz061-4049/W3bK3CTB9kRSwr5HWcWWbKDMQq9DPhyvkY/MacOSXUpdCombo10.4.11Intel.tar` | 346 746 880 o | idem | mêmes versions (i386) |
| Mac OS X 10.5.8 Combo (`041-98113`) | `http://swcdn.apple.com/content/downloads/33/32/041-98113-A_TI33BOXTMO/dqoewhlu9sd8s38j8who477umcenckft3q/MacOSXUpdCombo10.5.8.pkg` | 805 319 908 o | TOC xar SHA-1 OK, lu en flux | IOBluetoothFamily / IOBluetoothHIDDriver 2.1.8f2 (ppc + i386), AppleHIDKeyboard 1.0.9b4, framework, `blued`, prefPane |
| Mac OS X 10.6.8 Combo v1.1 (`041-98121`) : `MacOSXClientCombo10.6.8.pkg` **et** `SUBaseSystemCombo10.6.8.pkg` | `http://swcdn.apple.com/content/downloads/36/43/041-98121-A_GC9KUH2DGY/ukhwc206sod9l9fkfnsd0izm3g4d11g8r3/` + nom | 729 486 771 o et 337 054 830 o | TOC xar SHA-1 OK, lus en flux | **kexts réels dans `SUBaseSystemCombo10.6.8.pkg`** : IOBluetoothFamily / IOBluetoothHIDDriver 2.4.5f3 (x86_64, i386, ppc), AppleBluetoothHIDKeyboard 141.5, AppleBluetoothMultitouch, `blued` ; `IOBluetooth.framework` binaire **absent** des deux paquets |
| Aluminum Keyboard Firmware Update 1.1 (support.apple.com/106678, clavier aluminium **2007**) | `https://updates.cdn-apple.com/2019/cert/041-87757-20191017-0dd03310-a171-4ee2-8d82-b0df1fb3fc16/AlKybdFirmwareUpdate.dmg` | 1 779 984 o | sha256 `e169a0ba427d08e2199690f112231bc8c00762ba4de069be3a421abf7d711593` ; TOC SHA-1 OK ; signature RSA, 3 certificats | `WLKBFU/1/bfu`, `config.hex`, `fw5001.hex`, outil USB `HIDFirmwareUpdaterTool` |

URL issues des catalogues Software Update publics (`index-leopard-snowleopard.merged-1.sucatalog`,
`index-lion-snowleopard-leopard.merged-1.sucatalog`, `index-1.sucatalog`) et des pages support.apple.com. Les paquets
volumineux ont été lus **en flux** (requêtes HTTP Range → gzip → filtre cpio) : seuls les fichiers Bluetooth/HID ont
été écrits dans le bloc-notes, puis effacés après analyse.

**Comparaison avec la mission demandée**

* **Combo 10.6.8** : la conclusion « inutilisable » de `RE-PILOTES-ANCIENS.md` §1 est **fausse**. Dans les deux variantes,
  les répertoires des kexts du paquet principal ne contiennent que `_CodeSignature`, mais les binaires réels sont
  livrés dans le sous-paquet **`SUBaseSystemCombo10.6.8.pkg`** (6 079 entrées cpio, 213 fichiers BT/HID).
* **Mac OS X Server** 10.4.11, 10.5.8 et 10.6.8 : présents dans les catalogues, non relus. Ils embarquent les mêmes
  versions de composants Bluetooth que les combos client du même numéro **[déduction]**.
* **Developer Tools / SDK** : aucun en-tête IOBluetooth ou HID n'est livré dans les combos. `IOHIDKeys.h`,
  `IOBluetoothHIDDriverTypes.h` et les exemples de code ne sont publiés que dans Xcode, dont les archives exigent un
  compte développeur. Ils n'ont **pas** été téléchargés. Les sources Darwin du §3 couvrent `IOHIDKeys.h`.
* « Apple Wireless Keyboard Update 1.2 » (2007) n'existe dans aucun catalogue ni aucune page support retrouvée.

## 2. Ce que chaque version appelle (tableau d'apparition)

Union des dictionnaires `ExtendedFeatures` de tous les `Info.plist` de chaque paquet **[plist]**, et ID codés en dur dans
`IOBluetooth.framework` **[décompilé / chaîne]**.

| ID | Nom Apple | 10.2.8 (BT 1.3.2) | BT 1.5 / 10.3.9 (1.5.4) | 10.4.11 (1.9.5) | 10.5.8 (2.1.8) | 10.6.8 (2.4.5) | 10.7.5 | macOS 26.5 |
|---|---|---|---|---|---|---|---|---|
| `0x30` In | `BatteryState` | oui | oui | oui | oui | oui | oui | oui |
| `0x40` | `WillShutdown` | oui | oui | oui | oui | oui | oui | oui |
| `0x41` | `RecantConnection` | oui | oui | oui | oui | oui | oui | oui |
| `0x43` | `UserMode` (1-3) | oui | oui | oui | oui | oui | oui | oui |
| `0x44` | `FullFactoryDefault` | oui | oui | oui | oui | oui | oui | oui |
| `0x45` | `FactoryDefault` | oui | oui | oui | oui | oui | oui | oui |
| `0x47` | `BatteryPercent` (0-100) | oui | oui | oui | oui | oui | oui | oui |
| `0x49` | `BatteryVoltage` | — | — | — | — | — | **oui** | — |
| **`0x4A`** | (codé en dur : notification SCO) | — | **oui** | oui | oui | [chaîne `blued`] | oui | (#216) |
| `0x50` | `DeviceNameChange` | oui | oui | oui | oui | oui | oui | oui |
| `0x51-0x54` | `DeviceName1-4` (8 o) | oui | oui | oui | oui | oui | oui | oui |
| `0x55` | `LongDeviceName` (64 o) | — | — | — | — | **oui** | oui | oui |
| `0x60` | `CalibratedBatteryThresholds3` | — | — | — | — | — | **oui** | — |
| `0xD7` | `SuperMode` (Magic Mouse/Trackpad 781, 782, 784 seulement) | — | — | — | — | **oui** | oui | oui |

Produits couverts : 10.5.8 (2.1.8) ne connaît que les claviers 2004 (`0x208-0x20A`) et 2007 (`0x22C-0x22E`) ; les
claviers 2009 (`0x239-0x23B`) arrivent par le « Wireless Keyboard Update 2.0 » (BT 2.1.10). **10.6.8 connaît déjà
notre PID `598` (`0x0256`, « Wireless Keyboard 2009 B ISO »)**, avec la même carte que le 2009 et
`GetReportTimeoutMS = SetReportTimeoutMS = 3500` **[plist]**.

`0x55` (`LongDeviceName`) est absent de 10.5.8 (2.1.8) ; il arrive avec les personnalités 2009 du « Wireless Keyboard Update 2.0 » (BT 2.1.10, `RE-PILOTES-ANCIENS.md` §3) et figure dans 10.6.8. `0x49` et `0x60` n'apparaissent qu'avec Lion : le pilote 10.6.8
(`AppleBluetoothHIDKeyboard` 141.5) **ne surcharge pas** `updateBatteryLevel` ; il ne gère que l'arrêt du clavier
(Input `0x13`, bit 1 = 0 → notification `koff`) **[décompilé x86_64 et i386]**.

## 3. Sources Darwin (opensource.apple.com, miroir officiel `apple-oss-distributions`)

Archives des tags `IOHIDFamily-86.26`, `-185.38`, `-258.14`, `-315.7.16`, `-368.20` (10.3 à 10.7) et `IOKitUser-120.4`,
`-184`, `-297.13`, `-388.53.30`. Aucun dépôt `IOBluetoothFamily` ou `IOBluetooth` n'a été publié.

| Recherche | Résultat |
|---|---|
| `BatteryVoltage`, `WillShutdown`, `FactoryDefault`, `DeviceName1-4`, `ExtendedFeatures`, `SuperMode`, `RecantConnection`, `LongDeviceName`, `BatteryPercent`, `SCOLink`, `UserMode` | **0 occurrence** |
| PID `0x239-0x23B`, `0x255-0x257` | seul faux positif : `kHIDUsage_Csmr_ACNewWindow = 0x239` (table d'usages) |
| Page vendeur Apple `0xFF01` (`AppleHIDUsageTables.h`) | usages clavier `Function 0x03`, `Launchpad 0x04`, `Dashboard 0x02`, `Expose_* 0x10-0x11`, `Brightness 0x20-0x21`, **`CapsLockDelayEnable 0x0B`**, **`PowerState 0x0C`** (368.20). Ce sont des **usages**, pas des ID de rapport ; aucun lien établi avec nos registres |

Conclusion : le code source Darwin ne décrit aucun rapport vendeur des claviers Bluetooth Apple. Toute la logique est
dans des binaires fermés (`IOBluetoothHIDDriver`, `AppleBluetoothHIDKeyboard`, `IOBluetooth.framework`).

## 4. Analyse statique (Ghidra headless)

**Méthode.** Pour chaque binaire, extraction de la tranche (`llvm-lipo` ou découpe du fat header pour `ppc7400`),
projet Ghidra 12.1.2 jetable, analyse automatique complète, puis script post-analyse `ReportRefs` qui :
(1) liste toute instruction dont un opérande scalaire vaut l'un des ID visés (`0x46 0x47 0x49 0x4A 0x4B 0x5A 0x60 0xD0-0xDF
0xEB 0xF0-0xFF`, `0xF30D`) ; (2) décompile ces fonctions et toutes celles dont le nom évoque un rapport, une batterie,
un mode, un nom, une commande ou une mise à jour. Les décompilations restent dans le bloc-notes (non commitées).

| Binaire | Versions et tranches analysées | Fonctions décompilées |
|---|---|---|
| `IOBluetoothHIDDriver` (`IOBluetoothHIDDriver`, `IOAppleBluetoothHIDDriver`) | 10.4.11 ppc ; 10.5.8 i386 + ppc ; 10.6.8 x86_64 + i386 + ppc | 58 à 75 par tranche |
| `AppleHIDKeyboard` / `AppleBluetoothHIDKeyboard` | 10.4.11 ppc ; 10.5.8 i386 + ppc ; 10.6.8 x86_64 + i386 | 1 à 8 |
| `IOBluetoothFamily` | 10.5.8 i386 + ppc | 464 et 528 |
| `IOBluetooth.framework` (`AppleBluetoothHIDDevice`) | 10.4.11 ppc ; 10.5.8 i386 | 748 et 364 |
| `blued` | 10.4.11 ppc ; 10.5.8 i386 ; 10.6.8 x86_64 | 103, 43, 124 |
| `IOBluetoothUI`, `Bluetooth.prefPane`, `BluetoothUIServer` | 10.4.11 ppc (prefPane) ; 10.5.8 i386 | 0 à 72 |
| `bfu` 2007 et 2009, application « Aluminum Wireless Keyboard Firmware Update » | i386 + ppc | 5 à 7 |

### 4.1 Le pilote noyau : uniquement des noms

* `IOAppleBluetoothHIDDriver::getExtendedReport(const char*)` / `setExtendedReport(const char*, void*, size)` résolvent
  le nom dans `ExtendedFeatures` puis émettent GET/SET_REPORT. `processCommandWL` n'accepte que des chaînes :
  `UpdateBatteryLevel`, `UpdateBatteryState`, `StartBatteryUpdate`, `StopBatteryUpdate`, `BatteryState`, `CapsLock`,
  `WillShutdown`, `ReleaseAllChannelsWithSleepForHIDUpdate`, plus les propriétés `BatteryUpdateInterval`,
  `DefaultBatteryUpdateInterval`, `ForceBatteryPercent` / `DontForceBatteryPercent` **[décompilé 10.6.8, chaînes]**.
* `processInterruptData` ne reconnaît que **`0x30`** (`a1 30 xx` → `updateBatteryState`), en 10.4.11, 10.5.8 et 10.6.8.
  En 10.4.11 seulement, un cas `a1 03` pour les PID `0x309`/`0x30C` (souris). **Aucun** traitement de `0x04`/`0x05`
  **[décompilé]**.
* Faux positifs écartés : `0xD0`/`0xD4`/`0xD8` = décalages de structure (`decrementOutstandingIO`, `willTerminate`,
  `setPowerStateWL`…), `0xFA`/`0xFB` = types de paquets L2CAP (`receive12Packet`, décodeur de traces de `blued`),
  `0xD4 0xD5 0xFA` dans `_CreateHIDDeviceInterface` = octets d'UUID CFPlugIn, `0x4B` dans
  `AppleHIDKeyboardEventDriver::handleStart` = **75 ms** de délai Verr. Maj (PID `0x251` fw > `0x67`, ou fw > `0x136`).

### 4.2 Calcul du pourcentage (`0x47`) selon les époques

| Version | Noyau | Espace utilisateur (`-[AppleBluetoothHIDDevice batteryPercent]`) | Preuve |
|---|---|---|---|
| 10.2.8 → 10.4.11 | GET `0x47`, octet 1, borné à 100, publié `BatteryPercent` | méthode présente | [chaîne] ; [décompilé ppc 10.4.11 noyau] |
| 10.5.8 | idem (`if (v > 100) v = 100`) | lit la propriété 32 bits `r` et rend `(r_hi16 × 65536 + r_lo16) / 100` = **`r / 100`, linéaire**, `-1` si absent ; demande d'abord `UpdateBatteryLevel` si la propriété manque | [décompilé i386] ; constantes `100.0`, `65536.0`, `-1.0` lues dans `__literal4` |
| 10.6.8 | idem ; `ForceBatteryPercent` (débogage) remplace la valeur | binaire du framework absent du combo | [décompilé x86_64] |
| 10.7.5 → 26.5 | idem | **courbe** `r ≥ 54 → 100 ; 21-53 → 21 + (r−21) × 2,4375` pour `0x239-0x23B` et `0x255-0x257` | `RE-PILOTES-ANCIENS.md` §4, #213 |

**Formule exacte de `0x47`** : aucun logiciel Apple ne la calcule. De 2003 à 2026, le système recopie l'octet du
firmware, borné à 100. Le seul calcul côté hôte est l'**affichage** : linéaire jusqu'à 10.5.8, courbe 54/21/2,4375
depuis Lion. La conversion tension → pourcentage reste dans le firmware du BCM2042, non publié.

### 4.3 `0x4A` : notification de lien audio SCO, depuis 2004

`-[AppleBluetoothHIDDevice sendSCODevicePaired]` → `setFeatureWithReportID:0x4A value:1`, `sendSCODeviceUnpaired` → `2`,
`sendSCOLinkActive` → `3`, `sendSCOLinkInactive` → `4` **[décompilé 10.4.11 ppc et 10.5.8 i386]**.
`setFeatureWithReportID:value:` construit un rapport de 2 octets `[ID][valeur]` et l'envoie par
`IOHIDDeviceInterface::setReport(kIOHIDReportTypeFeature, ID, buf, 2, 1000 ms)` **[décompilé]**.

| Version | État |
|---|---|
| 10.2.8 (BT 1.3.2) | **absent** : la classe `AppleBluetoothHIDDevice` n'a que `batteryPercent`, `deviceNameFromHardware`, `setDeviceName:` [chaîne] |
| BT 1.5 (février 2004) → 10.3.9 | **apparaît** avec le support des casques [chaîne : les 4 méthodes] |
| 10.4.11, 10.5.8 | identique, valeurs 1-4 [décompilé] |
| 10.6.8 | `blued` contient `setConnectedAppleHIDDevicesToSCOLinkActive:excludeDevice:`, `sendSCOLinkActive`, `resetSniffParameters` [chaîne] |
| 10.7.5 | identique, liste de PID incluant `0x0255-0x0257` (`RE-PILOTES-ANCIENS.md` §5) |

**Lecture de `0x4A`** (nous lisons `0x12`) : aucun logiciel Apple ne fait de GET sur `0x4A`. Le sens de `0x12` n'a
**pas** de réponse logicielle. **[hypothèse]** un état interne codé en bits, que seul un essai d'écriture 3/4 contrôlé
(#216) pourrait éclairer.

## 5. Updaters : format et canal uniquement

Deux updaters Bluetooth de clavier existent : 2007 (`WLKBFU/1`, clavier aluminium A1255, PID `0x22C-0x22E`) et 2009
(`WLKBFU/2`, PID `0x239-0x23B`, déjà décrit dans `RE-PILOTES-ANCIENS.md` §6). **Aucun** ne vise `0x0255-0x0257`.

| Élément | 2007 (Aluminum Keyboard Firmware Update 1.1) | 2009 (2009 Aluminum Keyboard Firmware Update 1.0) |
|---|---|---|
| Condition (`Distribution`) | `IOAppleBluetoothHIDDriver`, VID `0x5AC`, PID `0x22C/0x22D/0x22E`, `VersionNumber < 0x141` ; variante USB : PID `0x220-0x222` `bcdDevice < 0x69` ou `0x228` | PID `0x239-0x23B`, `VersionNumber` `0x44`/`0x46` |
| `Parameters.plist` | `FWVersion = 321` (`0x141`), `PageTimeout = 65535`, `AckRecords = false` | `FWVersion = 80` (`0x50`), `PageTimeout = 32768`, `AckRecords = true` |
| Images | `config.hex` 3 610 o + **`fw5001.hex` 342 743 o** | `config.hex` 35 296 o seulement |
| Nature des images | entropie 7,92 et 7,98 bit/octet : **chiffrées**. Les deux fichiers commencent par des octets proches (`e3 c4 …` / `e3 c5 …`) **[mesuré]** | entropie 7,98 |
| `bfu` | 64 196 o, i386 + ppc7400 | 64 196 o, même code |
| Différence entre les deux `bfu` | **16 octets contigus** (offset 2061 de la tranche i386) **[mesuré]**. **[déduction]** une constante de 16 octets propre à chaque produit (clé ou vecteur) ; **non examinée, non relevée** | |

**Protocole (identique dans les deux `bfu`, [décompilé i386])**

| Étape | Trame hôte → clavier | Réponse attendue |
|---|---|---|
| Canal | `IOBluetoothDeviceOpenL2CAPChannelSync(…, PSM 0xF30D)` après connexion baseband (`PageTimeout`) ; politique de lien `0x000F` restaurée en fin | — |
| Début | `D1 01 <AckRecords>` | `D2` |
| Préparation | `D4 00` | `D5` |
| Sélection de banque | `D6 01 <n°>` | `D7`, puis lecture de l'image |
| Enregistrement | `D8 <longueur−2> <type> <adresse 16 bits> <données…> <somme>` : un enregistrement texte déchiffré, l'adresse recalée sur la banque (`fwBank1BaseAddr`, `fwFSOffSet`), somme de contrôle à complément | `D9` (attente 20 s) si `AckRecords` |
| Fin | `DA 01 <−Σ octets envoyés>` | `DB` |
| Abandon | `D3 00` | `D3` |
| Refus | — | `DC` (accepté en réponse à `D1` ou `D6`) |

Le canal de mise à jour du clavier est donc, **depuis 2007**, un PSM L2CAP vendeur distinct des canaux HID. Les opcodes
`0xD1-0xDC` sont des octets de protocole sur ce canal, **pas** des ID de rapport HID. **Aucun** code Apple n'envoie un
SET_REPORT `0xD0-0xDF` sur le canal HID. La coïncidence avec nos ID Feature `0xD0 0xD1 0xD4 0xD5 0xD8` reste une
coïncidence de plage : **[déduction]** le firmware range peut-être ses fonctions de service sous `0xDx`, ce qui ne
prouve aucun lien. Aucun outil de flashage n'a été écrit. `fw5001.hex` et `config.hex` n'ont pas été déchiffrés.

## 6. Table finale : registre → fonction → preuve → risque → apparition

| Registre | Trouvé ? | Fonction | Source exacte | Risque | Apparaît dans |
|---|---|---|---|---|---|
| `0xD0` (Feature WO) | **non** | inconnue | absent des 8 paquets (§1), de 10.7.5 et de 26.5 ; seulement des décalages de structure | inconnu → **ne jamais écrire** | aucune version |
| `0xD4` (Feature WO) | **non** (comme ID HID) | inconnue ; `0xD4` est aussi l'opcode « préparation » de l'updater sur PSM `0xF30D` | `bfu` 2007/2009 [décompilé] | **potentiellement maintenance** → ne jamais écrire | opcode updater 2007, 2009 |
| `0xD5` (Feature WO) | **non** (comme ID HID) | inconnue ; `0xD5` = acquit de `D4` côté updater | idem | idem | idem |
| `0xFA` (Feature WO) | **non** | inconnue | absent ; occurrences = types de paquets L2CAP, octets d'UUID | inconnu → ne jamais écrire | aucune |
| `0xFB` (Feature WO) | **non** | inconnue | idem | idem | aucune |
| `0x4B` (Feature, `00 08`) | **non** | inconnue ; la constante `0x4B` du pilote clavier est 75 ms de délai Verr. Maj, sans rapport | `AppleHIDKeyboardEventDriver::handleStart` [décompilé] | lecture rare | aucune (comme rapport) |
| `0xD1` (Feature, 0) | **non** (comme ID HID) | inconnue ; `D1` = opcode « début » de l'updater | `bfu` [décompilé] | lecture rare, jamais d'écriture | opcode updater |
| `0xD8` (Feature, 0) | **non** (comme ID HID) | inconnue ; `D8` = opcode « enregistrement de firmware » de l'updater | `bfu` [décompilé] | lecture rare, **jamais d'écriture** | opcode updater |
| `0xF6`, `0xF7` (Feature, `00 04`) | **non** | inconnues | absents de tous les logiciels examinés | lecture rare | aucune |
| Input `0x04`, `0x05` | **non** | inconnus ; jamais traités par `processInterruptData` (seul `0x30`, et `0x13` en 10.6+) | [décompilé 10.4.11 → 10.6.8] | nul (passif) | aucune |
| `0x4A` (écriture) | **oui** | notification casque : 1 appairé, 2 désappairé, 3 lien SCO actif, 4 inactif | `-[AppleBluetoothHIDDevice sendSCO*]` → `setFeatureWithReportID:0x4A value:n` [décompilé] | faible (trafic Apple de production) | BT 1.5 (2004), 10.3.9 → 10.7.5 |
| `0x4A` (lecture `0x12`) | **non** | inconnu : Apple n'a jamais lu `0x4A` | — | lecture | aucune |
| `0x47` (formule) | **oui, côté affichage seulement** | octet firmware borné à 100 ; affichage linéaire `r/100` jusqu'à 10.5.8, courbe 54/21/2,4375 depuis 10.7 | §4.2 | lecture | toutes |
| `0x55` | oui | `LongDeviceName` 64 o | [plist] | moyen en écriture (#192) | BT 2.1.10 (2009) / 10.6.8 → 26.5 |
| `0x49`, `0x60` | oui | `BatteryVoltage`, `CalibratedBatteryThresholds3` | [plist] | lecture (#215) | **10.7 seulement** |
| `0xD7` | oui, autre produit | `SuperMode` des Magic Mouse/Trackpad ; refusé `0x02` par notre clavier [mesuré] | [plist 10.6.8] | sans objet | 10.6.x → 26.5 |

**Conclusion.** Le gérant avait raison sur le fond : le système appelle bien des registres, et la liste complète
est désormais connue pour chaque version de 10.2.8 à 26.5 (§2). Mais **aucune** de ces versions n'appelle les onze
registres encore inexpliqués. Leur sens ne peut venir que du firmware lui-même : dump de la mémoire externe (#184/#185),
ou image `fw5001.hex` du clavier 2007, qui est chiffrée et ne sera pas déchiffrée dans ce cadre.

## 7. Corrections à apporter aux autres documents

| Document | Affirmation | Correction |
|---|---|---|
| `RE-PILOTES-ANCIENS.md` §1 | Combo 10.6.8 « inutilisable » | kexts réels dans `SUBaseSystemCombo10.6.8.pkg` (produits `041-98121` et `041-98179`) ; seul le binaire `IOBluetooth.framework` manque |
| `RE-PILOTES-ANCIENS.md` §5/§7 et `RE-PILOTE-MACOS.md` | `0x4A` : notification SCO « de Lion » | existe depuis **Bluetooth 1.5 (2004)**, valeurs 1-4 inchangées jusqu'à 10.7.5 |
| `RE-PILOTES-ANCIENS.md` §4 | courbe d'affichage « depuis Lion » | confirmé : 10.5.8 affichait le brut linéaire (`r/100`) |
| `RE-PILOTES-ANCIENS.md` §7 | `0xD0 0xD4 0xD5 0xFA 0xFB` … « aucun des quatre logiciels » | aucun des **douze** logiciels (8 paquets de ce document + 4 précédents) |
| `RE-PILOTES-ANCIENS.md` §6.2 | séquence `D1…DB` | ajouter `D6 <banque>`, la trame `D8` détaillée et le refus `DC` ; le protocole est identique en 2007 (`WLKBFU/1`) |
| `RE-COMMANDES-VENDEUR.md` | `0xFA`/`0xFB`, `0xDx` : hypothèses de maintenance | ajouter : absents de 10.2.8 à 26.5 ; seul l'updater (canal L2CAP `0xF30D`) utilise `0xD1-0xDC` |

## 8. Reproduire (lecture seule, sans clavier)

1. Catalogues `swscan.apple.com/content/catalogs/others/index-leopard-snowleopard.merged-1.sucatalog` et
   `index-1.sucatalog` (`catalogs/`) → URL `swcdn.apple.com` ; pages support.apple.com `106464`, `106678` pour les DMG.
2. Paquets xar (10.5+) : lire l'en-tête et la TOC par requêtes Range, vérifier le SHA-1 de la TOC, diffuser `Payload`
   dans `gzip -dc` et un filtre cpio `odc` sur `bluetooth|hid|keyboard|blued`. Pour 10.6.8, lire **aussi**
   `SUBaseSystemCombo10.6.8.pkg`. Paquets `.tar` (10.4) : tar imbriqué → `Archive.pax.gz` en flux. DMG (10.2, 10.3) :
   `7z x`, puis `Archive.pax.gz`.
3. `extfeat`-like : parcourir les `Info.plist`, relever `IOKitPersonalities/*/ExtendedFeatures` (`id`, `type`, `size`, `min`, `max`).
4. Ghidra headless (`/opt/ghidra/support/analyzeHeadless`), tranche par tranche, script post-analyse qui liste les
   scalaires `0x46-0x4B 0x5A 0x60 0xD0-0xDF 0xEB 0xF0-0xFF 0xF30D` et décompile les fonctions touchées, puis éliminer
   les décalages, UUID et délais. Points d'entrée : `IOAppleBluetoothHIDDriver::{getExtendedReport,setExtendedReport,
   updateBatteryLevel,processInterruptData,processCommandWL}`, `-[AppleBluetoothHIDDevice sendSCO*|setFeatureWithReportID:value:|batteryPercent]`,
   fonctions de `bfu` qui contiennent `0xF30D` et `0xD1-0xDC`.
5. **Ne pas** examiner la constante de 16 octets qui distingue les deux `bfu`, ni `FWDecrypt`, ni les `.hex`.

## 9. Suivi Gitea

| Issue | Objet |
|---|---|
| #229 | docs : corrections du §7 (combo 10.6.8 `SUBaseSystem`, `0x4A` depuis BT 1.5, protocole `bfu` complet, table d'apparition) |
| #216 | commentaire : historique de `0x4A` (1-4) de 2004 à 10.7.5, lecture `0x12` inexpliquée |
| #188 | commentaire : registres WO absents de 10.2.8 → 10.6.8 ; canal de mise à jour PSM `0xF30D`, identique en 2007 et 2009 |
| #213 | commentaire : 10.5.8 affichait le brut linéaire, la courbe arrive entre 10.6 et 10.7 |

Aucune nouvelle fonction sûre n'est exploitable : toutes les fonctions qu'Apple appelait sur notre PID ont déjà une issue
(#189, #190, #191, #192, #213, #215, #216, #217).
