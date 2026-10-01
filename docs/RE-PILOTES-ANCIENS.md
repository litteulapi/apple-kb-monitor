# RE — Pilotes et updaters Apple de l'époque (2009-2012) pour l'A1314 (BCM2042, firmware 0x0050)

Analyse **statique, en lecture seule**, de logiciels Apple **publiquement distribués** par les serveurs
d'Apple, à des fins d'interopérabilité avec le clavier du gérant (A1314 ISO, `05AC:0256` = 598,
bcdDevice `0x0050`). Complète `RE-PILOTE-MACOS.md` (macOS 26.5), qui n'envoie presque rien de spécifique.

* Aucun binaire Apple, aucun code décompilé, aucune clé n'est commité : seulement des faits courts
  (IDs, tailles, formats, noms de symboles et de clés).
* Aucun accès au clavier réel, aucune écriture HID. Aucun outil de flash n'est écrit.

Niveaux de preuve : **[plist]**, **[désassemblage]**, **[chaîne]**, **[déduction]**, **[mesuré]** (renvoie
à nos mesures : `RE-HID-EXHAUSTIF.md`, `HARDWARE-RAPPORTS-HID.md`).

*(document rédigé au fil de l'eau ; sections complétées par commits successifs)*

## 1. Sources téléchargées (serveurs Apple publics) et intégrité

| Paquet | URL officielle (Apple) | Taille | Empreinte |
|---|---|---|---|
| `AlKybdSoftwareUpdate.pkg` (« Wireless Keyboard Update 2.0 », produit `041-85086`) | `http://swcdn.apple.com/content/downloads/25/51/041-85086-A_0O8GEX5RKH/c20e4b3bz7iq0rp0rgdreyuy4rr1yzpix8/AlKybdSoftwareUpdate.pkg` | 11 332 239 o (= taille du catalogue) | sha256 `efb9618a…4abd` ; TOC xar SHA-1 vérifiée ; signature RSA, 3 certificats |
| `BluetoothFirmwareUpdate2.0.1.pkg` (produit `041-85074`) | `http://swcdn.apple.com/content/downloads/35/36/041-85074-A_MOUDL4NHT5/x3thzouodq1tckv99fa2albrqjf0k2ns4y/BluetoothFirmwareUpdate2.0.1.pkg` | 1 671 448 o (= catalogue) | sha256 `9f064473…7428` ; TOC SHA-1 OK ; RSA |
| `WirelessKybdFirmwareUpdate.dmg` (« 2009 Aluminum Keyboard Firmware Update 1.0 », support.apple.com/en-us/106755) | `https://updates.cdn-apple.com/2019/cert/041-87809-20191017-6ee067c4-5369-4075-bc88-f1cfb623f02b/WirelessKybdFirmwareUpdate.dmg` | 886 799 o | sha256 `449d819b…905d` ; pkg interne : TOC SHA-1 OK ; RSA |
| `MacOSXUpdCombo10.7.5.pkg` (produit `041-98155`) | `http://swcdn.apple.com/content/downloads/30/25/041-98155-A_E2HC6UPBOQ/y8k2fq4q0u2jo4ci8txpxkw96l9dzfsi19/MacOSXUpdCombo10.7.5.pkg` | 2 041 831 704 o | lu en flux (`Payload` 2 035 347 918 o, SHA-1 du flux `e8b7b6b5…` ), TOC SHA-1 OK ; seuls les composants Bluetooth/HID ont été extraits, rien n'a été stocké |

Les URL proviennent des catalogues publics de Software Update
(`https://swscan.apple.com/content/catalogs/others/index-lion-snowleopard-leopard.merged-1.sucatalog`).
Apple ne publie pas d'empreinte pour ces paquets : l'intégrité est établie par la taille annoncée dans le
catalogue, la somme SHA-1 de la table des matières xar et les sommes de chaque fichier de l'archive
(toutes vérifiées), plus la présence d'une signature RSA du paquet (chaîne de 3 certificats Apple).
Le Combo 10.6.8 (`041-98179`) a été écarté : il est découpé en 13 parties et la partie contenant les
répertoires des kexts Bluetooth/HID ne contient que des liens de signature, pas les binaires.

Outils (sans sudo) : lecteur xar en Python (vérification TOC + sommes), `gzip`/`cpio`, `7z` (DMG),
`llvm-objdump`/`llvm-nm`/`llvm-lipo`, capstone 5 (désassemblage i386/x86_64).

## 2. Quel logiciel pilotait quel A1314 : deux générations de PID

| Élément | Constat | Preuve |
|---|---|---|
| A1314 « 2009 » | PID `0x0239/0x023A/0x023B` (569/570/571, ANSI/ISO/JIS) | [plist] personnalités « Wireless Keyboard 2009 » de `AppleHIDKeyboard` 1.1.2f2 (paquet 2009) et 160.7 (10.7.5) |
| A1314 « 2009 B » (le nôtre) | PID `0x0255/0x0256/0x0257` (597/598/599) | [plist] « Wireless Keyboard 2009 B ISO » = **598** ; **absent** du paquet 2009 (Bluetooth 2.1.10), **présent** dans 10.7.5 |
| Différence 2009 / 2009 B côté Apple | **une seule** : la touche **F4** (`0x07:0x3D`) est remappée vers `0xFF01:0x0002` (Dashboard) sur 570, vers **`0xFF01:0x0004` (Launchpad)** sur 598. `ExtendedFeatures`, délais, notifications : identiques | [plist], comparaison clé par clé |
| Mise à jour de firmware 2009 (`bfu`) | ne vise **que** `0x0239-0x023B` avec `VersionNumber` `0x0044` ou `0x0046` (ou absent) ; porte la cible `FWVersion = 80` = **`0x0050`** | [chaîne] fonctions `hasAluminumKeyboard()` / `needsWirelessKeyboardUpdate()` du `Distribution` ; [plist] `Parameters.plist` |

**[déduction]** Notre clavier (`0x0256`, `0x0050`) est la révision « B » sortie avec Lion (F4 = Launchpad), livrée
**directement** avec le firmware `0x0050` — celui que l'updater de 2009 installait sur la première révision.
L'updater 2009 **ne s'applique pas** à notre PID : il n'existe **aucun** updater public pour `0x0255-0x0257`.
Le firmware `0x0050` est donc la **dernière** version publiée pour la famille.

## 3. `ExtendedFeatures` complet de l'époque (10.7.5, PID 598) — deux registres disparus de macOS 26

Personnalité `AppleBluetoothHIDKeyboard` 160.7, « Wireless Keyboard 2009 B ISO » **[plist]** (`type` : 0 Input, 2 Feature) :

| Nom Apple | ID | Type | Taille | Bornes | macOS 26.5 | Notre mesure |
|---|---|---|---|---|---|---|
| `BatteryState` | `0x30` | Input | 1 | 0-2 | présent | `30 00` |
| `WillShutdown` | `0x40` | Feature | — | — | présent | refus GET `0x03` |
| `RecantConnection` | `0x41` | Feature | — | — | présent | refus GET `0x03` |
| `UserMode` | `0x43` | Feature | 1 | 1-3 | présent | `0x02` (inexistant) |
| `FullFactoryDefault` | `0x44` | Feature | — | — | présent | refus GET `0x03` |
| `FactoryDefault` | `0x45` | Feature | — | — | présent | refus GET `0x03` |
| `BatteryPercent` | `0x47` | Feature | 1 | 0-100 | présent | `47 63` |
| **`BatteryVoltage`** | **`0x49`** | Feature | **2** | — | **absent** | `49 86 0b` = 2950 mV |
| `DeviceNameChange` | `0x50` | Feature | — | — | présent | refus GET `0x03` |
| `DeviceName1..4` | `0x51-0x54` | Feature | 8 | — | présent | nom ASCII |
| `LongDeviceName` | `0x55` | Feature | 64 | — | présent | refus GET `0x03` |
| **`CalibratedBatteryThresholds3`** | **`0x60`** | Feature | **8** | — | **absent** | `60 0b8a 09ca 0964 0806` |

La personnalité de 2009 (Bluetooth 2.1.10, PID 570) a la même carte **sans** `BatteryVoltage` ni
`CalibratedBatteryThresholds3` ; celle de 2007 (PID 557) n'a pas non plus `LongDeviceName` **[plist]**.

## 4. Batterie à l'époque de Lion : ce que lisait le pilote

`AppleBluetoothHIDKeyboard` 160.7 (10.7.5) **surcharge** `updateBatteryLevel` **[désassemblage]** :

1. appelle d'abord l'implémentation de base (`IOAppleBluetoothHIDDriver`) : GET Feature `0x47`, octet 1,
   borné à 100, publié tel quel (`BatteryPercent`). Même logique en 2009 (Bluetooth 2.1.10) et en 2026 :
   **le pourcentage a toujours été calculé par le micrologiciel**, jamais par macOS **[désassemblage]** ;
2. sauf si `DisableBatteryProperties`, construit un dictionnaire `Battery` :
   * `MeasuredVoltages.Latched` = `getLatchedBatteryVoltage()` = GET Feature **`0x49`** (`BatteryVoltage`),
     **u16 petit-boutiste**, `-1` en cas d'échec ;
   * `UsedThresholds` = `getVoltagesUsed()` = GET Feature **`0x60`** (`CalibratedBatteryThresholds3`),
     **4 × u16 gros-boutiste** nommés dans l'ordre **`Full`, `Low`, `Critical`, `Empty`** ;
   * `Chemistry` (`getBatteryChemistry`) et `Calibration` (`getVoltagesCalibrated`) : **méthodes vides** (renvoient
     NULL) pour ce clavier ; seules les clés existent ;
   * `Bluetooth` = `TotalBytesSent` / `TotalBytesReceived` du lien (propriétés de l'`IOBluetoothDevice`, pas un rapport HID) ;
3. remplace chaque clé par une abréviation (table `_BatteryAbbreviations`, 28 paires) puis publie le tout,
   **sérialisé en XML**, comme propriété binaire `Battery` de l'IORegistry.

Abréviations **[chaîne + désassemblage]** : `MV` MeasuredVoltages, `LT` Latched, `UT` UsedThresholds,
`F` Full, `L` Low, `C` Critical, `E` Empty, `A`/`B` LevelA/LevelB, `CH` Chemistry, `CT` ChemistryType,
`NM`/`NN` NiMH/NonNiMH, `IV` 6xInitialVoltage, `V1`/`V6` 6x10OhmLoadedVoltage1ms/6ms, `IR` InternalResistance,
`VR` VxVxR, `TR` TraceResistance, `DT` DetectionThreshold, `UN`/`LD` Unloaded/Loaded, `CA` Calibration,
`BT` Bluetooth, `S`/`R` TotalBytesSent/Received, `SI` AvgRSSI, `#` NumSamplesRSSI.

**Conséquences**

* Le blob IORegistry `"Battery" = <"MVLT…` relevé en 2014 (managingosx) se lit **`MV{LT=…}`** = tension
  « verrouillée » lue dans `0x49`. L'hypothèse « `MVLT` = millivolts » était juste sur le fond ; `RE-PILOTE-MACOS.md` §6
  ne vaut que pour macOS 26 (qui ne lit plus `0x49`) **[désassemblage]**.
* **`0x5A`/`0x60`/`0xEB` ne sont pas des seuils 100/75/50/25 %** : Apple nomme les quatre valeurs de `0x60`
  `Full` (2954 mV), `Low` (2506 mV), `Critical` (2404 mV), `Empty` (2054 mV). `Low` et `Critical` correspondent aux
  deux niveaux de `BatteryState` (1 = `LowBattery`, 2 = `CriticallyLowBattery`) ; `Empty` est la tension de fin
  **[plist + désassemblage pour les noms ; déduction pour le lien avec `BatteryState`, à confirmer passivement]**.
  Le suffixe `3` suggère que `0x5A` et `0xEB` (copies mesurées) sont les jeux 1 et 2 **[déduction]**.
* `0x49` est la grandeur qu'Apple appelle **`Latched`** (tension « verrouillée », mesurée puis figée par le micrologiciel),
  ce qui explique qu'elle varie par paliers et sans bruit (`HARDWARE-RAPPORTS-HID.md` §4) **[plist + déduction]**.
* **Cadence et sûreté** : Lion lisait `0x47`, `0x49` et `0x60` à **chaque** relevé batterie (60 s après connexion,
  puis toutes les 4 h) **[désassemblage]**. Ces trois lectures font donc partie du trafic de production d'Apple
  pour notre PID : elles sont **sûres au rythme Apple** (aucune ne figeait le clavier). `0xFE`, `0xEA`, `0xF4`-`0xFF`,
  `0x46`, `0x4A`-`0x4C` n'ont été lus par **aucune** version examinée.
