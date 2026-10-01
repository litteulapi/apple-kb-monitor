# Rétro-ingénierie du pilote macOS — ce que macOS envoie à l'A1314 (BCM2042)

*Document en cours de rédaction, mis à jour au fil de l'analyse.*

Analyse **statique, en lecture seule**, des pilotes Apple installés sur le Mac du gérant (Neo01, Mac17,5, macOS 26.5 build 25F71).
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
| `BatteryState` | **`0x30`** | **Input** | 1 | 0-2 | Input non déclaré, `30 00` | **identifié** : état batterie 0/1/2 [plist] ; sens des valeurs : §5 |
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

## 4. Ce que macOS envoie au clavier (en cours)

*Section alimentée par le désassemblage (à venir).*

## 5. Calcul et état de la batterie (en cours)

## 6. IDs encore inconnus

## 7. Corrections à apporter à nos documents
