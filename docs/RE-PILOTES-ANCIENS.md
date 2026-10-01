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
  pour notre PID : elles sont **validées par Apple au rythme d'Apple** (au plus une série toutes les 4 h), ce qui ne garantit rien pour des rafales. `0xFE`, `0xEA`, `0xF4`-`0xFF`,
  `0x46`, `0x4A`-`0x4C` n'ont été lus par **aucune** version examinée.

## 5. Ce que les pilotes de l'époque envoyaient réellement à notre PID (10.7.5)

Chronologie complète, à comparer avec `RE-PILOTE-MACOS.md` §5 (macOS 26.5). Octets « fil » HIDP.

| # | Quand | Émetteur | Transaction | Octets | Preuve |
|---|---|---|---|---|---|
| L1 | connexion | `IOBluetoothHIDDriver` 4.0.8 | SET_PROTOCOL(Report) | `71` | [désassemblage] identique à 26.5 |
| L2 | 60 s après connexion, puis toutes les 4 h (1 h après échec) | `IOAppleBluetoothHIDDriver` | GET Feature `0x47` | `43 47` | [désassemblage] constantes `60000`, `14400000`, `3600000` ms |
| L3 | juste après L2 | `AppleBluetoothHIDKeyboard` 160.7 | GET Feature **`0x49`** (`BatteryVoltage`) | `43 49` | [désassemblage] `getLatchedBatteryVoltage` |
| L4 | juste après L3 | idem | GET Feature **`0x60`** (`CalibratedBatteryThresholds3`) | `43 60` | [désassemblage] `getVoltagesUsed` |
| L5 | après chaque relevé, et sur `A1 30 xx` | `IOAppleBluetoothHIDDriver` | GET Input `0x30` | `41 30` | [désassemblage] |
| L6 | veille / réveil | `IOBluetoothHIDDriver` (`SuspendSupported` posé à vrai par la classe Apple) | HID_CONTROL SUSPEND / EXIT_SUSPEND | `13` / `14` | [désassemblage] `handleSleep` → `hidControl(3)`, `handleWake` → `hidControl(4)` |
| L7 | arrêt / redémarrage | `IOAppleBluetoothHIDDriver` | SET Feature `0x40` (`WillShutdown`) | `53 40` | [désassemblage] identique à 26.5 |
| L8 | Verr. Maj | idem | DATA Output `0x01` | `A2 01 02` / `A2 01 00` | [désassemblage] |
| **L9** | **un lien audio SCO (casque) s'ouvre / se ferme** pendant que le clavier est connecté | `blued` (`BluetoothHIDManager`) via `IOBluetooth.framework` | **SET Feature `0x4A`** = `03` (`sendSCOLinkActive`) / `04` (`sendSCOLinkInactive`) | `53 4A 03` / `53 4A 04` | [désassemblage] liste de PID testée **avant** l'envoi : `0x0208-0x020A`, `0x022C-0x022E`, `0x0239-0x023B`, **`0x0255-0x0257`**, `0x0309`, `0x030C` ; [chaîne] `sending sendSCOLinkACTIVE to %s`, `resetSniffParameters - Setting to SCOActive? %d` |
| **L10** | **« Supprimer » le clavier dans Préférences Système > Bluetooth** (après confirmation `kRemoveDevice`) | `Bluetooth.prefPane` → `-[AppleBluetoothHIDDevice fullFactoryDefault]` | **SET Feature `0x44`** (`FullFactoryDefault`, sans données), **puis** désappairage côté hôte (`remove`) | `53 44` | [désassemblage] séquence `withBluetoothDevice:` → `fullFactoryDefault` → `remove` |
| **L11** | renommage du clavier (Préférences ou Assistant Bluetooth) | `-[AppleBluetoothHIDDevice setDeviceName:]` | si `LongDeviceName` existe (cas du 598) : **SET Feature `0x55`** (64 o) ; sinon SET `0x51`…`0x54` (8 o chacun) puis SET `0x50` (`DeviceNameChange`) ; ensuite requête de nom distant (HCI) | `53 55 …` | [désassemblage] `getMaxDeviceNameLength` = 64 si `0x55` déclaré, 32 sinon |

**Correspondance avec macOS 26 [déduction forte]** : la liste de PID de L9 est **exactement** celle du « bit 3 »
reconstruit dans `bluetoothd` 26.5 (`RE-PILOTE-MACOS.md` §7). Le bit 3 désigne donc les **anciens appareils HID Apple
qui reçoivent la notification de lien SCO** (`0x4A`) et dont `blued` ajuste les paramètres de sniff quand un casque
est actif. C'est la réponse à l'inconnue n° 6 de `RE-PILOTE-MACOS.md` §9.

Valeurs de `0x4A` définies par `AppleBluetoothHIDDevice` **[désassemblage]** : `1` SCODevicePaired, `2` SCODeviceUnpaired,
`3` SCOLinkActive, `4` SCOLinkInactive (seules 3 et 4 sont envoyées par `blued`). Notre lecture `4a 12` (= 18) **[mesuré]**
n'est pas une de ces valeurs : le GET renvoie un état interne dont le codage reste inconnu.

Commandes acceptées par le pilote noyau depuis l'espace utilisateur (propriétés IORegistry) **[chaîne + désassemblage]** :
`WillShutdown`, `UpdateBatteryLevel`, `ForceBatteryPercent`/`DontForceBatteryPercent`, `BatteryUpdateInterval`,
`DefaultBatteryUpdateInterval`, `StartBatteryUpdate`/`StopBatteryUpdate`, `BatteryState`, `UpdateBatteryState`, `CapsLock`,
`ReleaseAllChannelsWithSleepForHIDUpdate` (utilisée par l'updater, §6), et côté classe générique `HIDSuspend`,
`HIDExitSuspend`, `VirtualCableUnplug`, `SetIdle`/`GetIdle`, `SetBootProtocol`/`SetReportProtocol`, `ReleaseAllChannels`.

API `AppleBluetoothHIDDevice` d'`IOBluetooth.framework` (2009 et 10.7) **[chaîne + désassemblage]** :
`recantConnection` (pose `SuppressDisconnectNotifications` puis SET `0x41` ; la classe générique non-Apple envoie à la
place HID_CONTROL `VIRTUAL_CABLE_UNPLUG`), `factoryDefault` (SET `0x45`), `fullFactoryDefault` (SET `0x44`),
`deleteAllLinkKeys` (2009 : SET Feature `0x44` d'un octet, délai 1000 ms — même registre que `FullFactoryDefault`),
`userMode`/`setUserMode:` (`0x43`, absent de notre firmware), `deviceNameFromHardware` (GET `0x51-0x54` concaténés),
`batteryLow`/`batteryDangerouslyLow` (lisent les propriétés `BatteryLow`/`BatteryPanic`), `connectionCounts:`
(GET Feature `0x4E`, 10 o — réservé au PID `0x0310` par `blued`, sans objet ici), `connectToHost:linkKey:`,
`removeCurrentHost`, `handoffAndRemoveHost:…` (**vides** en 10.7 : renvoient 0).

**Jamais envoyé à notre PID, quelle que soit l'époque** [désassemblage] : SET `0x09` (réservé `0x022C-0x022E`, fw ≥ `0x0137`),
SET `0x45` (`FactoryDefault`), SET `0x43` (`UserMode`), tout ID `0xD0-0xDF` ou `0xF0-0xFF` sur le canal HID.

## 6. L'updater 2009 (`bfu` + `config.hex`) : structure publique uniquement

Contenu de `WirelessKybdFirmwareUpdate.pkg` **[chaîne/plist]** : application « 2009 Aluminum Wireless Keyboard Firmware
Update », `Parameters.plist` (`FWVersion = 80`, `PageTimeout = 32768` slots ≈ 20,5 s, `AckRecords = true`,
`CopyAddressToPasteboard = true`, `LoggingOn = false`), et `/Library/Application Support/Apple/WLKBFU/2/` :
`bfu` (Mach-O i386 + ppc7400, 64 196 o) et `config.hex` (35 296 o). `bfu` connaît aussi `WLKBFU/1/` et `WLMMFU/3/`
(clavier précédent, Mighty Mouse) **[chaîne]**.

### 6.1 `config.hex`

| Propriété | Valeur | Preuve |
|---|---|---|
| Taille | 35 296 o = 2 206 × 16 | [mesuré] |
| Entropie | 7,98 bit/octet, 256 valeurs distinctes, aucun texte | [mesuré] |
| Traitement | lu par `dataWithContentsOfFile:`, passé à `decryptFWData:` → fonction `FWDecrypt`, puis découpé en enregistrements texte (séparateur `":\n"`, lignes marquées `>` et `#` traitées à part, paires hexadécimales → octets, type d'enregistrement en octet 3 avec `1` = fin) par `convertRecords:` | [désassemblage] |
| Nature | **chiffré** (taille multiple de 16 → chiffrement par blocs **[déduction]**) ; le clair est un fichier d'enregistrements de type Intel-HEX **[déduction]** | |

**Arrêt volontaire** : la clé et l'algorithme de `FWDecrypt` sont internes à `bfu` et non publiés. Conformément au cadre,
**ils n'ont pas été examinés ni reconstruits**, et `config.hex` **n'a pas été déchiffré**. Le contenu de l'image n'est
donc pas décrit. Le nom (`config`) et la présence de deux chemins `updateFW` / `updateConfigData` (+ `fwBank1BaseAddr`,
`fwFSOffSet`, `firstConfigRecordNum`) suggèrent que la mise à jour 2009 réécrit la **zone de configuration/correctifs**
(mémoire externe du BCM2042), pas la ROM masquée **[déduction]**.

### 6.2 Protocole de transfert

| Étape | Constat | Preuve |
|---|---|---|
| Isolement | `IOBluetoothIgnoreHIDDevice`, puis commande noyau `ReleaseAllChannelsWithSleepForHIDUpdate` : le pilote ferme les deux canaux HID (1,3 s d'attente après chacun) | [désassemblage] des deux côtés |
| Lien | connexion baseband (`PageTimeout`), politique de lien `0x000B` pendant la mise à jour (sniff interdit), `0x000F` restaurée ensuite ; types de paquets `0x0408` | [désassemblage] |
| **Canal** | **L2CAP, PSM `0xF30D`** (PSM dynamique vendeur), **pas** les PSM HID `0x11`/`0x13` (ré-autorisés en fin de mise à jour) | [désassemblage] `IOBluetoothDeviceOpenL2CAPChannelSync(…, 0xF30D, …)` |
| Trame de commande | octets bruts `[commande][longueur][paramètres]`, **sans en-tête HIDP**, accusé attendu sous 10 s | [désassemblage] `sendCommand:withAck:param:pLength:` |
| Séquence | `D1` (1 o) → acquit `D2` ; `D4` → `D5` ; `D6` (1 o) → `D7` ; puis enregistrements bruts, avec `DA` (1 o) → `DB` pour l'acquit des blocs (attente 20 s) ; `D3` = abandon (acquit `D3`) ; tout acquit inattendu → reprise | [désassemblage] `ackReceived:` (table de sauts sur `0xD2-0xDC`) |
| Fin | `##100##` sur la sortie standard (progression `##%03d##`), retour des PSM `0x11`/`0x13` | [chaîne + désassemblage] |

**Conséquences [déduction]**

* Le canal de mise à jour du A1314 est un **canal L2CAP vendeur séparé**, ouvert par l'hôte vers le PSM `0xF30D`
  — **pas** une suite de SET_REPORT HID. Cela **corrige** `RE-COMMANDES-VENDEUR.md` §4.3, qui présentait les registres
  HID write-only comme « candidats naturels » du canal de flash.
* Les opcodes du protocole occupent la plage **`0xD1-0xDB`**. Nos IDs Feature HID `0xD0`, `0xD4`, `0xD5` (refus GET `0x03`)
  et `0xD1`, `0xD8` (lisibles, 0) tombent dans la **même plage** : le micrologiciel range vraisemblablement ses fonctions
  de maintenance sous un même préfixe `0xDx`. **Aucune** preuve que les IDs HID `0xDx` déclenchent ces commandes ;
  la coïncidence suffit à les classer **« ne jamais écrire »**.
* Aucun updater ne vise `0x0255-0x0257` (§2) : il n'y a **aucune** raison légitime d'ouvrir le PSM `0xF30D` vers notre
  clavier. Aucun outil ne sera écrit pour cela.

## 7. Table finale : registre → fonction → preuve → risque (état après 2009, 10.7.5 et 26.5)

| ID | Type | Fonction | Preuve | Risque | Usage projet |
|---|---|---|---|---|---|
| `0x01` | Out | LED Verr. Maj | [désassemblage] 2009/10.7/26 | nul | déjà géré par le noyau |
| `0x09` | Feature | drapeau « délai Verr. Maj du firmware » (`01` = désactivé) ; écrit **seulement** pour `0x022C-0x022E` | [désassemblage] 10.7 = 26.5 | faible | lecture ; ne pas écrire |
| `0x13` | In | bit 1 = sous tension (0 → `KeyboardOff`) | [désassemblage] 10.7 = 26.5 | nul | #190 |
| `0x30` | In | `BatteryState` 0/1/2 (+3 accepté) | [plist + désassemblage] toutes époques | nul | #189 |
| `0x40` | Feature WO | `WillShutdown` | [plist + désassemblage] envoyé à chaque arrêt (10.7, 26.5) | faible | #191 |
| `0x41` | Feature WO | `RecantConnection` = « virtual cable unplug » version Apple | [désassemblage] `recantConnection` ; `blued` ne l'utilise que pour l'émulation HID | élevé (coupe/oublie l'hôte) | ne pas exposer |
| `0x43` | Feature | `UserMode` | absent du firmware `0x0050` [mesuré] | — | aucun |
| `0x44` | Feature WO | `FullFactoryDefault` = efface **toutes** les clés de lien (`deleteAllLinkKeys` en 2009) | [désassemblage] **envoyé par Lion à la suppression du clavier** | **élevé mais voulu** : le clavier oublie tous ses hôtes, ré-appairage obligatoire | « oublier proprement », accord du gérant (issue dédiée) |
| `0x45` | Feature WO | `FactoryDefault` | [plist + désassemblage] jamais appelé par aucun client Apple examiné | élevé, effet exact inconnu | ne pas écrire |
| `0x46` | Feature | tension instantanée mV (LE) | [mesuré] ; jamais lue par Apple | lecture | ≤ 1/5 min (#177) |
| `0x47` | Feature | `BatteryPercent` (calculé par le firmware) | toutes époques | lecture | en production |
| `0x49` | Feature | **`BatteryVoltage`** = tension `Latched` mV (LE) | [plist + désassemblage 10.7] | lecture | rythme Apple : 1/4 h (#139, nouvelle issue) |
| `0x4A` | Feature | état / notification de **lien SCO** : écrit `03`/`04` par `blued` (Lion) ; GET = `0x12` (codage inconnu) | [désassemblage] | faible (Apple l'écrivait en production) | issue « coexistence casque », accord requis |
| `0x4B` | Feature | inconnu (`00 08`) | — | — | lecture rare |
| `0x4C` | Feature | adresse de l'hôte appairé + 12 o | [mesuré] | lecture sensible | #140 |
| `0x4E` | Feature | `connectionCounts` (10 o) des produits `0x0310` | [désassemblage] | — | absent chez nous |
| `0x4F` | Feature | version firmware `0x0050` | [mesuré] | lecture | — |
| `0x50` | Feature WO | `DeviceNameChange` (validation des 4 fragments) | [désassemblage] `setDeviceName:` | moyen | #192 |
| `0x51-0x54` | Feature | `DeviceName1..4` (lus par `deviceNameFromHardware`) | [désassemblage] | moyen en écriture | #192 |
| `0x55` | Feature WO | `LongDeviceName` 64 o : **seul** registre écrit par Lion pour renommer un 598 | [désassemblage] | moyen | #192 |
| `0x5A`, `0xEB` | Feature | copies des seuils de `0x60` (jeux 1 et 2 ?) | [mesuré + déduction] | lecture | intégrité (`calib_mirror_*`) |
| `0x5B` | Feature | `0xF4` ‖ `0xF5` ‖ zéros | [mesuré] | — | — |
| `0x5C`, `0x5D` | Feature | vides | [mesuré] | — | — |
| `0x60` | Feature | **`CalibratedBatteryThresholds3`** : `Full`/`Low`/`Critical`/`Empty` mV (4 × u16 BE) | [plist + désassemblage 10.7] | lecture | nouvelle issue |
| `0xD0`, `0xD4`, `0xD5` | Feature WO | **inconnus** ; même plage que les opcodes de maintenance `0xD1-0xDB` de l'updater (PSM `0xF30D`) | [désassemblage bfu + déduction] | **inconnu, potentiellement maintenance** | **ne jamais écrire** |
| `0xD1`, `0xD8` | Feature | inconnus (0) ; même plage `0xDx` | [mesuré] | lecture rare | — |
| `0xEA` | Feature | second estimateur de % (hypothèse) ; jamais lu par Apple | [mesuré] | lecture | remplacer par `0x47` (#177) |
| `0xF4`-`0xF7` | Feature | constantes (`0xF5` = 900 : délai de veille ?) ; jamais lus par Apple | [mesuré] | lecture rare | #173 |
| `0xFA`, `0xFB` | Feature WO | **inconnus de tout logiciel Apple examiné** (2009, 10.7, 26.5, `bfu`) | — | **inconnu** | **ne jamais écrire** |
| `0xFE` | Feature | fige le firmware à la lecture | [mesuré] #175 | **lecture dangereuse** | exclu |
| `0xFF` | Feature | tension (BE) + `01` | [mesuré] | lecture | — |
| `0x04`, `0x05` | In (non déclarés) | inconnus d'Apple ; hypothèses WICED SLEEP / FUNC_LOCK | [source tierce] | nul (lecture passive) | — |

Les inconnues **définitivement sans réponse logicielle publique** sont donc `0xD0 0xD4 0xD5 0xFA 0xFB` (écriture),
`0x4B 0xD1 0xD8 0xF6 0xF7` (lecture) et les Input `0x04`/`0x05` : **aucun** des quatre logiciels Apple examinés ne les
mentionne. Seul un dump de la mémoire externe (#184/#185) peut encore les éclairer.

## 8. Fonctions Apple réellement supportées par l'A1314 B (`0x0256`, fw `0x0050`)

Par opposition aux fonctions du Magic Keyboard (`0x90`, `0x35`, rétro-éclairage `0xB0`, `0xF0`/`0xF2`, Lightning) :

1. **Batterie** : pourcentage firmware (`0x47`), état bas/critique poussé (`0x30`), tension `Latched` (`0x49`), seuils
   `Full`/`Low`/`Critical`/`Empty` (`0x60`), notifications `LowBattery`/`CriticallyLowBattery`, relevé toutes les 4 h.
2. **Extinction détectée** (`0x13` bit 1) → notification `KeyboardOff`.
3. **Arrêt de l'hôte annoncé** (`0x40` `WillShutdown`).
4. **Veille de l'hôte** : HID_CONTROL SUSPEND / EXIT_SUSPEND.
5. **Nom stocké dans le clavier** : lecture `0x51-0x54`, écriture `0x55` (64 o) — fonction « Renommer » de Lion.
6. **Oubli de tous les hôtes** (`0x44`) — fonction « Supprimer » de Lion ; `0x41` « renoncer à la connexion ».
7. **Coexistence avec un casque** : notification de lien SCO (`0x4A` = `03`/`04`) + paramètres de sniff adaptés côté hôte.
8. **Touches** : Fn = `0x00FF:0x0003`, F1-F12 remappées **par l'hôte** (F4 = Launchpad sur le 598), délai Verr. Maj
   de 75 ms appliqué **par l'hôte**, émulation pavé numérique par l'hôte.
9. **Pas** de mise à jour de firmware publique pour ce PID ; **pas** de `UserMode` ; **pas** de rétro-éclairage.

## 9. Corrections à apporter à nos documents (non modifiés ici, voir issues)

| Document | Affirmation actuelle | Correction | Preuve |
|---|---|---|---|
| `HARDWARE-RAPPORTS-HID.md` §2 et §3 | `0x5A` = « seuils 100/75/50/25 % » | `0x60` (et ses copies) = seuils **`Full`/`Low`/`Critical`/`Empty`** ; `Low`/`Critical` ↔ `BatteryState` 1/2 | §4 |
| `HARDWARE-RAPPORTS-HID.md` | `0x49` « tension lissée (hypothèse) » | `BatteryVoltage`, tension **`Latched`** pour Apple | §3-4 |
| `RE-PILOTE-MACOS.md` §6 | « hypothèse MVLT réfutée » | réfutée **pour 26.5 seulement** : Lion publiait `MV{LT}` = `0x49` | §4 |
| `RE-PILOTE-MACOS.md` §9 n° 6 | effet du bit 3 inconnu | bit 3 = appareils notifiés du lien SCO (`0x4A`) et sniff ajusté | §5 |
| `RE-PILOTE-MACOS.md` §5.1 | `0x09`… « jamais envoyé » | confirmé pour 2009 et 10.7 aussi | §5 |
| `RE-COMMANDES-VENDEUR.md` §4.3 | registres HID WO = « candidats naturels » du canal de flash ; `bfu` passe par le canal de contrôle HIDP | le flash passe par un **PSM L2CAP vendeur `0xF30D`**, trames brutes `D1…DB`, pas par HIDP | §6 |
| `RE-COMMANDES-VENDEUR.md` §4.3 | `bfu` est « le canal officiel du A1314 » / cible notre `0x0050` | `bfu` vise **`0x0239-0x023B`** (fw `0x44`/`0x46` → `0x50`) ; **aucun** updater pour `0x0255-0x0257` | §2 |
| `RE-COMMANDES-VENDEUR.md` §5 / `RE-PILOTE-MACOS.md` §8 | `0x44` « INTERDIT » | `0x44` est l'**oubli de tous les hôtes**, envoyé par Lion lors de « Supprimer » : destructif pour l'appairage mais **fonction Apple documentée** ; reste derrière accord explicite | §5 |
| `RE-COMMANDES-VENDEUR.md` §1.3 | `0xFA`/`0xFB` « porte d'écriture du délai de veille » (spéculation) | aucune trace dans 4 logiciels Apple ; spéculation non étayée, classer « inconnu » | §7 |

## 10. Reproduire (lecture seule, sans clavier)

1. Catalogue `index-lion-snowleopard-leopard.merged-1.sucatalog` → URL `swcdn.apple.com` des paquets (§1).
2. Lecteur xar : vérifier la somme SHA-1 de la TOC et de chaque fichier ; `Payload` = cpio `odc` gzip.
   Pour le Combo 10.7.5 (2 Go), lire le `Payload` en flux (`curl -r` → `gzip -dc` → extracteur cpio filtrant) :
   rien n'est stocké hormis les composants Bluetooth/HID.
3. `llvm-lipo -thin x86_64|i386`, `llvm-objdump --macho -d` (les sélecteurs Objective-C et les chaînes sont annotés),
   `llvm-nm -C` (les kexts gardent leurs symboles C++ ; `blued` et `bfu` sont dépouillés : passer par les références
   de sélecteurs et la table `--objc-meta-data`).
4. Points d'entrée : `AppleBluetoothHIDKeyboard::{updateBatteryLevel,getLatchedBatteryVoltage,getVoltagesUsed,processInterruptData}`,
   `IOAppleBluetoothHIDDriver::{processCommandWL,handleStart}`, `-[AppleBluetoothHIDDevice setDeviceName:|fullFactoryDefault|sendSCOLink*]`,
   chaîne `sending sendSCOLinkACTIVE` dans `blued`, `kRemoveDevice` dans `Bluetooth.prefPane`, `sendCommand:withAck:param:pLength:` et
   `ackReceived:` dans `bfu`.
5. **Ne pas** analyser `FWDecrypt` ni tenter de déchiffrer `config.hex`.
