# Rétro-ingénierie au décompilateur Ghidra — Bluetooth HID de macOS 26.5 pour l'A1314

Reprise **au décompilateur** (Ghidra 12.1.2, PyGhidra) de l'étude `RE-MACOS-SILICON.md`, qui reposait sur `ipsw`,
`strings` et capstone. Même machine (Neo01, Mac17,5, macOS 26.5 build 25F71), même clavier
(A1314 ISO, VID `0x05AC`, PID `0x0256`, micrologiciel `0x0050`).

Cadre inchangé : analyse statique en lecture seule, pour l'interopérabilité avec le clavier du gérant.
Aucun binaire Apple, aucun pseudo-C n'est commité : seulement des noms (classes, sélecteurs, symboles), des IDs,
des tailles, des valeurs de délais et des **adresses** (adresses de chargement Ghidra, citées pour retrouver la fonction).
Aucune clé, aucun secret n'a été recherché ni extrait. Aucun accès au clavier.

Niveau de preuve propre à ce document : **[décompilé]** = constaté dans le pseudo-C Ghidra de la fonction nommée,
à l'adresse indiquée. Les autres niveaux suivent `RE-PILOTE-MACOS.md`.

## 1. Méthode et sources

| Élément | Origine sur le Mac | Import Ghidra |
|---|---|---|
| Cache dyld arm64e (13 fichiers, 5,8 Go, **aucun `.symbols`** publié) | `/System/Cryptexes/OS/System/Library/dyld/dyld_shared_cache_arm64e*` | système de fichiers `DyldCacheFileSystem` (18 687 entrées) puis `DyldCacheExtractLoader`, option `Add libobjc.dylib` |
| `IOBluetooth`, `IOBluetoothUI`, `BluetoothServices`, `BluetoothManager`, `CoreBluetooth`, `MobileBluetooth` | images du cache | un projet Ghidra par image |
| `bluetoothd`, `bluetoothuserd`, `bluetoothaudiod`, `hidd`, `uarphidd` | `/usr/sbin`, `/usr/libexec`, tranche arm64e (`llvm-lipo -thin`) | `MachoLoader`, un projet par binaire |
| `Bluetooth.appex` (Réglages), `BluetoothSetupAssistant`, `BluetoothUIServer`, `BluetoothUIService` | ExtensionKit, CoreServices | chaînes seulement |

Analyse complète avec les analyseurs réellement présents dans Ghidra 12.1.2 : `Objective-C Message Analyzer`,
`Objective-C Type Metadata Analyzer`, `Basic Constant Reference Analyzer` (propagation de constantes) et `Decompiler Switch Analysis`.
Les noms de méthodes ObjC viennent des métadonnées de classes (le cache n'a pas de `.symbols`) ; ceux des kexts, de leurs tables de symboles.
Scripts réutilisables : `tests/live/re/ghidra/` (§9).

**Piège Ghidra 12 corrigé (05_fix_vcalls.py)** : sur les appels indirects signés PAC (`blraaz x20`), la propagation de
constantes recopie la cible du `bl` précédent (`-[BluetoothHIDDevice hidDeviceInterface]`). Le pseudo-C montre alors un
appel à `hidDeviceInterface` **sans arguments**, ce qui masque l'ID, la taille et le délai des
`IOHIDDeviceInterface::setReport` (vtable `+0x90`) et `getReport` (`+0x98`). Ces références fautives
(50 dans IOBluetooth, 34 dans IOBluetoothUI) sont supprimées avant décompilation ; chaque appel est ensuite
recontrôlé au listing (`04_disasm.py`). C'est vraisemblablement la cause des trous des études par scripts.

## 2. IOBluetooth : inventaire exhaustif des rapports

Décompilation de **toutes** les fonctions d'IOBluetooth (5 249, aucun échec). Il n'existe que **8 appels**
`setReport`/`getReport` par la vtable, tous dans `AppleBluetoothHIDDevice`, plus un `IOHIDDeviceGetReport` USB.
Format de l'appel : `(iface, type, id, tampon, taille | &taille, délai_ms, 0, 0, 0)` ; type 2 = Feature partout.

| Méthode | Adresse | Sens | ID | Taille | Délai | Preuve |
|---|---|---|---|---|---|---|
| `deleteAllLinkKeys` | `0x19641b71c` (appel `0x19641b77c`) | SET | **`0x44` en dur** | 1 (ID seul) | 1000 ms | [décompilé] + listing `mov w2,#0x44 / mov w4,#0x1 / mov w5,#0x3e8` |
| `deviceNameFromHardware` | `0x19641b420` | GET ×4 | `DeviceName1..4` (via `ExtendedFeatures`) | tampon 10, longueur en entrée/sortie | 1000 ms | [décompilé] |
| `setLLREnabled:` | `0x19641bd64` | SET | `0xDC` | 3 : `DC on 00` | 1000 ms | [décompilé] |
| `sendConnectionIntervalUpdate:intervalSlots:transmitAttempts:asymmetricMultiplier:` | `0x19641bf04` | SET | `0xC6` | 5 : `C6 en slots tx mult` | 1000 ms | [décompilé] |
| `getFeatureReport:` | `0x19641c168` | GET | `id` du dictionnaire | tampon 10 | 1000 ms | [décompilé] |
| `setFeatureReport:value:` | `0x19641c388` | SET | `id` du dictionnaire | `size + 1` | 1000 ms | [décompilé] |
| `setFeatureWithReportID:value:` | `0x19641c538` | SET | argument | 2 | 1000 ms | [décompilé] |
| `connectionCounts:` | `0x196463c08` | GET | `0x4E` | tampon 10 | 1000 ms | [décompilé] |
| `-[IOBluetoothAutomaticDeviceSetup usbHIDDeviceConnected:result:sender:device:]` | `0x196410c84` | `IOHIDDeviceGetReport` | **`0x34`** | 0x4D (77) | — | [décompilé] : appairage **par câble USB** (adresse BT à l'octet 4) ; sans objet pour l'A1314, sans port USB |

Aucune autre constante d'ID n'atteint un appel de rapport dans IOBluetooth : `0xD0 0xD4 0xD5 0xFA 0xFB 0x4B 0xD1 0xD8 0xF6 0xF7`
n'y sont **pas** émis ni lus **[décompilé, balayage exhaustif]**.

### 2.1 Détails nouveaux sur les accesseurs génériques

* **`ReportInfo`** (`-report:info:`, `0x19641c778`) = `{id, size, min, max}` lus dans `ExtendedFeatures` (clés `id`, `size`, `min`, `max`).
  Un rapport sans clé `id` est refusé ; l'`id` vaut −1 par défaut.
* **`getFeatureReport:`** n'accepte qu'une taille déclarée de **1 à 4** octets, vérifie que l'octet 0 rendu égale l'ID,
  puis assemble la valeur en **gros-boutiste** (`v = v << 8 | octet`). **`setFeatureReport:value:`** sérialise au contraire
  en **petit-boutiste**. L'asymétrie est réelle dans le code Apple **[décompilé]**. Elle n'a d'effet que pour les tailles > 1,
  donc aucun pour l'A1314 (seul `0x43 UserMode`, 1 octet, serait concerné, et il est absent de notre micrologiciel).
* `setFeatureReport:value:` : taille 0 → **ID seul** (1 octet), sauf PID `0x265 0x267 0x269 0x26C 0x310 0x29A 0x29C`
  (taille forcée à 1, valeur ajoutée). Confirme `RE-MACOS-SILICON.md` §3.1.
* `sendCommandFeatureReport:` (`0x19641c2bc`, **non documentée jusqu'ici**) : chemin commun de `fullFactoryDefault`,
  `factoryDefault` et `recantConnection`. Elle lit la clé facultative **`value`** du rapport dans `ExtendedFeatures`,
  puis appelle `setFeatureReport:value:`. La personnalité du PID 598 n'ayant pas de `value`, l'octet émis est l'ID seul.
* `getFloatFeatureReport:` / `setFloatFeatureReport:value:` (**non documentées**) : valeur normalisée
  `(v − min) / (max − min)`, arrondi `+0,5` à l'écriture. Aucun appelant dans le cache.
* **`deviceNameFromHardware`** : la longueur passée à `getReport` est une variable d'entrée/sortie **initialisée à 10 une seule fois**,
  donc réduite après la première lecture ; chaque morceau copie `longueur_rendue − 2` octets depuis l'octet 1, à la suite,
  borné à 42 octets. Avec une réponse de 9 octets (ID + 8), Apple ne garderait que 7 caractères par morceau
  **[décompilé ; effet = déduction]**. La longueur rendue par la pile Bluetooth n'est pas établie ici.

### 2.2 Batterie

* `-[AppleBluetoothHIDDevice batteryPercent]` (`0x19641ba94`) : **courbe confirmée au décompilé**, constantes `2.4375`,
  `21.0`, bornes `r − 0x36 ≤ 0x2E` (54-100 → 100) et `r − 0x15 ≤ 0x20` (21-53 → `21 + (r−21)×2,4375`) ; le masque
  `0x70000007` sur `PID − 0x239` couvre `0x239-0x23B` et `0x255-0x257`. Hors bornes (0-20 **et > 100**) la valeur passe inchangée.
  Si `BatteryPercent` manque, la méthode envoie la commande `UpdateBatteryLevel` et relit la propriété aussitôt
  (sans attendre la réponse du clavier) ; sinon −1.
* **Notifications batterie** (`-serviceInterestOfType:argument:`, `0x19641b318`, **nouveau**) : les messages IOKit du noyau
  deviennent des notifications `NSNotificationCenter` :
  `'btlw'` → `BluetoothHIDDeviceBatteryLow` ; `'btpn'` → `BluetoothHIDDeviceBatteryDangerouslyLow` ;
  `'rbsn'` (retour à la normale) → aucune alerte ; dans les trois cas `BluetoothHIDDeviceBatteryStateChanged`.
* `batteryLow` = `BatteryPanic` **ou** `BatteryLow` (propriétés IORegistry) ; `batteryDangerouslyLow` = `BatteryPanic`.
* **Client réel du pourcentage remappé (inconnue n° 4 de `RE-MACOS-SILICON.md`)** : dans tout le cache dyld, seul
  **`IOBluetoothUI`** appelle `batteryPercent` : `IOBluetoothUI_BatteryControl::batteryPercentForIcon` (`0x19f217d18`,
  icône de batterie) et `IOBluetoothUICollectionView::_statusString:` (`0x19f21d71c`, libellé « chargé » si > 98 %, utilisé
  seulement pour un appareil **en charge**, donc jamais pour l'A1314). Les Réglages de macOS 26.5 (`Bluetooth.appex`) ne
  passent pas par IOBluetooth : ils lisent `CBBatteryInfo` (CoreBluetooth → `bluetoothd`, valeur brute) **[chaîne]**.

### 2.3 Classe générique et câble virtuel (nouveau)

* `-[BluetoothHIDDevice serviceInterestOfType:argument:]` (`0x196431e94`) : le message noyau **`'vcup'`** publie
  `BluetoothHIDDeviceVirtualCableUnplugged`. macOS sait donc **recevoir** un débranchement de câble virtuel venant de l'appareil.
* `-[BluetoothHIDDevice recantConnection]` (`0x196432444`) : pour un HID non Apple, envoie la commande noyau
  `VirtualCableUnplug` (HID_CONTROL `0x15`) seulement si `-[IOBluetoothDevice HIDSupportsVirtualCable]`.
  La sous-classe Apple la remplace par SET `RecantConnection` (`0x41`).
* `-[BluetoothHIDDevice disconnect]` = `destroyConnection` (ACL) ; la version Apple envoie
  `ReleaseAllChannelsWithSleepForHIDUpdate` (fermeture des canaux L2CAP 17/19 seulement).
* `-[AppleBluetoothHIDDevice remoteNameRequestComplete:status:]` (`0x19641c904`) : après une requête de nom distant HCI,
  envoie la commande noyau `ForceReadDeviceName` (vide en arm64e, cf. `RE-MACOS-SILICON.md` §2).

### 2.4 Qui appelle ces méthodes sur macOS 26.5 ? (correction importante)

Balayage des pointeurs vers chaque sélecteur dans **toutes les images du cache** (`06_dsc_selref_scan.py`, contrôle positif :
`setName:` est trouvé dans des centaines d'images). Résultat :

| Sélecteur | Images qui le référencent |
|---|---|
| `sendSCODevicePaired`, `sendSCODeviceUnpaired`, `sendSCOLinkActive`, `sendSCOLinkInactive` | IOBluetooth seul |
| `fullFactoryDefault`, `factoryDefault`, `deleteAllLinkKeys`, `recantConnection` | IOBluetooth seul |
| `setLLREnabled:`, `sendConnectionIntervalUpdate:…`, `connectionCounts:`, `suspendDevice:`, `setUserMode:` | IOBluetooth seul |
| `batteryPercent`, `getMaxDeviceNameLength`, `registerForBatteryStateChangeNotifications:selector:` | IOBluetooth + **IOBluetoothUI** |

Et dans IOBluetooth même, **aucune** de ces méthodes n'a d'appelant **[décompilé]**. Les binaires hors cache copiés
(`bluetoothd`, `bluetoothuserd`, `bluetoothaudiod`, `hidd`, `uarphidd`, `Bluetooth.appex`, `BluetoothUIServer`,
`BluetoothUIService`) ne contiennent aucun de ces sélecteurs ; `BluetoothSetupAssistant` ne nomme que les classes.

**Conséquence** : sur macOS 26.5, les envois L9 (`0x4A` sur lien SCO) et L10 (`0x44` à la suppression du clavier) de
`RE-PILOTES-ANCIENS.md` §5 **ne sont plus faits par la couche IOBluetooth** ; ce sont des faits de 10.7.5.
Reste à vérifier si `bluetoothd` les a repris en C++ (§4).

## 3. Noyau arm64e : `IOBluetoothHIDDriver` 9.0.0 et `AppleBluetoothHIDKeyboard` 9410.2 au décompilateur

Kernelcache **démarré** (`kern.bootobjectspath`, IM4P de 30 261 171 o, charge LZFSE décompressée en `MH_FILESET` arm64e de
114 622 464 o), entrées extraites par `MachoFileSetFileSystem` + `MachoFileSetExtractLoader`. Adresses noyau non glissées.
Les 207 fonctions propres d'`IOBluetoothHIDDriver` et les 26 d'`AppleBluetoothHIDKeyboard` ont été décompilées (aucun échec).

### 3.1 Interface de commandes texte : 34 commandes (et non « une vingtaine »)

`IOAppleBluetoothHIDDriver::processCommandWL` (`0xfffffe000a3555a4`) traite les 13 commandes Apple puis, en ligne, les
21 de `IOBluetoothHIDDriver::processCommandWL` (`0xfffffe000a3618e4`). Les commandes génériques exigent l'appareil prêt
(`deviceReady` fait) et les deux canaux ouverts, sinon `kIOReturnNotReady` (`0xE00002D8`). Commande inconnue : `0xE00002BC`.

| Commande | Argument | Effet exact **[décompilé]** |
|---|---|---|
| `WillShutdown` | — | `willShutdown()` : SET Feature `WillShutdown` (`0x40`), 1 octet de valeur si la clé `value` existe, sinon l'ID seul |
| `UpdateBatteryLevel` | — | `updateBatteryLevel()` : GET `BatteryPercent` (`0x47`) |
| `ForceBatteryPercent` | n | mémorise n (`+0x1A0`) puis `updateBatteryLevel()`. **n ≤ 0 est ignoré** (valeur du clavier conservée) |
| `DontForceBatteryPercent` | — | `+0x1A0 = −1` puis `updateBatteryLevel()` |
| `BatteryUpdateInterval` | n (s) | intervalle = n × 1000 ms et réarme la minuterie ; sans minuterie : journal « Battery Update Is Off » |
| `DefaultBatteryUpdateInterval`, `StartBatteryUpdate` | — | `startBatteryUpdate()` (relevé 60 s plus tard) |
| `StopBatteryUpdate` | — | annule la minuterie |
| `BatteryState` | n facultatif | **simule** l'état : `updateBatteryState(n)` (avec n) ; sans n, journalise l'état courant |
| `UpdateBatteryState` | — | `updateBatteryState(getBatteryState())` : GET Input `0x30` |
| `BatteryStateNotifications` | — | renvoie les notifications d'état batterie |
| `CapsLock` | n | `setCapsLock(n≠0)` : `A2 01 02`/`A2 01 00` sur le canal d'interruption |
| `ReleaseAllChannelsWithSleepForHIDUpdate` | — | supprime la notification de déconnexion, ferme le canal **d'interruption**, attend **1300 ms**, ferme le canal **de contrôle**, attend 1300 ms (nouveau : ordre et délais) |
| `GetProtocol` / `SetReportProtocol` / `SetBootProtocol` | — | GET_PROTOCOL ; SET_PROTOCOL(1) ; SET_PROTOCOL(0) |
| `HIDControl` | n | `hidControl(n)` : octet `0x10 | (n & 0x0F)` sur le canal de contrôle |
| `SuspendSupported` | n facultatif | pose le drapeau `SuspendSupported` (1 par défaut) |
| `HIDSuspend` | — | `hidControl(3)` = SUSPEND, **sans** vérifier `SuspendSupported` |
| `HIDExitSuspend` | — | n'agit que si le pilote se croit suspendu (`+0x82`) : sortie de veille |
| `GetIdle` / `SetIdle` | — / n | GET_IDLE / SET_IDLE |
| `VirtualCableUnplug` | — | supprime la notification de déconnexion puis `hidControl(5)` = VIRTUAL_CABLE_UNPLUG |
| `ForceReadDeviceName` | — | `readDeviceName()` (vide en arm64e) |
| `ReleaseInterruptChannel` / `ReleaseControlChannel` | — | ferme le canal L2CAP 19 / 17 (notification de déconnexion supprimée) |
| `ReleaseAllChannels` | — | **`IOBluetoothDevice::closeConnection`** (vtable `+0x988`, résolue par `07_vtable_slot.py` dans `IOBluetoothFamily`) : coupe la connexion de l'appareil, pas seulement les canaux. Correction de `RE-MACOS-SILICON.md` §4 |
| `SuppressDisconnectNotifications` | — | drapeau `+0x169` |
| `Verbose` | n | niveau de journal (≥ 5 pour la batterie) |
| `LogPackets` / `DecodePackets` | n | trace brute / décodée des paquets HIDP |
| `ShowMTU` | — | journalise les MTU de l'appareil et des deux canaux |
| `Identity`, `ID`, `id` | — | journalise VID source/VID/PID/version/pays |
| `SetAuthenticated` | — | pose la propriété IORegistry `Authenticated = true` |

### 3.2 Délais et disjoncteur (corrigés et complétés)

* Délais lus dans la personnalité par `handleStart` (`0xfffffe000a358558`) : `GetReportTimeoutMS` (`+0x28`) et
  `SetReportTimeoutMS` (`+0x2C`), **5000 ms par défaut**, 3500 pour le PID 598.
* `waitForData` (`0xfffffe000a35daa0`, GET), `waitForHandshake` (`0xfffffe000a35e478`, SET) et `waitForOkToSend`
  (`0xfffffe000a35ec50`) attendent `timeout` ms ; un chien de garde est armé à **timeout + 1000 ms** (4500 ms pour l'A1314).
* **Premier échange après une veille** : `handleSleep` (`0xfffffe000a35abe4`) pose `_mUseSleepTimeout` ; l'attente passe alors
  à **4500 ms** (`0x1194`) et le chien de garde à **5500 ms** (`0x157C`), puis le mode est levé.
* **Compteur commun** (`mHandshakeTimeoutCounter`, `+0xA1`) aux trois attentes : chaque expiration l'incrémente ; **à 3** :
  1. le drapeau `+0xA0` est posé : `getReport` (`0xfffffe000a35b128`) **et `setReport`** (`0xfffffe000a35b3c0`) rendent
     aussitôt `kIOReturnDeviceError` (`0xE00002E9`) sans rien émettre ;
  2. **le pilote demande à `bluetoothd` de déconnecter l'appareil** : journal « 3 consecutive timeout happened -- notifying
     bluetoothd to disconnect », appel `IOBluetoothDevice::SetHIDDriverReady(false)` (vtable `+0xA50`). **Nouveau.**
* Remise à zéro : toute réponse (HANDSHAKE ou DATA) remet `+0xA0`/`+0xA1` à 0 ; `deviceConnectTimerFired` remet le compteur à 0.
  **Après une veille, le compteur repart à 1** (`+0xA0 = 0x0100`) : 2 expirations suffisent alors à couper. **Nouveau.**

Conséquence pour #175/#214 : macOS ne se contente pas de refuser les requêtes, il **déconnecte** le clavier qui ne répond
plus. Le comportement mesuré sous Linux (liaison coupée après un GET sans réponse) est donc aussi celui d'Apple, en plus strict.

### 3.3 Connexion, batterie, veille

* `IOBluetoothHIDDriver::deviceReady` (`0xfffffe000a35a588`) : SET_PROTOCOL(Report) avec un délai **temporaire de 10 000 ms**
  (au lieu de `SetReportTimeoutMS`), sauf `SuppressSetProtocol`. **En cas d'échec, l'appareil n'est pas déclaré prêt** :
  aucune commande générique ni relevé de batterie ne démarre. **Nouveau.**
* `IOAppleBluetoothHIDDriver::deviceReady` (`0xfffffe000a354f2c`) : crée la minuterie, intervalle **14 400 000 ms** (4 h),
  annule tout forçage, état batterie mémorisé = 0, puis `startBatteryUpdate` (`0xfffffe000a356034`) : premier relevé à **60 000 ms**.
* `batteryLevelTimerFired` (`0xfffffe000a355034`) : `updateBatteryLevel()` puis `updateBatteryState(getBatteryState())` ;
  prochain relevé à 4 h si **les deux** réussissent, sinon **3 600 000 ms** (1 h). Un échec du GET `0x30` suffit donc à passer en 1 h.
* `updateBatteryLevel` (`0xfffffe000a35611c`) : octet 1 de `0x47`, forçage s'il est > 0, `≥ 100 → 100`, propriété `BatteryPercent` (8 bits).
* `updateBatteryState(n)` (`0xfffffe000a356378`) : **seulement si n change** : relit d'abord `0x47`, puis pose
  `BatteryLow`/`BatteryPanic` et envoie `'rbsn'` (0), `'btlw'` (1) ou `'btpn'` (2, 3) ; n > 3 → erreur `0xE00002BC`.
  Les notifications d'état sont renvoyées à chaque appel, même sans changement.
* `IOAppleBluetoothHIDDriver::processInterruptData` : rapport de 3 octets `A1 30 xx` → `updateBatteryState(xx)`.
* `AppleBluetoothHIDKeyboard::processInterruptData` (`0xfffffe00090758a0`) : `A1 13 xx`, bit 1 = 0, sans `DisablePoweredOffCheck` →
  notification `KeyboardOff` (`'bsk2'`) **et** drapeau « supprimer la notification de déconnexion » : la coupure qui suit
  l'extinction n'est pas annoncée comme une déconnexion. **Nouveau.**
* **Veille** : en 26.5, `handleSleep` du noyau **n'émet plus SUSPEND** ; il note seulement l'état « suspendu » (`+0x82`) si
  `SuspendSupported`. L'émission est faite par `bluetoothd` (§4). Au réveil (`setPowerStateWL`, ordinal 1, `0xfffffe000a35fcf4`),
  l'instant est mémorisé et la sortie de suspension est traitée si l'état « suspendu » est posé.
* Rejet des rapports d'entrée au réveil : `processInterruptData` (`0xfffffe000a35d1c8`) contient une fenêtre de **1,6 s**
  (`Δµs >> 9 < 3125`) pendant laquelle les DATA sont ignorées, pour tous les PID sauf `0x265 0x267 0x269 0x26C 0x29A 0x29C 0x310`.
  Le drapeau qui l'active (`+0x68`) n'est jamais mis à 1 dans le code décompilé : **inactive** en 26.5.
* Réassemblage HIDP : `DATA` (`0xA_`) et `DATC` (`0xB_`) sont recollés dans un tampon de 64 Kio quand le paquet atteint le MTU.

## 4. Nouveau : `0xD5` = démarrage/arrêt du test radio « PER » des anciens HID Apple (CoreBluetooth)

C'est la première référence Apple à l'un des cinq IDs « refusés en lecture » restés inconnus (`0xD0 0xD4 0xD5 0xFA 0xFB`).
Elle se trouve dans **CoreBluetooth.framework**, classe privée **`CBHIDPerformanceMonitor`** (journal `CBHIDPerf`), que les
études par `strings`/scripts n'avaient pas ouverte.

| Méthode | Adresse | Ce qu'elle fait **[décompilé]** |
|---|---|---|
| `-_hidStartPERAndRetunError:` (*sic*) | `0x199b38250` | choisit l'ID et la valeur selon le PID, puis `_hidSetFeatureWithReportID:value:error:` |
| `-_hidStopPERAndRetunError:` | `0x199b2009c` | même ID, **valeur 0** |
| `-_hidSetFeatureWithReportID:value:error:` | `0x199b201ac` | SET Feature de **2 octets `[ID, valeur]`**, délai 1000 ms, par `IOHIDDeviceInterface::setReport` (`+0x90`) pour les anciens HID ; par `IOHIDDeviceSetReport` (IOHIDManager) sinon |
| `-_isAppleOldHIDs:` | `0x199b1eff4` | « ancien HID Apple » = `0x208-0x20A`, `0x22C-0x22E`, `0x239-0x23B`, **`0x255-0x257`**, `0x309`, `0x30C-0x30E` |
| `-_packetLoggerStart`, `-_packetLoggerProcessPacketData:`, `-_rssiAndHandleRead`, `-_calculatePercentile:percentile:`, `-_showSummaryResult:…` | — | la mesure se fait sur les paquets HCI (intervalles en µs, centiles, « intervalle excessif », RSSI), pas sur un rapport HID |

Table ID/valeur de démarrage (`_hidStartPER`) :

| PID | ID | Valeur de démarrage |
|---|---|---|
| **`0x239-0x23B`, `0x255-0x257` (dont l'A1314 `0x0256`)** | **`0xD5`** | **`0x07`** |
| `0x30D` (Magic Mouse) | `0xD5` | `0x0C` |
| `0x30E` (Magic Trackpad) | `0xD5` | `0x0F` |
| `0x267`, `0x26C`, `0x29A`, `0x29C`, `0x29F`, `0x320-0x322` | `0xD6` | `0x0A` |
| `0x269`, `0x323` | `0xD6` | `0x2D` |
| `0x265`, `0x324` | `0xD6` | `0x30` |
| autres (`0x266 0x268 0x26A 0x26B`…) | — | « Unsupported HID » |

Octets sur le fil pour l'A1314 [déduction du format HIDP] : démarrage `53 D5 07`, arrêt `53 D5 00`.

**Cohérence avec nos mesures** : `0xD5` refuse le GET (`ERR_UNSUPPORTED_REQUEST`) [mesuré, `RE-HID-EXHAUSTIF.md`], comme une
commande en écriture seule. **Sens** : « test PER » (taux d'erreur paquets) = le périphérique passe en émission radio
périodique que l'hôte chronomètre ; la valeur `07` est un paramètre du test (cadence ou motif), **son unité n'est pas établie**.

**Qui l'appelle ?** Personne dans macOS 26.5 livré : la classe n'est référencée que par CoreBluetooth lui-même
(balayage des sélecteurs du cache et des démons copiés). C'est un outil de laboratoire Apple non livré, mais le code et la table
de PID sont dans le système.

**Risque** : **moyen**. Le clavier passe dans un mode de test radio (consommation, trafic continu, effet sur la saisie inconnu) ;
l'arrêt documenté est `D5 00`. Aucun essai sans l'accord du gérant (plan W de #182). Ce n'est **pas** le canal de flash.

## 5. `bluetoothd` arm64e : ce qu'il envoie réellement à l'A1314

Tranche arm64e de `/usr/sbin/bluetoothd` (11,6 Mo, **sans symboles** : noms `FUN_<adresse>`, adresses du binaire non glissé,
base `0x100000000`). Analyse Ghidra complète (≈ 69 min), 225 fausses références d'appel indirect retirées, puis décompilation
des 853 fonctions qui citent une chaîne HID (et de leurs appelés directs).

Toutes les émissions HID de `bluetoothd` passent par **`FUN_10069a2b8(profil, type, handle, données, longueur)`**
(`enqueueUserSpaceHIDDataForDevice`) : type **1** = HID_CONTROL, type **5** = SET_REPORT, premier octet des données = type de
rapport HIDP (`03` = Feature) puis l'ID. Elle n'a que **8 sites d'appel** ; tous ont été lus :

| Site | Rôle | Données | Concerne l'A1314 ? | Preuve |
|---|---|---|---|---|
| `FUN_1006a1ae0` (`prepareForSleep`) | SUSPEND à la mise en veille | HID_CONTROL `03` → `0x13` | **oui** (bit 0) | [décompilé] |
| `FUN_1006a1ae0` | « Clear Wake Reason » | SET `03 F0 C5 00` | non (bit 2 seulement) | [décompilé] |
| `FUN_1006a1ae0` | trackpad avant veille | SET `03 F2 21 00` | non (bit 7) | [décompilé] |
| `FUN_1006a2484` | trackpad au réveil | SET `03 F2 21 01` | non | [décompilé] |
| `FUN_1006a2108` (`prepareForWake`) | EXIT_SUSPEND | HID_CONTROL `0x14` | **oui** | [décompilé] |
| **`FUN_10069b8a4`** (`setSniffIntervalForOneSniffAttemptAppleHID`) | « SCO Link State feature report » | **SET `03 4A 03`** | **oui** (bit 3) | [décompilé] + listing `0x1006a05f0` `mov w2,#0x4a` / `mov w3,#0x3` |
| `FUN_10069b9ec` (`setSniffIntervalForAppleHID`) | `SET_CONNECTION_INTERVAL_CONTROL` | SET `03 C6 01 slots tentatives asym` | non (Apple sans bit 3) | [décompilé] |
| `FUN_10035133c` | clavier Touch ID | SET `03 C9 …` | non | [décompilé] |

### 5.1 `0x4A = 03` est envoyé par macOS 26.5 à l'A1314 (correction)

`FUN_1006a0488` (`0x1006a0488`) parcourt les HID connectés à **chaque recalcul de la configuration de sniff multi-HID**
(`FUN_10069ccc8` ; déclencheurs observés parmi ses 17 appelants : session SCO stéréo, connexion ou départ d'un appareil audio,
changement de mode LE, recalculs HID). Pour chaque HID **Apple (bit 0) de la famille ancienne (bit 3**, qui contient `0x0255-0x0257`) :
**SET Feature `0x4A` = `03`** (« SCO Link Active »), fil `53 4A 03`, **sans condition sur l'état SCO réel** dans ce chemin.
Les autres HID Apple reçoivent à la place, **seulement si leur intervalle doit changer**, `C6 01 <18|24> <1|2> <0x2C|0x20>` (intervalle 11,25 ou 15 ms, 1 ou 2 tentatives).
La valeur `04` (`SCOLinkInactive`) n'apparaît dans aucun site d'appel de `bluetoothd`.

Conséquences : L9 de `RE-PILOTES-ANCIENS.md` survit en 26.5 **dans bluetoothd** (pas dans IOBluetooth), simplifié à la seule
valeur `03`. Écrire `4A 03` reproduit donc un trafic de production Apple : **risque faible**. Notre lecture `4A 12` (18) reste inexpliquée.

### 5.2 Oublier le clavier : `RecantConnection` (`0x41`), pas `FullFactoryDefault` (correction)

* `FUN_1005a3f64` (journal « HID device … will unpair ») : si l'appareil est **connecté**, appelle `FUN_1005a41e4`
  (« Unplugging virtual cable to device … »), puis attend **2000 ms** la déconnexion (« HID recanted Successfully » /
  « HID timedout waiting to recant »).
* `FUN_1005a41e4` (`0x1005a41e4`) choisit selon la classe [décompilé + listing `0x1005a4310`-`0x1005a4330`, données lues à `0x1008c1ae4`] :
  * HID **non Apple** : HID_CONTROL `VIRTUAL_CABLE_UNPLUG` (`0x15`) ;
  * HID Apple **classique** (bit 2 = 0, **A1314**) : SET Feature de 1 octet **`41`** = `RecantConnection`, fil `53 41` ;
  * HID Apple moderne (bit 2 = 1) : SET Feature `40 03` (`WillShutdown` avec la valeur 3).
* Le même chemin est accessible par la commande de propriété `BT_KEY_HID_VIRTUAL_CABLE_UNPLUG` (`FUN_1005b6a98`).

Donc, sur macOS 26.5, **« Oublier » ce clavier dans Réglages envoie `RecantConnection` (`0x41`) et non `FullFactoryDefault`
(`0x44`)** comme Lion (L10). L'effet de `0x41` sur le clavier (oubli de l'hôte ou simple coupure) reste à observer.

### 5.3 Veille, réveil, sniff, reconnexion

* **Veille** (`FUN_1006a1ae0`) : pour chaque HID connecté, Apple classique → HID_CONTROL SUSPEND **une seule fois par handle**
  (ensemble des appareils déjà suspendus), puis attente **≤ 1000 ms** des événements de changement de mode
  (« Received mode change events for all suspended devices » ou « Timeout waiting… »).
* **Réveil** (`FUN_1006a2108`) : pour chaque HID Apple, EXIT_SUSPEND (par le pilote noyau ou directement selon que l'appareil a
  un pilote HID en espace utilisateur, `FUN_1004ca8f4`).
* **Classement sniff** (`FUN_1005a6db0`) : 4 compteurs {total, Apple 15 ms (bit 2), **Apple classique 11,25 ms (bit 0 sans bit 2)**,
  non Apple}. Confirme `RE-MACOS-SILICON.md` §5 au décompilé.
* Accesseurs de l'octet de classe `+0x310` : bit 0 `FUN_1004c676c`, bit 2 `FUN_1004ca878`, **bit 3 `FUN_1004cac94`**,
  bit 4 `FUN_1004cacac`, bit 7 `FUN_1004cacc4` ; `+0x311` bit 0 `FUN_1004cacd0`.
* **Reconnexion** : `BT::HIDProfile::startHIDAutoConnect(bool)` est lancée au changement d'état d'alimentation
  (« powerStateChanged, starting HID auto connect »), avec files `updateAutoConnectQueues`, `autoConnectNextHID`,
  pause/reprise/annulation, et « Skipping nvram write because HID auto connect is in progress ». **L'hôte reconnecte lui-même
  les HID appairés** au démarrage et au réveil du contrôleur [chaîne + noms de fonctions].
* **Clés d'appairage** : non extraites. Le flux se limite, côté HID, à `removeTrustedPairingForHID:` et au chemin d'oubli §5.2.
* Autres émissions de `bluetoothd` (par `IOHIDDeviceSetReport`/`GetReport`) : appairage **par câble USB** des Magic Keyboard
  (`FUN_100631860`, `FUN_100633098`, `FUN_1006512d8` : GET Feature `0x34` = adresse, SET Feature `0x35` de 25 octets = clé de lien
  et adresse hôte, cf. CVE-2024-0230 dans `RE-COMMANDES-VENDEUR.md` §3.3), manettes Sony (`FUN_10064ff88`) et appareils
  `VID 0x4C / page 0xFF00 / usage 0x12` (`FUN_10069f0b8`). **Aucun ne concerne l'A1314** (pas d'USB, VID source USB `0x05AC`).
  Flux décrit seulement : aucune clé n'a été lue ni extraite.

## 6. Recherche active des IDs encore inconnus

| ID | Constante immédiate dans un appel de rapport | Tables / chaînes / sélecteurs | Verdict |
|---|---|---|---|
| **`0xD5`** | **oui** : CoreBluetooth `CBHIDPerformanceMonitor` (§4) | log `CBHIDPerf`, `Unsupported HID: PID 0x%04X` | **identifié : test PER** |
| `0xD6` | oui (même classe, Magic 2015+) | — | sans objet A1314 |
| `0xD0`, `0xD4`, `0xFA`, `0xFB` | **non** (IOBluetooth, IOBluetoothUI, CoreBluetooth, BluetoothServices, BluetoothManager, MobileBluetooth : décompilation exhaustive ; kexts HID : exhaustive ; `bluetoothd` : les 8 sites d'émission + 6 appelants `IOHIDDevice*Report`) | aucune personnalité (`ExtendedFeatures`), aucun sélecteur ni format | inconnus de macOS 26.5 |
| `0x4B`, `0xD1`, `0xD8`, `0xF6`, `0xF7` (lisibles) | non | non | jamais lus par macOS 26.5 |
| Input `0x04`, `0x05` | — : le noyau ne filtre en entrée que `A1 13` (clavier) et `A1 30` (batterie) | non | inconnus |
| `0xC9` | oui, `bluetoothd` `FUN_10035133c` | `AppleMagicTouchIDKeyboard` | Touch ID, sans objet |

`0x4A` (écrit `03` par `bluetoothd`), `0x41` (écrit à l'oubli), `0x44` (`deleteAllLinkKeys`, sans appelant) : voir §2 et §5.

## 7. Croisement avec nos mesures et nos documents

**Confirmé au décompilé** : courbe `batteryPercent` (21/53/2,4375) ; GET `0x47` puis Input `0x30` toutes les 4 h, premier relevé à
60 s, 1 h après échec ; `ExtendedFeatures` → `type` 2 par défaut ; `A1 13` bit 1 = extinction ; `A1 30 xx` = état batterie ;
SET_PROTOCOL(Report) à la connexion ; `setCapsLockDelay` réservé à `0x22C-0x22E` ; SUSPEND/EXIT_SUSPEND pour les HID Apple ;
A1314 = « Apple Classic 11,25 ms » ; `0x4E`, `0xC6`, `0xDC` réservés aux appareils récents ; `connectToHost:linkKey:` & co. vides.

**Corrigé** :

| Affirmation antérieure | Correction | § |
|---|---|---|
| L9 (`0x4A`) et L10 (`0x44`) « ne sont plus envoyés » (déduction tirée d'IOBluetooth seul) | `0x4A = 03` **est** envoyé par `bluetoothd` à chaque réajustement du sniff ; l'oubli envoie **`0x41`**, pas `0x44` | 5.1, 5.2 |
| Disjoncteur : « erreur immédiate jusqu'à la prochaine réponse » (GET seulement) | aussi sur SET ; **déconnexion demandée à bluetoothd** ; compteur à 1 après veille | 3.2 |
| ~25 commandes, `ReleaseAllChannels` = fermeture L2CAP | 34 commandes ; `ReleaseAllChannels` = `IOBluetoothDevice::closeConnection` | 3.1 |
| `HIDSuspend` conditionné par `SuspendSupported` | non conditionné | 3.1 |
| SUSPEND envoyé par le noyau à la veille (E5) | en 26.5, envoyé par `bluetoothd` ; le noyau ne fait que marquer l'état | 3.3, 5.3 |
| Client de la courbe % inconnu | IOBluetoothUI (icône) ; Réglages lit le % brut via CoreBluetooth | 2.2 |

**Nouveau** : `0xD5` = test PER (§4) ; notifications `BluetoothHIDDeviceBatteryLow/DangerouslyLow/StateChanged/VirtualCableUnplugged` ;
délais 4500/5500 ms après veille ; SET_PROTOCOL en échec ⇒ appareil non prêt ; `ReleaseAllChannelsWithSleepForHIDUpdate`
(1300 ms ×2) ; `getFeatureReport:` gros-boutiste / `setFeatureReport:value:` petit-boutiste ; attente de 2000 ms à l'oubli ;
`ForceBatteryPercent 0` ignoré ; `KeyboardOff` supprime la notification de déconnexion suivante.

## 8. Table registre → fonction → preuve → risque (état après ce reverse)

| Registre / commande | Fonction Apple (macOS 26.5) | Preuve | Notre clavier | Risque d'écriture |
|---|---|---|---|---|
| Feature `0x47` | % brut, relevé toutes les 4 h ; courbe d'affichage IOBluetoothUI | [décompilé] noyau + IOBluetooth | `47 63` | lecture |
| Input `0x30` | état batterie 0/1/2-3 ; GET après chaque relevé + poussé | [décompilé] | `30 00` | lecture passive |
| Input `0x13` bit 1 | extinction → `KeyboardOff`, déconnexion silencieuse | [décompilé] | — | lecture passive |
| Feature `0x40` | `WillShutdown` à l'arrêt/redémarrage (ID seul) | [décompilé] | refus GET | faible (production Apple) |
| Feature `0x41` | **`RecantConnection` envoyé à l'oubli** (`bluetoothd` `FUN_1005a41e4`), attente 2 s de la coupure | [décompilé] + listing | refus GET | **élevé** : coupe la liaison, effet sur l'appairage du clavier non observé |
| Feature `0x44` | `deleteAllLinkKeys` / `fullFactoryDefault` : API IOBluetooth **sans appelant** en 26.5 | [décompilé] + balayage cache | refus GET | **INTERDIT** |
| Feature `0x45` | `factoryDefault` : sans appelant | idem | refus GET | **INTERDIT** |
| Feature `0x4A` | **`03` écrit par `bluetoothd`** à chaque réajustement sniff/SCO (bit 3) ; 1-4 dans l'API IOBluetooth sans appelant | [décompilé] + listing `0x1006a05f0` | lit `12` | **faible** pour `03` (trafic Apple) ; autres valeurs : moyen |
| **Feature `0xD5`** | **test PER** : `07` démarre, `00` arrête (CoreBluetooth, outil non livré) | [décompilé] | refus GET | **moyen** (mode de test radio) |
| `0xD0 0xD4 0xFA 0xFB` | aucune | balayage exhaustif | refus GET | **élevé** (inconnus) : ne pas toucher |
| `0x50`-`0x55` | lecture `0x51-0x54` seulement (`deviceNameFromHardware`) | [décompilé] | — | moyen (NVRAM) |
| HID_CONTROL `0x13`/`0x14` | SUSPEND (bluetoothd, une fois par handle, attente ≤ 1 s) / EXIT_SUSPEND | [décompilé] | — | faible |
| HID_CONTROL `0x15` | non-Apple seulement à l'oubli ; commande noyau `VirtualCableUnplug` | [décompilé] | — | élevé |
| GET/SET ×3 sans réponse | disjoncteur + **déconnexion** | [décompilé] | — | à imiter (#214) |

## 9. Ce qui reste inconnu

1. `0xD0 0xD4 0xFA 0xFB` et les Input `0x04`/`0x05` : aucune trace dans macOS 26.5 (frameworks, kexts, `bluetoothd`).
   Restent à chercher dans les paquets historiques (#188) ou par mesure passive.
2. Sens de la valeur `07` de `0xD5` (cadence ? motif ?) et comportement du clavier pendant le test PER.
3. Effet exact de `0x41` sur l'A1314 (simple coupure ou oubli de l'hôte) ; lien avec la lecture `4A 12`.
4. Liste exhaustive des événements qui déclenchent le recalcul `FUN_10069ccc8` (17 appelants, partiellement identifiés).
5. `uarphidd`, `hidd`, `bluetoothuserd`, `bluetoothaudiod` : analysés par Ghidra mais seulement survolés (chaînes) :
   aucun indice de lien avec l'A1314 (UARP visé par descripteur, pas par PID 598).
6. La fenêtre de rejet d'entrée de 1,6 s du noyau : drapeau jamais posé dans le code lu (inactive ?).

## 10. Reproduire (sans root, lecture seule, hors dépôt)

Scripts `tests/live/re/ghidra/` (PyGhidra 3.1, Ghidra 12.1.2 ; `pip install /opt/ghidra/Ghidra/Features/PyGhidra/pypkg/dist/pyghidra-*.whl`,
plus `pyliblzfse` pour le kernelcache) :

| Script | Rôle |
|---|---|
| `akm_ghidra.py` | démarrage PyGhidra, options d'analyse (ObjC, constantes, `switch`) |
| `01_import_analyze.py` | import d'images du cache dyld (`--dsc`/`--image`), d'entrées du kernelcache (`--fileset`/`--entry`, `--list`) ou de Mach-O (`--bin`), un projet par binaire, analyse complète |
| `02_decompile.py` | décompilation (toutes, par nom, par chaîne citée, par adresse ; `--callees`) vers un fichier hors dépôt |
| `03_objc_inventory.py` | classes ObjC filtrées et leurs méthodes |
| `04_disasm.py` | listing d'une fonction (contrôle des arguments d'appels virtuels) |
| `05_fix_vcalls.py` | **retire les fausses références d'appel sur `blr*`** avant décompilation (indispensable) |
| `06_dsc_selref_scan.py` | quelles images du cache référencent un sélecteur ou une classe (Python pur) |
| `07_vtable_slot.py` | nom de la méthode C++ à un décalage de vtable d'un kext |

Étapes : copier le cache (`/System/Cryptexes/OS/System/Library/dyld/dyld_shared_cache_arm64e*`), le kernelcache démarré
(`kern.bootobjectspath` → `…/com.apple.kernelcaches/kernelcache`, retirer l'IM4P : flux LZFSE de `bvx2` à `bvx$`) et
`/usr/sbin/bluetoothd` (`llvm-lipo -thin arm64e`), puis 01 → 05 → 02 (→ 04 pour contrôler). Les copies Apple ont été supprimées
du poste d'analyse à la fin de l'étude.

## 11. Suivi Gitea

| Issue | Objet |
|---|---|
| #239 | `0xD5` = test radio PER (D5 07 / D5 00) |
| #242 | corrections de docs issues de ce reverse |
| #243 | disjoncteur complet façon 26.5 (SET compris, coupure au 3e échec, compteur à 1 après veille) |
| #244 | F47 — SUSPEND/EXIT_SUSPEND à la veille du poste |
| commentaires | #188, #213, #214, #216 (`4A 03` par bluetoothd), #217 (oubli = `0x41`) |
