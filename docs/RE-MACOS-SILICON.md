# Rétro-ingénierie macOS Apple Silicon — ce que le code arm64e réellement exécuté ajoute pour l'A1314

* Corrigé le 2026-10-02 (issue #242) : §3.1 (boutisme, méthodes non listées, oubli = `0x41`), §3.2 et §8 (client du % remappé), §4 (34 commandes, `ReleaseAllChannels`, `HIDSuspend`, disjoncteur avec déconnexion), §5 et §7 (`0x4A` = `03` envoyé par `bluetoothd`) ; source : `RE-GHIDRA-IOBLUETOOTH.md`.
* Corrigé le 2026-10-02 (issue #245) : §3.1, §4 et §7 : `HIDExitSuspend` ne met rien sur le fil (pas `0x14`) ; le disjoncteur bloque aussi `sendData` ; un refus HANDSHAKE compte comme réponse ; source : `RE-GHIDRA-KEXT.md` §2.2, §2.4, §2.5.
* Corrigé le 2026-10-02 (issue #239) : `0xD5` n'est plus « inconnu » : commande de test radio PER des anciens HID Apple (`D5 07` / `D5 00`, CoreBluetooth `CBHIDPerformanceMonitor`) ; source : `RE-GHIDRA-IOBLUETOOTH.md` §4.

Complément de [`RE-PILOTE-MACOS.md`](RE-PILOTE-MACOS.md). Cette analyse-là portait sur les collections de noyau **x86_64**,
n'avait pas ouvert **IOBluetooth.framework** (cache dyld) et n'avait lu `bluetoothd` qu'en partie.
Cette étude reprend les **binaires arm64e réellement démarrés** sur le Mac du gérant (Neo01, Mac17,5, macOS 26.5 build 25F71).

Même cadre : analyse statique en lecture seule, à des fins d'interopérabilité avec **son** clavier A1314 ISO
(VID `0x05AC`, PID `0x0256`, micrologiciel `0x0050`).

* Aucun binaire Apple ni code décompilé n'est commité, seulement des faits courts : IDs, tailles, formats, noms de symboles.
* Aucun accès au clavier, aucune écriture HID.
* Aucune clé, aucun secret, aucun trousseau n'a été recherché ni lu. Les fonctions Apple qui manipulent des clés de lien sont seulement **nommées**.
  Sur macOS 26.5, ce sont d'ailleurs des coquilles vides (§3.4).
* Les copies des fichiers Apple ont été supprimées du poste d'analyse à la fin de l'étude.

Niveaux de preuve : **[plist]**, **[désassemblage]**, **[chaîne]**, **[déduction]** et **[mesuré]**, comme dans `RE-PILOTE-MACOS.md`.
Le niveau **[mesuré]** renvoie à `HARDWARE-RAPPORTS-HID.md`.

## 1. Sources réellement examinées

| Élément | Emplacement sur le Mac | Remarque |
|---|---|---|
| **kernelcache arm64e démarré** | `/System/Volumes/Preboot/<UUID>/boot/A794DF24…/System/Library/Caches/com.apple.kernelcaches/kernelcache` (30 261 171 o, IM4P) | identifié par `sysctl kern.bootobjectspath`. Un second kernelcache (03/09) dort dans un autre `boot/` et n'est pas démarré. Lisible sans root |
| ↳ décompressé | Mach-O `MH_FILESET`, `AARCH64 ARM64e`, 114 622 464 o | `ipsw kernel dec` puis `ipsw kernel extract` |
| ↳ kexts extraits | `IOBluetoothHIDDriver` 9.0.0, `AppleBluetoothHIDKeyboard` 9410.2, `AppleHIDKeyboard` 9410.2, **`IOBluetoothFamily` 9.0.0**, `AppleBluetoothDebug(Service)`, `AppleBluetoothMultitouch` 106, `IOHIDPowerSource` | symboles C++ présents (458 pour `IOBluetoothHIDDriver`) |
| **cache dyld arm64e** | `/System/Volumes/Preboot/Cryptexes/OS/System/Library/dyld/dyld_shared_cache_arm64e*` (12 fichiers, 5,5 Go) | `ipsw dyld extract`, `class-dump`, `disass` |
| ↳ images extraites | **`IOBluetooth`** (1,4 Mo), `IOBluetoothUI`, `CoreBluetooth`, `BluetoothServices`, `BluetoothManager`, `MobileBluetooth`, `BluetoothFirmware` | |
| `/usr/sbin/bluetoothd` | tranche **arm64e** (11,6 Mo, sans symboles) | l'étude précédente avait lu la tranche x86_64 |
| `/usr/libexec/bluetoothuserd`, `/usr/sbin/BTLEServer` | arm64e | aucune référence aux rapports vendeur |
| Programmes de mise à jour | `MobileAccessoryUpdater.framework/XPCServices/*`, `/System/Library/AssetsV2/PreinstalledAssetsV2/InstallWithOs/com_apple_MobileAsset_MobileAccessoryUpdate_*` | seuls les noms et les métadonnées ont été lus (§5) |

Aucune extraction n'a demandé de droits root.

## 2. Ce que l'arm64e confirme (pas de divergence avec l'étude x86_64)

* Les personnalités de PID 598 dans le `__PRELINK_INFO` arm64e sont **identiques octet pour octet** aux personnalités x86 **[plist]** :
  `AppleBluetoothHIDKeyboard` et `AppleEmbeddedKeyboard`, la même carte `ExtendedFeatures`, `Get/SetReportTimeoutMS` = 3500 et `CapsLockDelay` = 75.
* L'union des `ExtendedFeatures` de **toutes** les personnalités Apple du kernelcache ne contient que 14 noms :
  `0x30 0x40 0x41 0x43 0x44 0x45 0x47 0x50-0x55`, plus `SuperMode` `0xD7` (PID 781, 782, 784) **[plist]**.
  Aucune personnalité ne nomme `0xD0 0xD4 0xD5 0xFA 0xFB 0xEA 0xF4-0xF7 0xFE`.
* `IOAppleBluetoothHIDDriver` arm64e garde les mêmes constantes **[désassemblage]** :
  * relevé de batterie toutes les 4 h, codé `0xDBBA00` ms ;
  * bornage à 100 dans `updateBatteryLevel` ;
  * `setCapsLockDelay` limité aux PID `0x22C`-`0x22E` dont le micrologiciel est ≥ `0x137` ;
  * `willShutdown` lit la clé `value`.

  `AppleBluetoothHIDKeyboard::processInterruptData` teste toujours `A1 13 xx`, bit 1, avant d'émettre la notification `KeyboardOff`.
* `IOAppleBluetoothHIDDriver::processSecondaryPacket` et `IOBluetoothHIDDriver::readDeviceName` sont **vides** en arm64e
  (respectivement `ret` et `return 1`) **[désassemblage]**.

**Conclusion** : la partie noyau de `RE-PILOTE-MACOS.md` vaut aussi pour la machine Apple Silicon. Ce qui manquait est ailleurs, dans les trois points suivants.

## 3. Nouveau n° 1 — IOBluetooth.framework : la classe `AppleBluetoothHIDDevice`

Dans le cache dyld, `IOBluetooth.framework` contient une couche espace utilisateur **dédiée aux claviers et souris Bluetooth Apple classiques**.

`-[BluetoothHIDDeviceController bluetoothHIDDeviceForHIDDevice:]` choisit la classe selon la présence de la propriété IORegistry `ExtendedFeatures` **[désassemblage]** :

* présente : classe **`AppleBluetoothHIDDevice`**. C'est le cas de l'A1314, via `IOAppleBluetoothHIDDriver` ;
* absente : classe `AppleBluetoothHIDDeviceGen2` (appareils récents, service d'événements), sinon `BluetoothHIDDevice`.

Elle parle au clavier par `IOHIDDeviceInterface::setReport` (vtable `+0x90`) et `getReport` (`+0x98`), en Feature (type 2),
avec un **délai de 1000 ms**. Les autres ordres passent au pilote noyau sous forme de chaîne de commande (§4).

### 3.1 Méthodes et rapports utilisés

Les octets sont donnés côté HID : ID puis données. Sur le fil HIDP, un SET Feature est précédé de `0x53` et un GET Feature de `0x43`.

| Méthode Objective-C | Rapport | Octets | Taille | Preuve |
|---|---|---|---|---|
| `-batteryPercent` | lit la propriété `BatteryPercent`. Si elle est absente, envoie la commande `UpdateBatteryLevel` au pilote, qui fait un GET `0x47`. Puis **remappe** la valeur (§3.2) | — | — | [désassemblage] |
| `-deviceNameFromHardware` | **GET `DeviceName1..4`** (`0x51`-`0x54`), concaténés | GET `0x51`…`0x54` | tampon de 10 | [désassemblage] |
| `-getMaxDeviceNameLength` | 64 si `LongDeviceName` est présent, sinon 32 | — | — | [désassemblage] |
| `-setDeviceName:` | **renomme côté hôte seulement** (`-[IOBluetoothDevice setName:]`, cache de nom distant), **aucun SET_REPORT** | — | — | [désassemblage] |
| `-factoryDefault` | commande `SuppressDisconnectNotifications`, puis `setFeatureReport:"FactoryDefault"` | `45` | 1 (l'ID seul) | [désassemblage] |
| `-fullFactoryDefault` | idem, avec `"FullFactoryDefault"` | `44` | 1 | [désassemblage] |
| **`-deleteAllLinkKeys`** | **SET Feature `0x44` en dur** ; journal *« Could not tell device to forget its link keys »* | `44` | 1 | [désassemblage] + [chaîne] |
| `-recantConnection` | `SuppressDisconnectNotifications`, puis `setFeatureReport:"RecantConnection"` | `41` | 1 | [désassemblage] |
| `-userMode` / `-setUserMode:` | GET ou SET `UserMode`, bornes min/max lues dans le plist | `43 vv` | 2 | [désassemblage] (absent de notre micrologiciel [mesuré]) |
| **`-sendSCODevicePaired`** | SET Feature `0x4A` = 1 | `4A 01` | 2 | [désassemblage] |
| **`-sendSCODeviceUnpaired`** | SET Feature `0x4A` = 2 | `4A 02` | 2 | [désassemblage] |
| **`-sendSCOLinkActive`** | SET Feature `0x4A` = 3 | `4A 03` | 2 | [désassemblage] |
| **`-sendSCOLinkInactive`** | SET Feature `0x4A` = 4 | `4A 04` | 2 | [désassemblage] |
| **`-connectionCounts:`** | GET Feature `0x4E`, tampon de 10 ; renvoie deux `int` pris à partir de l'octet 5 | GET `4E` | 10 | [désassemblage] |
| **`-sendConnectionIntervalUpdate:intervalSlots:transmitAttempts:asymmetricMultiplier:`** | SET Feature `0xC6` | `C6 en slots tx mult` | 5 | [désassemblage] |
| **`-setLLREnabled:`** | SET Feature `0xDC` ; lit `IsConnectionLLREnabled` et `IsTBFCSuspended` | `DC on 00` | 3 | [désassemblage] |
| `-suspendDevice:` | commande `HIDSuspend` ou `HIDExitSuspend` au pilote : HID_CONTROL `0x13` pour la première ; la seconde ne met rien sur le fil (corrigé #245) | — | — | [désassemblage] |
| `-disconnect` | commande `ReleaseAllChannelsWithSleepForHIDUpdate` au pilote | — | — | [désassemblage] |
| `-setFeatureReport:value:` | générique : taille lue dans `ExtendedFeatures`, de 0 à 4 octets, valeur en petit-boutiste | `ID [v0..v3]` | 1-5 | [désassemblage] |
| `-setFeatureWithReportID:value:` | générique : ID et 1 octet | `ID vv` | 2 | [désassemblage] |
| `-connectToHost:linkKey:`, `-handoffAndRemoveHost:pageType:deviceAddress:linkKey:`, `-removeCurrentHost` | **coquilles vides** sur macOS 26.5 : renvoient 0, aucun envoi | — | — | [désassemblage] |

Dans `-setFeatureReport:value:`, quand la taille déclarée vaut 0, les PID `0x265 0x267 0x269 0x26C 0x310 0x29A 0x29C` reçoivent une taille forcée à 1.
Les autres PID, dont le **598**, n'envoient **que l'ID** **[désassemblage]**.

**Compléments du décompilateur (#242, `RE-GHIDRA-IOBLUETOOTH.md` §2.1) [décompilé]** :

* `getFeatureReport:` assemble la valeur lue en **gros-boutiste**, alors que `setFeatureReport:value:` écrit en
  **petit-boutiste** ; sans effet pour l'A1314, qui n'a aucun rapport de plus d'un octet sur ce chemin ;
* méthodes absentes du tableau : `sendCommandFeatureReport:` (chemin commun de `fullFactoryDefault`, `factoryDefault` et
  `recantConnection` ; lit la clé facultative `value`), `getFloatFeatureReport:`, `setFloatFeatureReport:value:` et
  `report:info:` ;
* `deviceNameFromHardware` copie `longueur_rendue − 2` octets par morceau.

Avant `FactoryDefault`, `FullFactoryDefault` et `RecantConnection`, Apple désactive la notification de déconnexion.
Cela confirme que ces trois commandes **font tomber la liaison**.

**Corrigé (#242)** : sur macOS 26.5, `fullFactoryDefault` et `deleteAllLinkKeys` n'ont plus d'appelant. « Oublier » un HID
Apple classique envoie **`RecantConnection` (`0x41`, ID seul)** depuis `bluetoothd` (`FUN_1005a41e4`, attente de 2000 ms),
et non `FullFactoryDefault` (`0x44`) **[décompilé]** (`RE-GHIDRA-IOBLUETOOTH.md` §5.2).

Le nom de méthode `deleteAllLinkKeys` le confirme pour `0x44` : la remise à zéro complète **efface les clés d'appairage du clavier** **[désassemblage] + [chaîne]**.

### 3.2 Pourcentage de batterie affiché par IOBluetooth pour l'A1314

Le noyau publie `BatteryPercent` brut (`0x47`, borné à 100). `-[AppleBluetoothHIDDevice batteryPercent]` applique ensuite
une **courbe propre aux claviers Apple de 2007 et 2009** **[désassemblage]** :

* elle concerne les PID `0x239`-`0x23B` et `0x255`-`0x257`, soit le masque `0x70000007` sur `PID − 0x239`, ce qui **inclut le 598** ;
* soit `r` la valeur brute de `0x47`.

| `r` brut | % rendu par IOBluetooth |
|---|---|
| 54 à 100 | **100** |
| 21 à 53 | **21 + (r − 21) × 2,4375** (53 → 99, 40 → 67,3, 30 → 42,9) |
| 0 à 20 | `r` (inchangé) |

Le résultat est divisé par 100 et rendu en `float` de 0 à 1. Il vaut −1 si la propriété est absente.

Pour comparaison, une autre courbe s'applique au PID `0x30D` (Magic Mouse), micrologiciel ≤ `0x1FF` :

* 56 à 100 → 100 ;
* 31 à 55 → 11 + (r − 31) × 3,667 ;
* moins de 31 → r / 3.

Les constantes en double précision sont lues dans `__TEXT,__const` d'IOBluetooth : 2,4375, 3,6667 et 0,3333.

**Conséquences [déduction]** :

* le noyau ne convertit pas les mV en %. Apple corrige en revanche, **côté hôte**, la non-linéarité du % du micrologiciel :
  au-dessus de 54 % bruts, le clavier est présenté comme plein ; en dessous, l'échelle est **étirée** jusqu'au seuil de 21 % ;
* `#194` reste juste pour le noyau, mais doit être nuancé : une couche d'affichage Apple existe ;
* nos alertes (#178) et le pourcentage fin (#139) restent fondés sur la tension, ce qui est plus exact que la courbe Apple.
  La courbe Apple ne sert qu'à afficher un « % façon macOS » ;
* l'étude n'a pas établi quel client actuel de macOS 26.5 affiche ce chiffre.
  `bluetoothd` ne contient pas cette courbe : il lit `BatteryPercent` brut via `kCBMsgArgBatteryPercent` **[chaîne]**.
  **Corrigé (#242)** : le seul client du % remappé est **IOBluetoothUI** (icône de batterie de la liste héritée). Réglages
  (`Bluetooth.appex`) passe par `CBBatteryInfo` (CoreBluetooth → `bluetoothd`), donc par le % brut (`RE-GHIDRA-IOBLUETOOTH.md` §2.2).

### 3.3 Nouveaux rapports nommés par Apple, croisés avec nos mesures

| ID | Sens d'après IOBluetooth | Notre micrologiciel `0x0050` | Commentaire |
|---|---|---|---|
| `0x4A` | **état SCO** (audio d'appel) écrit par l'hôte : 1 = appareil SCO appairé, 2 = SCO désappairé, 3 = liaison SCO active, 4 = liaison SCO inactive | GET lisible, **`4a 12`** (18), constant [mesuré] | la valeur lue (18) n'appartient pas à l'énumération 1-4 écrite. Le sens en lecture n'est pas établi. Hypothèse : le clavier adapte sa politique radio quand l'hôte gère de l'audio SCO [déduction] |
| `0x4E` | **compteurs de connexion** : GET de 10 octets, deux entiers de 32 bits | **absent** : pas parmi nos 27 IDs lisibles ni parmi les 11 refus `UNSUPPORTED` [mesuré] | réservé aux appareils plus récents |
| `0xC6` | **mise à jour d'intervalle de connexion** : SET de 5 octets (activation, intervalle en slots, tentatives, multiplicateur asymétrique) | absent | idem |
| `0xDC` | **LLR** (*Low Latency / reliable*, réglage radio) : SET de 3 octets | absent | idem |

Les 11 IDs « sans GET » de notre clavier (`0x40 0x41 0x44 0x45 0x50 0x55 0xD0 0xD4 0xD5 0xFA 0xFB`) restent ceux de `RE-PILOTE-MACOS.md`.

`0xD0 0xD4 0xD5 0xFA 0xFB` **n'apparaissent dans aucun** des éléments suivants : kernelcache arm64e, IOBluetooth, IOBluetoothUI,
BluetoothServices, BluetoothManager, CoreBluetooth, `bluetoothd`, `bluetoothuserd`, `BTLEServer`.
Cela a été vérifié par balayage de tous les appels `setReport` et `getReport` à ID immédiat **[désassemblage]**.

**Corrigé (#239) [décompilé]** : ce balayage avait manqué CoreBluetooth. Sa classe privée `CBHIDPerformanceMonitor` envoie
**SET Feature `0xD5`** = `07` (démarrage) puis `00` (arrêt) d'un test radio « PER » (taux d'erreur paquets) aux PID
`0x239-0x23B` et `0x255-0x257`, dont l'A1314 ; fil HIDP déduit : `53 D5 07` / `53 D5 00`. Aucun client de cette classe
n'est livré dans macOS 26.5 (`RE-GHIDRA-IOBLUETOOTH.md` §4). Seuls `0xD0 0xD4 0xFA 0xFB` restent sans référence.

### 3.4 Écriture du nom dans le micrologiciel

* **macOS 26.5 n'écrit jamais `DeviceNameChange` (`0x50`), `DeviceName1..4` ni `LongDeviceName` (`0x55`)** :
  * `-setDeviceName:` ne renomme que le cache hôte ;
  * l'interface de renommage (`IOBluetoothUI`, `changeDeviceName:newName:`) se sert de `getMaxDeviceNameLength` (64 pour le 598) pour borner la saisie, puis de `setDeviceName:` **[désassemblage] + [chaîne]**.
* macOS **lit** en revanche le nom du micrologiciel (`deviceNameFromHardware` = GET `0x51`-`0x54`). Le format 4 × 8 octets ASCII est conforme à nos mesures.
* Le protocole d'écriture de F40 (#192) reste donc **non observé chez Apple** : l'ordre `0x51-0x55` puis `0x50` est une déduction.

## 4. Nouveau n° 2 — L'interface de commandes du pilote noyau (`setProperties`)

`IOBluetoothHIDDriver::setProperties(OSObject*)` accepte une **chaîne `"Commande [argument]"`**.

* L'argument est lu par `%u`.
* La chaîne est transmise par `IORegistryEntrySetCFProperties` depuis l'espace utilisateur.
* Elle est ensuite dispatchée par `processCommandWL` **[désassemblage]**.

Commandes reconnues, chaînes exactes **[chaîne] + [désassemblage]** :

> **Corrigé (#242)**, au décompilateur (`RE-GHIDRA-IOBLUETOOTH.md` §3.1) : l'interface compte **34 commandes** (13 propres à la classe Apple,
> 21 génériques), et non la vingtaine du tableau ci-dessous, qui omet notamment `SuspendSupported [n]`.

| Classe | Commande | Effet sur le fil |
|---|---|---|
| `IOBluetoothHIDDriver` | `GetProtocol`, `SetReportProtocol` (1), `SetBootProtocol` (0) | GET_PROTOCOL `0x60` ; SET_PROTOCOL `0x71` ou `0x70` |
| | `GetIdle`, `SetIdle n` | GET_IDLE `0x80` ; SET_IDLE `0x90 n` |
| | `HIDControl n` | HID_CONTROL `0x10|n` brut |
| | `HIDSuspend` | HID_CONTROL `0x13`, **sans** condition sur `SuspendSupported` (corrigé #242) |
| | `HIDExitSuspend` | **rien sur le fil** : n'appelle que `handleWake` (effacement de drapeaux) ; pas `0x14` (corrigé #245) |
| | **`VirtualCableUnplug`** | **HID_CONTROL `0x15`** : débranchement du câble virtuel, l'appareil doit oublier l'hôte |
| | `ForceReadDeviceName` | `readDeviceName()`, vide en arm64e |
| | `ReleaseInterruptChannel`, `ReleaseControlChannel` | fermeture L2CAP du PSM 19 ou 17, sans message HID |
| | `ReleaseAllChannels` | **`IOBluetoothDevice::closeConnection`** : coupe la connexion de l'appareil, ce n'est pas une simple fermeture L2CAP (corrigé #242) |
| | **`ReleaseAllChannelsWithSleepForHIDUpdate`** | ferme le canal d'interruption, attend 1300 ms, ferme le canal de contrôle, attend 1300 ms ; sans message HID (précisé #242) |
| | `SuppressDisconnectNotifications`, `Verbose`, `LogPackets`, `DecodePackets`, `ShowMTU`, `Identity`, `SetAuthenticated` | journalisation et état interne, rien sur le fil |
| `IOAppleBluetoothHIDDriver` | `WillShutdown` | SET Feature `0x40` (E7 de l'étude précédente) |
| | `UpdateBatteryLevel`, `UpdateBatteryState` | GET `0x47` ; GET Input `0x30` |
| | `ForceBatteryPercent n`, `DontForceBatteryPercent`, `BatteryUpdateInterval n`, `DefaultBatteryUpdateInterval`, `StartBatteryUpdate`, `StopBatteryUpdate`, `BatteryState`, `BatteryStateNotifications` | cadence et simulation du relevé batterie ; `ForceBatteryPercent 0` est ignoré et `BatteryState n` **simule** l'état n (précisé #242) |
| | `CapsLock n` | Output `0x01` (E8) |

Le pilote décode aussi pour le journal `HID_CONTROL(NOP / HARD_RESET / SOFT_RESET / SUSPEND / EXIT_SUSPEND / VIRTUAL_CABLE_UNPLUG)`.
Il n'envoie de lui-même que SUSPEND et EXIT_SUSPEND.
**Corrigé (#242)** : en 26.5, `handleSleep` du noyau **n'émet plus SUSPEND**, il marque seulement l'état ; l'émission est
dans `bluetoothd` (`RE-GHIDRA-IOBLUETOOTH.md` §3.3, §5.3).
**Corrigé (#245)** : il n'émet jamais EXIT_SUSPEND (`0x14`) non plus ; seule la commande brute `HIDControl 4` le fait (`RE-GHIDRA-KEXT.md` §2.4).

**Délai et disjoncteur [chaîne]** : après **3 expirations consécutives** de GET_REPORT, `IOBluetoothHIDDriver::getReport`
rend `kIOReturnDeviceError` **immédiatement**, sans plus rien émettre, jusqu'à la prochaine réponse.

**Corrigé (#242) [décompilé]** (`RE-GHIDRA-IOBLUETOOTH.md` §3.2) : le disjoncteur protège aussi **SET_REPORT** ; le compteur est commun aux
attentes GET, SET et d'émission ; à la 3e expiration, le pilote **demande à `bluetoothd` de déconnecter** le clavier
(`IOBluetoothDevice::SetHIDDriverReady(false)`). Le délai normal est `GetReportTimeoutMS` (3500 ms pour le 598, 5000 ms par
défaut sans personnalité), la garde ce délai + 1000 ms. Après une veille, le compteur repart à **1** et le premier échange
attend 4500 ms (garde 5500 ms).

**Complété (#245) [décompilé]** (`RE-GHIDRA-KEXT.md` §2.2, §2.5) : le drapeau du disjoncteur est aussi testé dans `sendData`
(`0xfffffe000a35b9cc`) : une fois levé, plus **aucune** émission (HID_CONTROL, SET_PROTOCOL, LED). Un refus par HANDSHAKE
(`0x03`…) compte comme une réponse et remet le compteur à 0 : seul le silence arme le disjoncteur.

C'est la protection d'Apple contre le blocage que nous observons (#175, #177).

## 5. Nouveau n° 3 — `bluetoothd` arm64e : politique de sniff de l'A1314

L'octet de classement `[appareil + 0x310]` a été retrouvé en arm64e : les accesseurs lisent les bits 0 à 7 **[désassemblage]**.

* **Bit 0 (Apple) sans bit 2 (Apple moderne)** : journal *« Device … is an Apple Classic HID Device that **only supports switching Sniff interval to 11.25 ms** »*.
  C'est le cas de l'A1314.
  Les appareils qui ont le bit 2 « supportent 15 ms » **[désassemblage] + [chaîne]**.
* **Bit 3** (famille ancienne `0x0208`-`0x020A`, `0x022C`-`0x022E`, `0x0239`-`0x023B`, `0x0255`-`0x0257`, `0x0309`, `0x030C`)
  entre, avec le bit 1, dans un compteur de la fonction qui journalise
  `numAppleHID / numOldAppleHID / numIncompatibleHID / numTwoSniffApple / numSCODevice / numOfLEHID`.
  Cette fonction choisit l'intervalle de sniff commun quand plusieurs HID sont connectés **[désassemblage partiel]**.
  Cela répond en grande partie à l'inconnue n° 6 de `RE-PILOTE-MACOS.md` : bit 3 = « ancien HID Apple » pour le calcul du sniff.
  **Corrigé (#242) [décompilé]** : la notification SCO de Lion (L9 de `RE-PILOTES-ANCIENS.md` §5) survit en 26.5 **dans
  `bluetoothd`**, pas dans IOBluetooth, dont l'API `sendSCO*` n'a aucun appelant. `FUN_1006a0488` envoie **SET Feature
  `0x4A` = `03`** (fil `53 4A 03`) à tout HID Apple de la famille ancienne (bit 3, dont `0x0256`) à chaque réajustement du
  sniff multi-HID ; `04` n'est jamais envoyé (`RE-GHIDRA-IOBLUETOOTH.md` §5.1).
* La politique d'« Apple HID à 15 ms » (`moveAllAppleHIDsTo15`) **exclut** donc l'A1314 : macOS le maintient à **11,25 ms** (18 slots) de sniff.
* Les mécanismes `setReportWithKeyhole` et `getReportWithKeyhole` (`KeyholeReportID`) de `bluetoothd` ne visent que les appareils récents
  (périphérique `IOHIDUserDevice`). Rien ne relie l'A1314 à ce chemin **[chaîne]**.

## 6. Programme de mise à jour du micrologiciel

* **macOS 26.5 ne contient aucun programme de mise à jour pour les PID `0x0255`-`0x0257`** **[mesuré sur le Mac]** :
  * aucune personnalité ni aucun service `MobileAccessoryUpdater` ne vise 598 ;
  * les services XPC (`UARPUpdaterServiceHID`, `StandaloneHIDAudService`, `EAUpdaterService`, …) visent les PID `0x275`-`0x279` et `0x26C`, ou d'autres classes ;
  * les ressources préinstallées `KeyboardFirmware_5/6/8/10` et `ExternalKeyboardFirmware` ne contiennent que des images `.afu` pour
    `A111`, `A211`, `p0x029A`, `CUST-P029C` et `p0x029F` (Magic Keyboard 2015 et suivants, protocole UARP/AFU).
* Le seul vestige est la commande `ReleaseAllChannelsWithSleepForHIDUpdate`.
  Son nom montre que l'ancien programme (`HIDFirmwareUpdaterTool`, voir `RE-FIRMWARE-MAINTENANCE.md`) **libérait les deux canaux HID L2CAP**
  et ne passait donc pas par le pilote HID **[déduction]**.
  macOS 26.5 la réutilise pour une simple déconnexion (`-[AppleBluetoothHIDDevice disconnect]`).
* Le format du canal de flash du BCM2042 reste donc hors de portée de ce système. Il est à chercher dans les paquets historiques publics (#188).

## 7. Table de synthèse : registre → fonction → preuve → risque

Seules les lignes nouvelles ou corrigées par rapport à `RE-PILOTE-MACOS.md` §8 figurent ici.

| Registre / commande | Fonction Apple | Preuve | Notre clavier | Risque d'écriture |
|---|---|---|---|---|
| `0x47` (lecture) | % brut ; **courbe d'affichage IOBluetooth** pour 0x255-0x257 (§3.2) | [désassemblage] | lisible | aucun : calcul côté hôte |
| `0x44` | `FullFactoryDefault` = **`deleteAllLinkKeys`** (oublie toutes les clés d'appairage) ; sans appelant en 26.5, où « Oublier » envoie `0x41` (corrigé #242) | [désassemblage] + [chaîne] | refus de GET | **INTERDIT** |
| `0x45` | `FactoryDefault` ; Apple coupe la notification de déconnexion avant l'envoi | [désassemblage] | refus de GET | **INTERDIT** |
| `0x41` | `RecantConnection` ; même précaution ; **envoyé par `bluetoothd` quand l'utilisateur oublie le clavier** (ID seul, attente 2000 ms ; corrigé #242) | [désassemblage] ; [décompilé] RE-GHIDRA-IOBLUETOOTH §5.2 | refus de GET | élevé : coupe la liaison |
| `0x4A` | état SCO écrit par l'hôte (1-4) ; en 26.5, `bluetoothd` écrit `03` à l'A1314 à chaque réajustement du sniff (corrigé #242) | [désassemblage] ; [décompilé] RE-GHIDRA-IOBLUETOOTH §5.1 | lit 18 | faible pour `03` (trafic de production Apple) ; moyen pour les autres valeurs |
| `0x4E` | compteurs de connexion (GET de 10 octets) | [désassemblage] | absent | sans objet |
| `0xC6` | intervalle de connexion (SET de 5 octets) | [désassemblage] | absent | sans objet |
| `0xDC` | LLR (SET de 3 octets) | [désassemblage] | absent | sans objet |
| `0x51`-`0x54` (lecture) | `deviceNameFromHardware` | [désassemblage] | lisible | lecture |
| `0x50`, `0x55` (écriture) | **jamais écrits par macOS 26.5** ; le renommage est côté hôte | [désassemblage] | refus de GET | moyen (NVRAM) ; protocole non observé |
| HID_CONTROL `0x15` | `VirtualCableUnplug` (commande de débogage, jamais émise d'office) | [chaîne] + [désassemblage] | — | **élevé** : désappairage demandé au clavier |
| HID_CONTROL `0x13`/`0x14` | `HIDSuspend`/`HIDExitSuspend` (veille et réveil, déjà E5/E6) ; `HIDExitSuspend` n'émet pas `0x14` (corrigé #245) | [désassemblage] ; [décompilé] RE-GHIDRA-KEXT §2.4 | — | faible |
| GET_REPORT ou SET_REPORT ×3 sans réponse | disjoncteur : erreur immédiate, plus d'émission, **puis déconnexion demandée à `bluetoothd`** (corrigé #242) | [chaîne] ; [décompilé] RE-GHIDRA-IOBLUETOOTH §3.2 | — | sans risque : à imiter (#175, #243) |
| Sniff | A1314 = « Apple classique », **11,25 ms seulement** | [désassemblage] + [chaîne] | — | lecture et réglage côté hôte |

## 8. Ce qui reste inconnu

1. `0xD0 0xD4 0xD5 0xFA 0xFB` : **absents de tout macOS 26.5**, noyau arm64e et espace utilisateur.
   **Corrigé (#239)** : sauf `0xD5`, commande de test radio PER de CoreBluetooth (§3.3).
   Ce sont soit des registres du micrologiciel sans client Apple actuel, soit l'ancien canal de flash. Ils sont à chercher dans les paquets historiques (#188).
2. Input `0x04`/`0x05` : aucune référence Apple.
3. Sens de `0x4A` **en lecture** (18) par rapport aux valeurs écrites 1-4.
4. Le client actuel de `-[AppleBluetoothHIDDevice batteryPercent]` sur macOS 26.5 (Réglages, menu, `system_profiler` ?) : non identifié.
   **Résolu (#242)** : IOBluetoothUI seul ; Réglages lit le % brut par CoreBluetooth (§3.2).
5. Effets exacts de `WillShutdown` et `RecantConnection` sur le clavier : non observables sans écriture (#182).
6. Usage exact du bit 3 dans le compteur `numOldAppleHID` (désassemblage partiel).

## 9. Reproduire (lecture seule, sans root)

1. Sur le Mac : `sysctl kern.bootobjectspath` donne le dossier `boot/<hash>` du kernelcache démarré.
   Copier `…/System/Library/Caches/com.apple.kernelcaches/kernelcache` et `/System/Volumes/Preboot/Cryptexes/OS/System/Library/dyld/dyld_shared_cache_arm64e*`
   vers le poste d'analyse, jamais dans un dépôt.
2. Sur le poste d'analyse, avec `ipsw` 3.1.729 :
   * noyau : `ipsw kernel dec kernelcache` puis `ipsw kernel extract <kc> com.apple.driver.IOBluetoothHIDDriver` ;
   * cache dyld : `ipsw dyld extract <dsc> IOBluetooth` ;
   * classes : `ipsw class-dump <dsc> IOBluetooth --class 'AppleBluetoothHIDDevice.*'` ;
   * méthode : `ipsw dyld disass <dsc> --symbol '-[AppleBluetoothHIDDevice deleteAllLinkKeys]' --symbol-image IOBluetooth`.
3. Appels de rapport : chercher `ldr xN, [x8, #0x90]` (setReport) ou `#0x98` (getReport) suivi de `blraaz xN`.
   Les arguments sont dans `w1` (type), `w2` (ID), `w4` (taille) et `w5` (délai).
4. `bluetoothd` arm64e : `lipo -thin arm64e` ; les accesseurs de classement se trouvent par le motif `ldrb w8, [x0, #0x310]` / `ubfx w0, w8, #n, #1` / `ret`.

## 10. Suivi Gitea

| Issue | Objet |
|---|---|
| #213 | F41 — pourcentage « façon macOS » (courbe IOBluetooth, calcul hôte) |
| #214 | disjoncteur après 3 GET_REPORT expirés de suite, comme macOS |
| commentaires | #175, #181, #188, #192, #193, #194 |
