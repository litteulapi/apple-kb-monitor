# Rétro-ingénierie du pilote macOS — ce que macOS envoie à l'A1314 (BCM2042)

Analyse **statique, en lecture seule**, des pilotes Apple installés sur le Mac du gérant (non reproductible depuis le poste Linux : aucun binaire ni extrait commité ; les faits [plist]/[désassemblage] n'ont pas pu être recontrôlés au contre-audit) (Neo01, Mac17,5, macOS 26.5 build 25F71).
Elle sert l'interopérabilité avec **son** clavier Apple Wireless Keyboard A1314 ISO (VID `0x05AC`, PID `0x0256` = 598).

* Aucun binaire Apple ni code décompilé n'est commité : seulement des faits courts (IDs, tailles, noms de clés et de symboles).
* Aucun accès au clavier réel, aucune écriture HID.
* Aucun secret (clé de lien, IRK, trousseau) n'a été recherché ni lu.

Niveaux de preuve :

* **[plist]** : clé lue dans un `Info.plist` / `__PRELINK_INFO` livré par Apple ;
* **[désassemblage]** : constaté dans le code machine ;
* **[chaîne]** : chaîne de caractères présente dans le binaire ;
* **[déduction]** : raisonnement à partir des éléments précédents ;
* **[mesuré]** : renvoie à nos mesures sur le clavier (docs `RE-HID-EXHAUSTIF.md`, `HARDWARE-RAPPORTS-HID.md`).

## 1. Sources examinées

| Élément | Emplacement | Remarque |
|---|---|---|
| `IOBluetoothHIDDriver.kext` 9.0.0 | `/System/Library/Extensions` (enveloppe), code dans `BootKernelExtensions.kc` | classe générique BT HID |
| `AppleHIDKeyboard.kext` 9410.2 | idem | classe `AppleEmbeddedKeyboard` (couche IOHID, Fn, cartes de touches) |
| **`AppleHIDKeyboard.kext/Contents/PlugIns/AppleBluetoothHIDKeyboard.kext`** 9410.2 | présent **uniquement** dans le `__PRELINK_INFO` du KC | **classe `AppleBluetoothHIDKeyboard`, celle qui pilote l'A1314 côté Bluetooth** |
| `AppleBluetoothMultitouch.kext` 106 | idem | comparaison (Magic Trackpad/Mouse) |
| `IOHIDPowerSource.kext` | `SystemKernelExtensions.kc` | traduction HID → source d'alimentation |
| `/usr/sbin/bluetoothd` | binaire autonome | démon espace utilisateur |

Les deux collections de noyau présentes dans `/System/Library/KernelCollections` sont des **Mach-O `MH_FILESET` x86_64**
(`cputype 0x01000007`) : ce sont les collections destinées aux Mac Intel, livrées aussi sur Apple Silicon. Le code arm64e
réellement exécuté se trouve dans le kernelcache signé de la partition Preboot, non examiné. Les personnalités IOKit
et la logique des pilotes sont communes aux deux architectures **[déduction]**.

## 2. Appariement du clavier (A1314 ISO, PID 598)

Deux personnalités IOKit correspondent à `VendorID 1452 / ProductID 598` **[plist]** :

### 2.1 Couche Bluetooth : `AppleBluetoothHIDKeyboard` (« Wireless Keyboard 2009 B ISO »)

| Clé | Valeur | Sens |
|---|---|---|
| `IOClass` | `AppleBluetoothHIDKeyboard` | sous-classe de `IOBluetoothHIDDriver` [déduction] |
| `IOProviderClass` / `PSM` | `IOBluetoothL2CAPChannel` / 17 | canal de contrôle HID |
| `GetReportTimeoutMS` / `SetReportTimeoutMS` | **3500** / **3500** | délai d'attente GET/SET_REPORT ; BlueZ attend 3 s, le noyau Linux 5 s |
| `PoweredOffNotificationType` | `KeyboardOff` | notification « clavier éteint » |
| `BatteryLowNotificationType` / `BatteryDangerouslyLowNotificationType` | `LowBattery` / `CriticallyLowBattery` | deux niveaux d'alerte |
| `DebuggingOn` | `false` | journalisation interne |
| **`ExtendedFeatures`** | voir §3 | **carte des rapports vendeur** |

### 2.2 Couche IOHID : `AppleEmbeddedKeyboard` (« Wireless Keyboard 2009 B ISO Map »)

| Clé | Valeur | Sens |
|---|---|---|
| `IOProviderClass` | `IOHIDInterface` | |
| `FnModifierUsagePage` / `FnModifierUsage` | **`0xFF` / `0x03`** | la touche Fn est l'usage vendeur `0x00FF:0x0003` du descripteur |
| `StandardType` | 1 | ISO (0 = ANSI, 2 = JIS) |
| `alt_handler_id` | 44 | ISO |
| `CapsLockDelay` | 75 | ms d'appui avant de basculer Verr. Maj (anti-effleurement), **côté hôte** |
| `FnFunctionUsageMap` | F1→`0xFF01:0x21`, F2→`0xFF01:0x20`, F3→`0xFF01:0x10`, F4→`0xFF01:0x04`, F7→`0x0C:0xB4`, F8→`0x0C:0xCD`, F9→`0x0C:0xB3`, F10→`0x0C:0xE2`, F11→`0x0C:0xEA`, F12→`0x0C:0xE9` | **remappage Fn fait par macOS**, pas par le clavier |
| `FnKeyboardUsageMap` | flèches→Origine/Fin/PgPréc/PgSuiv, Retour arrière→Suppr, Entrée→Entrée pavé | idem |
| `NumLockKeyboardUsageMap` | émulation du pavé numérique | idem |

**Conséquence [déduction, plist]** : le mode « F1-F12 standard / touches spéciales » et la touche Fn sont **entièrement gérés par
macOS** à partir de l'usage `0xFF:0x03`. Rien n'indique que macOS envoie un mode Fn au clavier : notre approche Linux
(`hid-apple fnmode`, keyd) est équivalente.

## 3. Carte `ExtendedFeatures` : les rapports vendeur selon Apple

`ExtendedFeatures` est un dictionnaire `nom → {id, type, size, min, max}` déclaré par Apple pour la personnalité 598 **[plist]**.
`type` suit `IOHIDReportType` : 0 = Input, 1 = Output, 2 = Feature.

| Nom Apple | ID | Type | Taille | Bornes | Notre mesure (GET) | Conclusion |
|---|---|---|---|---|---|---|
| `BatteryState` | **`0x30`** | **Input** | 1 | 0-2 | Input non déclaré, `30 00` | **identifié** : état batterie 0/1/2 [plist] ; sens des valeurs : §6 |
| `WillShutdown` | **`0x40`** | Feature | — | — | ERR_UNSUPPORTED_REQUEST | **identifié** : commande « l'hôte va s'éteindre » (écriture seule) [plist] |
| `RecantConnection` | **`0x41`** | Feature | — | — | ERR_UNSUPPORTED_REQUEST | **identifié** : commande « renoncer à la connexion » (écriture seule) [plist] |
| `UserMode` | `0x43` | Feature | 1 | 1-3 | ERR_INVALID_REPORT_ID | déclaré par Apple, **absent du micrologiciel de notre A1314** [mesuré] |
| `FullFactoryDefault` | **`0x44`** | Feature | — | — | ERR_UNSUPPORTED_REQUEST | **identifié** : réinitialisation usine complète — **DANGER** |
| `FactoryDefault` | **`0x45`** | Feature | — | — | ERR_UNSUPPORTED_REQUEST | **identifié** : réinitialisation usine — **DANGER** |
| `BatteryPercent` | `0x47` | Feature | 1 | 0-100 | DATA | confirmé : % batterie lu par macOS en Feature [plist] |
| `DeviceNameChange` | **`0x50`** | Feature | — | — | ERR_UNSUPPORTED_REQUEST | **identifié** : commande de validation du changement de nom |
| `DeviceName1..4` | `0x51-0x54` | Feature | 8 chacun | — | DATA | confirmé : nom en 4 × 8 octets |
| `LongDeviceName` | **`0x55`** | Feature | 64 | — | ERR_UNSUPPORTED_REQUEST | **identifié** : nom long (64 octets) ; écriture seule sur notre clavier |

Les trackpads/souris Magic (2009-2011) ont la même carte plus `SuperMode` (`0xD7`, Feature, 0-1) et
`PowerFeaturePollRateSec = 14400` (relevé batterie toutes les 4 h) **[plist]**.

**Six des onze IDs « sans GET » sont ainsi identifiés** : `0x40 0x41 0x44 0x45 0x50 0x55`.
L'Input non déclaré `0x30` est `BatteryState`. Restent inconnus : `0xD0 0xD4 0xD5 0xFA 0xFB` et les Input `0x04`/`0x05`.


`type` absent = Feature par défaut (`getExtendedReport`/`setExtendedReport` prennent 2 si la clé manque) **[désassemblage]**.

## 4. Architecture du pilote (classes et chemins)

| Classe (kext) | Rôle pour l'A1314 | Méthodes notables |
|---|---|---|
| `IOBluetoothHIDDriver` (IOBluetoothHIDDriver.kext) | « shim » HID : construit les transactions HIDP, délègue l'émission L2CAP à `bluetoothd` (journaux `-- HIDShim --`) | `deviceReady`, `getReportWL`, `setReportWL`, `hidControl`, `setProtocol`, `setIdle`, `handleSleep/Wake` |
| `IOAppleBluetoothHIDDriver` (même kext) | couche « appareils Apple » : `ExtendedFeatures`, batterie, arrêt | `getExtendedReport`, `setExtendedReport`, `updateBatteryLevel`, `getBatteryState`, `updateBatteryState`, `willShutdown`, `setCapsLock`, `processCommandWL` |
| **`AppleBluetoothHIDKeyboard`** (plug-in d'AppleHIDKeyboard) | sous-classe pour les claviers 2009 (PID 569-571, 597-599) | `init`, `handleReport` (trace si `DebuggingOn`), `processInterruptData` (détection d'extinction) |
| `AppleHIDKeyboardEventDriver(V2)` (AppleHIDKeyboard.kext) | couche événements : Fn, cartes de touches, délai Verr. Maj | `turnOffCapsLockDelay`, `setFeatureReport`, `handleStart` |
| `bluetoothd` (`BT::HIDProfile`) | pile Bluetooth en espace utilisateur : L2CAP, SUSPEND/EXIT_SUSPEND, politique de sniff | `prepareForSleep`, `prepareForWake` |

Les noms de méthodes viennent de la table des symboles des kexts, présente dans le KC **[désassemblage]**.

## 5. Ce que macOS envoie à l'A1314 (PID 598)

Chronologie complète de ce que le pilote noyau et `bluetoothd` émettent vers **ce** clavier.
Les octets « fil » suivent HIDP (en-tête de transaction puis charge utile).

| # | Quand | Transaction | Octets émis | Preuve |
|---|---|---|---|---|
| E1 | connexion, `deviceReady` | **SET_PROTOCOL(Report)** sur le canal de contrôle, délai 10 s | `71` | [désassemblage] `setProtocol(1)` sauf si `SuppressSetProtocol` (absent pour 598) |
| E2 | 60 s après la connexion, puis toutes les **4 h** (1 h après un échec) | **GET_REPORT Feature `0x47`** (`BatteryPercent`), tampon de 2 octets, vérifie `buf[0] == 0x47` | `43 47` | [désassemblage] `startBatteryUpdate` (60 000 ms), `deviceReady` (14 400 000 ms), `batteryLevelTimerFired` (3 600 000 ms) |
| E3 | juste après chaque E2 réussi | **GET_REPORT Input `0x30`** (`BatteryState`), tampon de 2 octets | `41 30` | [désassemblage] `getBatteryState` |
| E4 | rapport d'interruption `A1 30 xx` reçu du clavier | aucun envoi direct ; si l'état change, relit `0x47` (E2) | `43 47` | [désassemblage] `processInterruptData` → `updateBatteryState` |
| E5 | **veille du système** | **HID_CONTROL SUSPEND** (envoyé par `bluetoothd` aux appareils HID Apple) ; arrêt du relevé batterie | `13` | [chaîne] `Sending SUSPEND command…as system is going to sleep` + [désassemblage] condition « appareil Apple » (bit 0 du type, §7) ; `handleSleep` → `stopBatteryUpdate` |
| E6 | **réveil du système** | **HID_CONTROL EXIT_SUSPEND** ; relance le relevé (E2 60 s plus tard) | `14` | [chaîne] `prepareForWake() -- Sending EXIT_SUSPEND` ; [désassemblage] `handleWake` → `startBatteryUpdate` |
| E7 | **arrêt ou redémarrage de l'hôte** | **SET_REPORT Feature `0x40`** (`WillShutdown`), charge utile vide | `53 40` | [désassemblage] `handleShutdown`/`handleRestart` → `willShutdown` → `setExtendedReport("WillShutdown", NULL, 0)` : tampon d'1 octet = l'ID seul, car la personnalité 598 n'a pas de clé `value` |
| E8 | commande `CapsLock` reçue d'espace utilisateur | DATA Output `0x01` sur le canal d'**interruption** | `A2 01 02` (allumée) / `A2 01 00` (éteinte) | [désassemblage] `setCapsLock` ; le reste du temps, la LED suit la pile IOHID générique (même rapport `0x01`) [déduction] |

Format des requêtes, établi par le désassemblage de `getReportWL` :

* **GET_REPORT** = `0x40 | (type + 1)` suivi de l'ID, soit **2 octets, sans bit Size ni BufferSize**. C'est exactement ce que fait BlueZ ;
* type IOHID 0 = Input → `0x41`, 2 = Feature → `0x43`.

### 5.1 Ce que macOS **n'envoie pas** à l'A1314

| Envoi absent | Preuve |
|---|---|
| `setCapsLockDelay`, SET Feature `0x09` = `09 01 00 00` | [désassemblage] `IOAppleBluetoothHIDDriver::handleStart` le réserve aux PID `0x022C-0x022E` (claviers 2007) dont le micrologiciel est ≥ `0x0137`. `AppleHIDKeyboardEventDriver::turnOffCapsLockDelay` (SET Feature `0x09`, données `01 00 00`) suit la même condition et exige `FWCapsLockDelay`, absent de la personnalité 598 |
| Mode Fn, F1-F12, Verr. Maj | entièrement traités côté hôte (§2.2) ; le délai Verr. Maj de 75 ms est appliqué **par macOS** (`CapsLockDelay` = 75) |
| Écriture du nom (`0x50-0x55`), `FactoryDefault`, `FullFactoryDefault`, `RecantConnection`, `UserMode` | déclarés dans le plist, mais **aucun appel** dans le noyau. Seuls `BatteryPercent`, `BatteryState` et `WillShutdown` sont référencés [désassemblage]. Ces noms sont aussi **absents** des chaînes de `bluetoothd` [chaîne]. IOBluetooth.framework (dans le cache dyld) n'a pas été examiné |
| « Clear Wake Reason » : SET Feature `0xF0`, données `C5 00` | [désassemblage + émulation de la table des types] réservé aux appareils de « bit 2 » (PID `0x0265`, `0x0267`, `0x0269`, `0x026C`, `0x029A`-`0x029F`, `0x0320`-`0x0324` : Magic Keyboard/Trackpad/Mouse de 2015 et après) |
| SET Feature `0xF2` = `21 00` avant la veille | même chemin, réservé aux trackpads (bit 7 : `0x0265`, `0x030E`, `0x0324`) |
| GET de `0x46`, `0x49`, `0x4A`-`0x4C`, `0x4F`, `0x5A`-`0x5D`, `0x60`, `0xD1`, `0xD8`, `0xEA`, `0xEB`, `0xF4`-`0xF7`, `0xFE`, `0xFF` | aucune référence dans le noyau ni dans `bluetoothd` : **macOS ne lit jamais ces rapports** [désassemblage] |

## 6. Batterie : ce que fait macOS

* **Pourcentage** : `updateBatteryLevel` lit l'octet 1 de la Feature `0x47`, le **borne à 100** (`min(v, 100)`), puis le publie tel quel
  (propriété IORegistry `BatteryPercent`). Une valeur forcée (`ForceBatteryPercent`) peut le remplacer à des fins de test.
  **Aucune conversion tension → % n'est faite par macOS** : le pourcentage est entièrement calculé par le micrologiciel [désassemblage].
  L'hypothèse « macOS tire le % d'une tension en mV » (RE-HID-EXHAUSTIF §3) est donc **réfutée pour l'A1314** (pilote 2026).
  *Contre-audit* : le bloc `"Battery" = <"MVLT…` relevé en 2014 (managingosx, revérifié) existe bien ; son origine
  (autre version du pilote, autre rapport) reste **inconnue** : seule la conversion mV → % est réfutée.
* **État** : Input `0x30`, lu par GET après chaque relevé et **poussé spontanément** par le clavier (`A1 30 xx`) [désassemblage].

  | `xx` | `BatteryLow` | `BatteryPanic` | Message IOKit | Notification |
  |---|---|---|---|---|
  | 0 | faux | faux | `'rbsn'` | aucune (retour à la normale) |
  | 1 | vrai | faux | `'btlw'` | `LowBattery` |
  | 2 ou 3 | vrai | vrai | `'btpn'` | `CriticallyLowBattery` |
  | autre | — | — | erreur `0xE00002BC` | — |

  Le plist borne `BatteryState` à 0-2 ; le code accepte aussi 3 **[désassemblage]**. Notre lecture `30 00` signifie « normal ».
  Les seuils (tension ou %) auxquels le clavier passe à 1 ou 2 sont **décidés par le micrologiciel** et restent à mesurer.
* **Cadence** : premier relevé 60 s après la connexion, puis toutes les 4 h, 1 h après un échec. Le relevé est suspendu pendant la veille.
  La propriété `BatteryUpdateInterval` (en s) permet de changer la cadence **[désassemblage]**.
  À comparer avec UPower/noyau Linux, qui interroge toutes les 30 s (#146).
* **Délais** : `GetReportTimeoutMS` et `SetReportTimeoutMS` valent 3500 pour le 598 ; la valeur par défaut est de 5000 ms.

## 7. Extinction et classement de l'appareil

* **Extinction (bouton)** : `AppleBluetoothHIDKeyboard::processInterruptData` reconnaît un rapport d'interruption de 3 octets `A1 13 xx`.
  Si **le bit 1 de `xx` vaut 0** et que `DisablePoweredOffCheck` est faux (cas du 598), macOS envoie la notification `KeyboardOff`,
  message `'bsk2'` **[désassemblage]**. L'Input `0x13` (vendeur `FF01:0A`/`0C`) porte donc un bit « clavier sous tension ».
* **`bluetoothd`** classe chaque appareil HID dans un octet de drapeaux, à partir d'une table interne (VID source, VID, liste de PID).
  La table a été reconstruite par émulation de son initialiseur **[désassemblage + émulation]** :
  * pour l'A1314 (`0x0256`) : **bit 0** (tout appareil Apple, `0x004C`/`0x05AC`) et **bit 3** (famille ancienne : `0x0208`-`0x020A`,
    `0x022C`-`0x022E`, `0x0239`-`0x023B`, `0x0255`-`0x0257`, `0x0309`, `0x030C`) ;
  * le bit 0 conditionne l'envoi de SUSPEND à la veille ;
  * le bit 3 intervient dans le choix des paramètres de liaison (sniff). Son effet exact n'a pas été établi **[désassemblage partiel]**.

## 8. Croisement avec nos rapports

| Rapport | Avant cette étude | Selon macOS | Niveau | Risque d'écriture |
|---|---|---|---|---|
| Feature `0x09` (`FF01:0B`) | constant `09 01 00 00`, sens inconnu | **drapeau « délai Verr. Maj du micrologiciel »** : macOS écrit `01` pour **désactiver** ce délai sur les claviers 2007, puis applique son propre délai de 75 ms. Notre A1314 vaut déjà `01` | [désassemblage] + [déduction] pour le sens de `00` | faible, réversible (W6 de #182 = réactiver le délai interne) |
| Input `0x13` bit 1 | réveil / Fn vendeur | **sous tension** : 0 = le clavier s'éteint | [désassemblage] | lecture passive |
| Input `0x30` | inconnu (`30 00`) | **`BatteryState`** : 0 normal, 1 bas, 2-3 critique ; aussi poussé en interruption | [plist] + [désassemblage] ; confirme #187 | lecture passive |
| Feature `0x40` | refus GET | **`WillShutdown`** : SET sans données, envoyé à chaque arrêt de macOS | [plist] + [désassemblage] | **faible** : macOS l'envoie en production |
| Feature `0x41` | refus GET | **`RecantConnection`** : « renoncer à la connexion » | [plist] ; jamais envoyé | **moyen à élevé** : peut couper la liaison ou faire oublier l'hôte [déduction] |
| Feature `0x43` | `ERR_INVALID_REPORT_ID` | `UserMode` (1-3), **absent** de notre micrologiciel | [plist] + [mesuré] | sans objet |
| Feature `0x44` | refus GET | **`FullFactoryDefault`** | [plist] ; jamais envoyé | **INTERDIT** : remise à zéro complète, perte d'appairage |
| Feature `0x45` | refus GET | **`FactoryDefault`** | [plist] ; jamais envoyé | **INTERDIT** : remise à zéro, perte d'appairage probable |
| Feature `0x47` | % | `BatteryPercent`, recopié tel quel, borné à 100 | [plist] + [désassemblage] | lecture |
| Feature `0x50` | refus GET | **`DeviceNameChange`** : validation après écriture du nom | [plist] ; jamais envoyé | moyen (NVRAM) |
| Feature `0x51`-`0x54` | nom 4 × 8 | `DeviceName1..4`, 8 octets chacun | [plist] | moyen (NVRAM) |
| Feature `0x55` | refus GET ; #188 : « config vendeur 64 o » | **`LongDeviceName`**, 64 octets | [plist] — corrige #188 | moyen (NVRAM) |
| Feature `0xD0`, `0xD4`, `0xD5`, `0xFA`, `0xFB` | refus GET | **inconnus de macOS** (aucune personnalité, aucun code) | — | **élevé** : candidats du canal de mise à jour (#188), ne pas toucher |
| Input `0x04`, `0x05` | `04 00`, `05 02` | **inconnus de macOS** | — | lecture passive seulement (hypothèses WICED de #187 non confirmées ici) |
| Autres Feature (`0x46`, `0x49`, `0x4A`-`0x4C`, `0x4F`, `0x5A`-`0x60`, `0xD1`, `0xD8`, `0xEA`, `0xEB`, `0xF4`-`0xFF`) | voir HARDWARE-RAPPORTS-HID | **jamais lus par macOS** | [désassemblage] | — |

**Enseignement pour les coupures (#175)** : macOS ne lit que `0x47` et `0x30`, au plus une fois toutes les 4 h, avec un délai de 3,5 s.
Les registres que nous balayons (`0xFE` notamment) ne sont jamais sollicités par le pilote d'origine.

## 9. Inconnues restantes

1. `0xD0`, `0xD4`, `0xD5`, `0xFA`, `0xFB` et les Input `0x04`/`0x05` : absents de tout le code examiné.
   Il reste à examiner l'outil de mise à jour Apple (`bfu`, #188) et IOBluetooth.framework (cache dyld).
2. Effet réel de `WillShutdown` sur le clavier (veille immédiate ? pas de tentative de reconnexion ?) : non observable sans écriture.
3. Sens de la valeur `00` dans `0x09` (délai interne actif ?) : déduit des noms Apple, non mesuré.
4. Seuils du micrologiciel pour `BatteryState` 1 et 2.
5. Kernelcache arm64e réellement exécuté sur le Mac : non examiné ; analyse faite sur les KC x86_64 de la même version (25F71).
6. Effet du bit 3 de `bluetoothd` (paramètres de sniff des anciens claviers Apple).

## 10. Reproduire (lecture seule)

1. Sur le Mac : `plutil -convert xml1 -o - /System/Library/Extensions/<kext>/Contents/Info.plist`, puis copier
   `/System/Library/KernelCollections/BootKernelExtensions.kc` et `/usr/sbin/bluetoothd` vers le poste d'analyse (jamais dans un dépôt).
2. KC : Mach-O `MH_FILESET`, entrées `LC_FILESET_ENTRY` (`0x80000035`). La personnalité `AppleBluetoothHIDKeyboard` n'existe que dans `__PRELINK_INFO`
   (XML avec `ID`/`IDREF`). Les symboles de chaque kext (`LC_SYMTAB`) sont présents. Les pointeurs sont en chaînage
   `0xffffff8000100000 + (p & 0x3fffffff)`.
3. Désassembler (capstone x86_64) `IOAppleBluetoothHIDDriver::{getExtendedReport, updateBatteryLevel, updateBatteryState,
   willShutdown, setCapsLock, deviceReady, batteryLevelTimerFired}`, `IOBluetoothHIDDriver::{deviceReady, getReportWL, hidControl}`,
   `AppleBluetoothHIDKeyboard::processInterruptData`.
4. `bluetoothd` (tranche x86_64, sans symboles) : xrefs de la chaîne `HIDProfile::prepareForSleep()`. Les prédicats lisent l'octet `[appareil + 0x310]`,
   rempli par la fonction qui journalise `Updating HID Device Types`. Sa table (VID source, VID, PID, n° de bit) se reconstruit en émulant (unicorn)
   l'initialiseur qui remplit le vecteur à `0x100b51740` (build 25F71).

## 11. Suivi Gitea

| Issue | Objet |
|---|---|
| #189 | F37 — alertes batterie pilotées par le clavier (Input `0x30`, écoute passive) |
| #190 | F38 — détection d'extinction (Input `0x13` bit 1) |
| #191 | F39 — `WillShutdown` (SET Feature `0x40`) à l'arrêt, accord du gérant requis |
| #192 | F40 — nom dans le micrologiciel (`0x50`-`0x55`) |
| #193 | docs — carte des rapports avec les noms Apple |
| #194 | docs — batterie : pas de conversion mV → % côté macOS |
| commentaires | #130, #146, #175, #179, #181, #182, #187, #188 |
