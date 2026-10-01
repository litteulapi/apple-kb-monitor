# Rétro-ingénierie du pilote noyau Bluetooth HID de macOS avec Ghidra : A1314 (PID 0x0256, fw 0x0050)

Reprise **au décompilateur** (Ghidra 12.1.2) du travail de [`RE-PILOTE-MACOS.md`](RE-PILOTE-MACOS.md),
[`RE-MACOS-SILICON.md`](RE-MACOS-SILICON.md) et [`RE-PILOTES-ANCIENS.md`](RE-PILOTES-ANCIENS.md), faits jusqu'ici
avec `strings`, `ipsw` et capstone. Objectif : retrouver les fonctions manquées et apporter, pour chaque fait,
**l'adresse et le nom de la fonction** qui le prouvent.

Cadre : analyse statique en lecture seule des logiciels Apple installés sur le Mac du gérant (Neo01, Mac17,5,
macOS 26.5 build 25F71), pour l'interopérabilité avec **son** clavier A1314 ISO.

* Aucun binaire Apple, aucun pseudo-C, aucun code décompilé n'est commité : seulement des faits courts
  (noms de classes et de symboles, adresses, IDs, tailles, valeurs de temporisation).
* Aucun accès au clavier, aucune écriture HID, aucune clé ni secret recherché.
* Les copies des fichiers Apple et le projet Ghidra ont été supprimés du poste d'analyse à la fin.

Niveaux de preuve : **[plist]**, **[décompilé]** (sortie du décompilateur Ghidra, relue), **[désassemblage]**,
**[chaîne]**, **[déduction]**, **[mesuré]** (renvoie à `HARDWARE-RAPPORTS-HID.md`).
Les adresses sont celles du kernelcache arm64e démarré (`0xfffffe00…`) ; celles du KC x86_64 (`0xffffff80…`) sont signalées.

## 0. Rapport avec `RE-GHIDRA-IOBLUETOOTH.md` (travail parallèle)

Une étude menée en parallèle (branche `re/ghidra-iobluetooth`, issues #239, #242-#244) a décompilé avec Ghidra
IOBluetooth.framework, CoreBluetooth, `bluetoothd` **et** le pilote noyau. Les deux études ont été faites **indépendamment**
(projets Ghidra, scripts et lectures distincts) ; leurs constats noyau concordent, ce qui vaut contre-vérification :
disjoncteur commun GET/SET avec `SetHIDDriverReady(false)`, compteur à 1 après veille, délais 3 500/4 500/5 500 ms,
34 commandes, `handleSleep` sans émission, SET_PROTOCOL initial à 10 000 ms, `KeyboardOff` + drapeau `+0x169`,
fenêtre de rejet de 1,6 s inactive.

**Propre à ce document** (absent de `RE-GHIDRA-IOBLUETOOTH.md`) :

1. le test du disjoncteur est aussi dans **`sendData`** : une fois levé, **plus aucune** émission, HID_CONTROL et LED comprises (§2.2) ;
2. **`HIDExitSuspend` n'émet pas `0x14`** : il n'appelle que `handleWake`, qui ne fait qu'effacer des drapeaux ; le noyau
   n'appelle `hidControl(4)` nulle part. Pour l'A1314 (pilote noyau), le « EXIT_SUSPEND » de `bluetoothd` au réveil passe par
   cette commande et **ne met donc rien sur le fil** ; cela nuance #244 (§2.4) ;
3. table `DecodedHandshake` (codes de retour IOKit par type de refus) et rejet des réponses dont l'**ID** diffère de la requête
   (« Report does not equal the report we asked for » → expiration) (§2.5) ;
4. `getExtendedReport` **exige `size`** : les entrées `0x40 0x41 0x44 0x45 0x50` sont illisibles par construction (§4) ;
5. `newReportDescriptor` ne corrige pas le descripteur ; `A1 30 xx` est consommé avant IOHID (§4) ;
6. second chemin `0x09` par IOHID (`FWCapsLockDelay`, claviers USB `0x220-0x222`/`0x24F-0x251`, version ≥ `0x68`) (§4.1) ;
7. classe **`IOBluetoothHIDChannel`** d'`IOBluetoothFamily`, second pilote HID sans couche Apple (§5.1) ;
8. balayage exhaustif des immédiats **et des segments de données** de six kexts, dont `IOHIDFamily` et `AppleHIDTransport` (§5) ;
9. **diff x86_64 ↔ arm64e** (§6) ;
10. `Info.plist` du plug-in présent sur disque, clés `Connection/DisconnectionNotificationType`, `'bsk2'` = type générique (§2.1) ;
11. chaîne Ghidra réutilisable pour les KC fileset, dont l'application des `LC_SYMTAB` que le chargeur de Ghidra 12.1.2 omet (§1).

## 1. Sources et méthode

| Élément | Origine (Mac) | Traitement |
|---|---|---|
| kernelcache **arm64e démarré** | `Preboot/<UUID>/boot/A794DF24…/…/kernelcache` (30 261 171 o, IMG4), désigné par `sysctl kern.bootobjectspath`, lisible sans root | IMG4 → IM4P → LZFSE décodés en espace utilisateur (`pyimg4` + `pyliblzfse`) : Mach-O `MH_FILESET` arm64e de 114 622 464 o |
| `BootKernelExtensions.kc` | `/System/Library/KernelCollections` (67 551 232 o) | Mach-O `MH_FILESET` x86_64, pour le diff |
| `Info.plist` des kexts | `/System/Library/Extensions/{IOBluetoothHIDDriver,AppleHIDKeyboard(+PlugIns),AppleHIDKeyboardEmbedded,AppleMultitouchDriver,IOBluetoothFamily}.kext` | `plutil -convert xml1` côté Mac, lecture `plistlib` |

Méthode Ghidra (scripts réutilisables dans `tests/live/re/ghidra/`) :

1. `kc_extract.py decode` : kernelcache → Mach-O ; `kc_extract.py entries` : entrées `LC_FILESET_ENTRY` ;
2. `ghidra_import.py` : import **du KC entier** par le chargeur Mach-O de Ghidra (fileset reconnu, chaînage des pointeurs
   résolu ; symboles : voir étape 4), puis **analyse automatique limitée** aux segments de six kexts :
   `IOBluetoothHIDDriver`, `IOBluetoothFamily`, `AppleHIDKeyboard`, `AppleBluetoothHIDKeyboard`, `IOHIDFamily`,
   `AppleHIDTransport`. Garder le KC entier permet de nommer les appels inter-kexts et les appels au noyau ;
3. `ghidra_dump.py` : inventaire des fonctions, décompilation **locale** des classes visées, balayage de toutes les
   constantes immédiates (IDs de rapport, PID, temporisations) et des références de chaînes.
4. `ghidra_symbols.py` : **indispensable** — le chargeur Mach-O de Ghidra 12.1.2 n'applique **aucun** symbole des entrées
   d'un `MH_FILESET` (8 262 fonctions, toutes `FUN_…`, au premier passage). Le script relit le `LC_SYMTAB` de chacune des
   366 entrées, pose 314 083 symboles, en démangle 245 272 (`DemanglerCmd`) et crée 199 703 fonctions ;
5. `annotate_vcalls.py` : annote les appels virtuels `(*(*this + off))(…)` du pseudo-C local avec le nom lu dans la vtable
   de la classe réelle de l'A1314 (`__ZTV25AppleBluetoothHIDKeyboard`, 413 entrées exportées dans `vtables.tsv`) ;
6. `ghidra_decomp.py` : décompilation ciblée par regex hors des kexts visés, avec la liste des appelants.

Durées sur le poste (20 cœurs, 31 Go) : import 60-90 s ; analyse limitée 124 s, à condition de **désactiver**
l'analyseur « Non-Returning Functions - Discovered », qui boucle plus de 30 min en réparation de flot sur le KC entier.

Bilan de l'inventaire arm64e (fonctions par kext, après symboles) : `IOBluetoothHIDDriver` 224, `AppleBluetoothHIDKeyboard` 30,
`AppleHIDKeyboard` 123, `IOBluetoothFamily` 1 250, `IOHIDFamily` 4 247, `AppleHIDTransport` 2 388. Pseudo-C produit et relu
pour les 377 fonctions des trois premiers et pour 141 fonctions HID des deux familles.

## 2. Ce que les études précédentes avaient manqué ou inversé

### 2.1 Personnalité du 598 : deux clés oubliées, et l'`Info.plist` du plug-in existe sur disque

`AppleHIDKeyboard.kext/Contents/PlugIns/AppleBluetoothHIDKeyboard.kext/Contents/Info.plist` (9410.2) **existe** sur le volume
système et contient les 3 personnalités 597/598/599 ; `RE-PILOTE-MACOS.md` §1 le disait présent « uniquement dans
`__PRELINK_INFO` ». Carte `ExtendedFeatures` du 598 inchangée (13 entrées, §3 de `RE-PILOTE-MACOS.md`) **[plist]**.
Clés absentes des tableaux précédents **[plist]** :

| Clé | Valeur | Lue par |
|---|---|---|
| `ConnectionNotificationType` | `Connected` | `IOBluetoothHIDDriver::handleStart` (`0xfffffe000a358558`) |
| `DisconnectionNotificationType` | `Disconnected` | idem ; `sendDeviceDisconnectNotifications` ajoute `MouseDisconnected` pour une souris |
| `HIDDefaultBehavior` | chaîne vide | couche IOHID |

Les notifications (`KeyboardOff`, `LowBattery`, `CriticallyLowBattery`, `Connected`, `Disconnected`) partent toutes par
`messageClientsWithString` avec le **même** type de message `'bsk2'` (`0x62736b32`) et la chaîne en charge utile **[décompilé]** :
`'bsk2'` n'est donc pas propre à `KeyboardOff` (correction de `RE-PILOTE-MACOS.md` §7).

### 2.2 Disjoncteur : après 3 expirations, macOS fait **déconnecter** le clavier

`RE-MACOS-SILICON.md` §4 décrivait un disjoncteur qui rend `kIOReturnDeviceError` « jusqu'à la prochaine réponse ».
Le décompilé montre davantage **[décompilé]** :

| Élément | Fonction (adresse arm64e) | Fait |
|---|---|---|
| Compteur `mHandshakeTimeoutCounter` (octet `+0xa1` de l'état interne) | `waitForData` `0xfffffe000a35daa0`, `waitForHandshake` `0xfffffe000a35e478`, `waitForOkToSend` `0xfffffe000a35ec50` | +1 à chaque expiration, **commun** aux GET (attente des données), aux SET Feature (attente du HANDSHAKE) et aux DATA Output (attente « OK to send ») |
| Seuil | idem | à la **3e** expiration consécutive (`compteur > 2`) : drapeau disjoncteur (`+0xa0`) = 1, **puis `IOBluetoothDevice::SetHIDDriverReady(false)`** (journal *« 3 consecutive timeout happened -- notifying bluetoothd to disconnect »*) ; cette méthode passe la propriété `HIDShimDeviceCreated` à faux et prévient le contrôleur, donc `bluetoothd` |
| Effet du drapeau | `getReport` `0xfffffe000a35b128`, `setReport` `0xfffffe000a35b3c0`, **`sendData` `0xfffffe000a35b9cc`** | rendent `0xE00002E9` (`kIOReturnDeviceError`) **sans rien émettre** ; le test dans `sendData` bloque **toute** émission (HID_CONTROL, SET_PROTOCOL, LED Verr. Maj comprises) |
| Remise à zéro | `waitForData`/`waitForHandshake`/`waitForOkToSend` (réponse reçue), `init` ; `deviceConnectTimerFired` `0xfffffe000a35a8bc` (compteur seul) | drapeau et compteur repassent à 0 à la première réponse valide (y compris un refus HANDSHAKE) ; le compteur seul à la connexion. Une fois le drapeau levé, plus rien n'est émis : en pratique seuls une nouvelle connexion (nouvel objet pilote) ou une veille de l’hôte (`handleSleep` : drapeau à 0, compteur à 1) le lèvent |
| **Après une veille** | `IOBluetoothHIDDriver::handleSleep` `0xfffffe000a35abe4` | compteur forcé à **1** (*« setting mHandshakeTimeoutCounter to 1 and _mUseSleepTimeout to true »*) : au réveil, **2** expirations suffisent à déclencher la déconnexion |

Conséquence pour #175/#243 : le comportement Apple n'est pas « se taire » mais **se taire puis abandonner la liaison**.

### 2.3 Délais réels d'une requête (et non 3,5 s)

| Grandeur | Valeur pour le 598 | Preuve |
|---|---|---|
| Attente de la réponse à un GET_REPORT | `GetReportTimeoutMS` = **3 500 ms** (`commandSleep`, boucle) | `waitForData` lit `+0x28`, rempli par `handleStart` depuis `GetReportTimeoutMS` (5 000 par défaut) |
| Minuterie de garde du même GET | `GetReportTimeoutMS + 1000` = **4 500 ms** | `waitForData` : `timer->setTimeoutMS(+0x28 + 1000)` |
| Attente du HANDSHAKE d'un SET | `SetReportTimeoutMS` = 3 500 ms, garde 4 500 ms | `waitForHandshake` lit `+0x2c` |
| **Mode « après veille »** (`_mUseSleepTimeout`, posé par `handleSleep`, effacé après la 1re requête) | attente **4 500 ms** (`0x1194`), garde **5 500 ms** (`0x157c`), quelles que soient les clés du plist | `waitForData`, `waitForHandshake`, `waitForOkToSend` |
| Attente que le pilote soit prêt avant d'émettre | **5 s** (`clock_interval_to_deadline(5, 1e9)`), sinon `kIOReturnNotReady` (`0xE00002D8`) | `getReportWL` `0xfffffe000a360d28`, `setReportWL` `0xfffffe000a36111c` |
| Fermeture des canaux pour mise à jour | 1 300 ms après chaque canal (`IOSleep(0x514)`) | `IOAppleBluetoothHIDDriver::processCommandWL` `0xfffffe000a3555a4` |

L'« espacement de 1000 ms » cité jusqu'ici est le **délai d'appel** passé par IOBluetooth.framework (espace utilisateur,
`RE-MACOS-SILICON.md` §3) ; dans le noyau, 1 000 ms est la **marge de la minuterie de garde** ajoutée au délai du plist.
Aucun espacement imposé entre deux requêtes n'existe dans le pilote noyau : la sérialisation vient du `IOCommandGate`
(une requête à la fois) **[décompilé]**.

### 2.4 Veille et réveil : le noyau 26.5 n'émet **ni SUSPEND ni EXIT_SUSPEND** de lui-même

`RE-PILOTE-MACOS.md` (E5/E6) et `RE-PILOTES-ANCIENS.md` (L6 : `handleSleep → hidControl(3)` en 10.7) laissaient croire que
le pilote noyau envoyait `0x13`/`0x14`. En 26.5 arm64e **[décompilé]** :

| Fonction | Adresse | Ce qu'elle fait réellement |
|---|---|---|
| `IOBluetoothHIDDriver::setPowerStateWL` | `0xfffffe000a35fcf4` | état 0 → `handleSleep` ; état 1 → horodate le réveil (µs, `GetCurrentTime`) puis `handleWake` si une suspension est en cours |
| `IOBluetoothHIDDriver::handleSleep` | `0xfffffe000a35abe4` | **aucun envoi** : compteur d'expirations à 1, mode « délai après veille », et, si `SuspendSupported` et pilote prêt, marque `_mHIDSuspendSent = 1`, `_mExitHIDSuspendResult = 0` |
| `IOAppleBluetoothHIDDriver::handleSleep` | `0xfffffe000a354d5c` | `stopBatteryUpdate`, puis la version de base |
| `IOBluetoothHIDDriver::handleWake` / `IOAppleBluetoothHIDDriver::handleWake` | `0xfffffe000a35ad04` / `0xfffffe000a354e40` | **aucun envoi** : efface `_mHIDSuspendSent`, puis `startBatteryUpdate` (GET `0x47` 60 s plus tard) |
| `IOBluetoothHIDDriver::hidControl` | `0xfffffe000a35f488` | **seul** émetteur de HID_CONTROL : octet `0x10 | (n & 0x0F)` sur le canal de contrôle ; n = 3 (SUSPEND) bascule aussi l'état LLR/`IsTBFCSuspended` de l'`IOBluetoothDevice` |
| Appelants de `hidControl` | — | **uniquement** `IOBluetoothHIDDriver::processCommandWL` (commandes `HIDControl n`, `HIDSuspend` → 3, `VirtualCableUnplug` → 5). Aucun appel depuis `handleSleep`, `handleWake`, `setPowerStateWL` (vérifié sur les 224 fonctions du kext) |
| `HIDExitSuspend` | `processCommandWL` | **n'émet pas `0x14`** : appelle seulement `handleWake` (effacement des drapeaux) si une suspension est marquée |
| `getReportWL` / `setReportWL` | `0xfffffe000a360d28` / `0xfffffe000a36111c` | si `_mHIDSuspendSent` et le dernier « exit suspend » a échoué, appellent `handleWake` puis émettent quand même : la **première requête après le réveil** fait office de sortie de veille |

Conclusion **[décompilé + déduction]** : sur macOS 26.5, `0x13` n'est émis que si `bluetoothd` envoie la commande `HIDSuspend`
(`prepareForSleep`, `RE-GHIDRA-IOBLUETOOTH.md` §5.3). Au réveil, `bluetoothd` (`prepareForWake`) passe, pour un appareil piloté
par le noyau comme l'A1314, par la commande `HIDExitSuspend`, qui **ne met rien sur le fil** ; le chemin « direct » de
`bluetoothd` concerne les appareils à pilote HID en espace utilisateur (`RE-GHIDRA-IOBLUETOOTH.md` §5.3). Le clavier sort donc de SUSPEND par son propre trafic
(frappe) ou par la requête suivante de l'hôte. `0x14` n'est émissible qu'avec la commande brute `HIDControl 4`.

**Filtre de touches au réveil (code mort en 26.5)** : `IOBluetoothHIDDriver::processInterruptData` (`0xfffffe000a35d1c8`)
contient un filtre qui **jette les rapports Input** reçus moins de **1,6 s** (`Δµs >> 9 < 3125`) après l'horodatage du réveil,
sauf pour les PID `0x265 0x267 0x269 0x26C 0x29A 0x29C 0x310`. Le drapeau qui l'arme (`+0x68`) n'est jamais mis à 1 dans le
kext arm64e (seulement remis à 0 par `handleStart` et par le filtre) : filtre **inactif** sur cette version **[décompilé]**.

### 2.5 HANDSHAKE, réponses refusées et câble virtuel

* `DecodedHandshake` (`0xfffffe000a35e324`) : `0` → succès ; `1` NOT_READY → `kIOReturnNotReady` ; `2` INVALID_REPORT_ID →
  `kIOReturnBadMessageID` (`0xE00002C6`) ; `3` UNSUPPORTED_REQUEST → `kIOReturnUnsupported` (`0xE00002C7`) ;
  `4` INVALID_PARAMETER → `kIOReturnBadArgument` ; `0xE`/`0xF` → `kIOReturnError` **[décompilé]**.
  Un refus par HANDSHAKE **est une réponse** : il remet le compteur d'expirations à 0. Nos 11 IDs « sans GET » (refus `0x03`)
  ne déclenchent donc jamais le disjoncteur ; seul le **silence** le déclenche (cas `0xFE`, #175).
* `processControlData` (`0xfffffe000a35c898`) n'accepte une réponse DATA que si son type (`0xA0 | type`) **et son ID**
  (octet 1) sont ceux de la requête en cours ; sinon *« Report does not equal the report we asked for »* : paquet ignoré,
  la requête finit en expiration. Paquets de la taille du MTU : réassemblage DATC (`0xB0`) jusqu'à 64 Kio.
* **HID_CONTROL VIRTUAL_CABLE_UNPLUG reçu du clavier** (`0x15` sur le canal de contrôle) : message IOKit `'vcup'`, fermeture du canal
  d'interruption puis du canal de contrôle, `IOSleep(1000)`, puis `IOBluetoothDevice::closeConnection` **[décompilé]**.
* `setReportWL` : Feature → `0x53` sur le canal de contrôle puis attente du HANDSHAKE ; Output/Input → `0xA0 | type` sur le
  canal d'**interruption** puis attente « OK to send » ; fragmentation au MTU sortant avec des fragments DATC `0xB0 | type`
  (et un fragment vide final si la charge est un multiple exact) **[décompilé]**.
* `deviceReady` (`0xfffffe000a35a588`) : SET_PROTOCOL(Report) `0x71` avec un délai de HANDSHAKE **porté à 10 000 ms** le temps
  de cette seule requête ; **si elle échoue, le pilote reste « non prêt »** (aucun relevé batterie, requêtes refusées
  `kIOReturnNotReady`). `SuppressSetProtocol` (absent pour le 598) la supprime **[décompilé]**.

## 3. Interface de commandes texte (liste exhaustive)

`IOBluetoothHIDDriver::setProperties` (`0xfffffe000a35b658`) prend une `OSString` « `Commande` » ou « `Commande n` »
(coupure au premier espace, `sscanf("%u")` dans un `OSNumber` 32 bits), puis l'exécute sous le `IOCommandGate` par
`IOAppleBluetoothHIDDriver::processCommandWL` (`0xfffffe000a3555a4`), qui délègue à `IOBluetoothHIDDriver::processCommandWL`
(`0xfffffe000a3618e4`). La fonction de base exige pilote prêt et canaux de contrôle et d'interruption ouverts
(sinon `kIOReturnNotReady`). **34 commandes** **[décompilé + chaîne]** :

| Commande | Classe | Émis sur le fil (A1314) |
|---|---|---|
| `WillShutdown` | Apple | SET Feature `0x40`, ID seul (`53 40`) ; si l'entrée a une clé `value`, 1 octet de plus |
| `UpdateBatteryLevel` | Apple | GET Feature `0x47`, tampon 2 o (`43 47`), vérifie `buf[0] == 0x47` |
| `UpdateBatteryState` | Apple | GET **Input** `0x30`, tampon 2 o (`41 30`) |
| `BatteryState [n]` | Apple | rien ; sans argument : journal ; avec n : simule l'état n (notifications) |
| `BatteryStateNotifications` | Apple | rien (renvoie les notifications d'état) |
| `ForceBatteryPercent n` / `DontForceBatteryPercent` | Apple | rien, puis relevé `0x47` |
| `BatteryUpdateInterval n` | Apple | rien ; période = n × 1000 ms |
| `DefaultBatteryUpdateInterval`, `StartBatteryUpdate` | Apple | relevé 60 s plus tard (`0x47` puis `0x30`) |
| `StopBatteryUpdate` | Apple | rien |
| `CapsLock n` | Apple | DATA Output sur le canal d'interruption : `A2 01 02` (n ≠ 0) ou `A2 01 00` |
| `ReleaseAllChannelsWithSleepForHIDUpdate` | Apple | fermeture L2CAP interruption puis contrôle, 1 300 ms après chacune ; aucun octet HID |
| `GetProtocol` | base | `60` |
| `SetReportProtocol` / `SetBootProtocol` | base | `71` / `70`, puis attente du HANDSHAKE |
| `GetIdle` / `SetIdle n` | base | `80` / `90 nn` |
| `HIDControl n` | base | **`1n`** brut (n & 0x0F) : y compris HARD_RESET `11`, SOFT_RESET `12`, EXIT_SUSPEND `14` |
| `HIDSuspend` | base | `13` (SUSPEND) |
| `HIDExitSuspend` | base | **rien** (§2.4) |
| `VirtualCableUnplug` | base | `15`, et marque la déconnexion comme attendue |
| `SuspendSupported [n]` | base | rien ; pose le drapeau qui autorise le marquage de suspension (posé à vrai par `IOAppleBluetoothHIDDriver::handleStart`) |
| `ForceReadDeviceName` | base | rien (`readDeviceName` vide) |
| `ReleaseInterruptChannel`, `ReleaseControlChannel`, `ReleaseAllChannels` | base | fermeture L2CAP PSM 19 / 17 / les deux |
| `SuppressDisconnectNotifications` | base | rien ; drapeau « déconnexion attendue » (`+0x169`) |
| `Verbose n`, `LogPackets n`, `DecodePackets n`, `ShowMTU`, `Identity`, `SetAuthenticated` | base | rien (journal, propriété `Authenticated`) |

La fonction ne contrôle pas elle-même les privilèges de l'appelant **[décompilé]** ; le contrôle d'accès éventuel relève
d'IOKit en amont, non étudié ici.

## 4. `ExtendedFeatures` : chemins GET et SET exacts

| Étape | Fonction | Fait **[décompilé]** |
|---|---|---|
| Chargement | `IOAppleBluetoothHIDDriver::handleStart` `0xfffffe000a35456c` | lit `ExtendedFeatures` (dictionnaire) ; pose `SuspendSupported = true`, `BatteryLow = BatteryPanic = false` ; lit les trois types de notification |
| Lecture | `getExtendedReport(nom)` `0xfffffe000a356a78` | **exige la clé `size`** (sinon aucune requête) ; tampon `size + 1` ; type = clé `type`, **2 par défaut** ; `getReport(type, id)` ; refuse la réponse si `buf[0] != id` ; rend un `OSData` de `size` octets |
| Écriture | `setExtendedReport(nom, données, n)` `0xfffffe000a356f84` | tampon `[id] + n octets` ; type par défaut 2 ; `setReport` ; aucune vérification de `min`/`max` dans le noyau |
| Conséquence | — | les entrées sans `size` du 598 (`0x40 0x41 0x44 0x45 0x50`) ne peuvent **pas** être lues par le noyau, seulement écrites ; `0x55` (64 o) et `0x51-0x54` (8 o) sont lisibles par ce chemin mais **aucun appelant noyau** ne le fait |

Appelants noyau de `getExtendedReport` : `updateBatteryLevel` (`"BatteryPercent"`) et `getBatteryState` (`"BatteryState"`).
Appelant noyau de `setExtendedReport` : `willShutdown` (`"WillShutdown"`). **Aucun autre** (recherche des appels directs et des appels virtuels `vtable+0xc40`/`+0xc48` dans tout le pseudo-C des trois kexts).

Batterie **[décompilé]** : `batteryLevelTimerFired` (`0xfffffe000a355034`) enchaîne `updateBatteryLevel` puis
`getBatteryState` + `updateBatteryState` ; si l'un échoue, nouvelle tentative dans **3 600 000 ms**, sinon dans la période
(`deviceReady` : 14 400 000 ms). `updateBatteryState` ne réagit qu'à un **changement** d'état ; il relit alors `0x47`,
met à jour `BatteryLow`/`BatteryPanic`, envoie `'btpn'` (2-3), `'btlw'` (1) ou `'rbsn'` (0), puis la notification chaîne.
`AppleBluetoothHIDKeyboard` 9410.2 ne surcharge plus `updateBatteryLevel` (3 méthodes seulement : `init`, `handleReport`,
`processInterruptData`) : `0x49`/`0x60` (lus par Lion) ne sont plus lus **[symboles + décompilé]**.

Entrées décodées par le noyau **[décompilé]** :

* `IOAppleBluetoothHIDDriver::processInterruptData` (`0xfffffe000a355388`) : paquet de **3 octets exactement** `A1 30 xx` →
  `updateBatteryState(xx)` ; tout le reste part à la classe de base ;
* `AppleBluetoothHIDKeyboard::processInterruptData` (`0xfffffe00090758a0`) : 3 octets `A1 13 xx` avec le **bit 1 de `xx` à 0** et
  `DisablePoweredOffCheck` faux → notification `KeyboardOff` **et** drapeau « déconnexion attendue » (`+0x169`), ce qui
  **supprime la notification `Disconnected`** (et le message IOKit associé) émise par `willTerminateWL` (`0xfffffe000a363c50`) quand la liaison tombe ensuite ;
* aucun autre ID d'entrée (`0x04`, `0x05`…) n'est testé.

`IOBluetoothHIDDriver::newReportDescriptor` (`0xfffffe000a35afd0`) recopie la propriété `ReportDescriptor` (fournie par
`bluetoothd` depuis le SDP) **sans la modifier** : macOS ne corrige pas le descripteur de l'A1314. Les entrées non déclarées
(`0x30`, `0x04`, `0x05`) n'existent pour lui que par les deux tests ci-dessus ; `A1 30 xx` est **consommé** par
`IOAppleBluetoothHIDDriver::processInterruptData` et n'atteint pas IOHID, `A1 13 xx` y est transmis après le test **[décompilé]**.

### 4.1 `0x09` : ce qui est écrit, à qui, et par quel chemin

Deux chemins indépendants, **aucun** ne vise le 598 **[décompilé + octets lus dans le KC]** :

| Chemin | Fonction | Condition | Octets |
|---|---|---|---|
| Bluetooth, à la connexion | `IOAppleBluetoothHIDDriver::handleStart` `0xfffffe000a35456c` (`setCapsLockDelay` dans le journal) | PID `0x22C`-`0x22E` (claviers 2007) **et** `VersionNumber` ≥ `0x137` | tampon constant `09 01 00 00` (lu à `0xfffffe00078f2878`), `setReport(Feature)` : fil `53 09 01 00 00` |
| IOHID, au démarrage de la couche clavier | `AppleHIDKeyboardEventDriver::turnOffCapsLockDelay` `0xfffffe000906fe08`, appelée par `handleStart` | propriété `FWCapsLockDelay` vraie ; pour les PID `0x220-0x222` et `0x24F-0x251` (claviers USB alu.), version ≥ `0x68` en plus | `IOHIDInterface::setReport(Feature, id 9, 01 00 00)` |

Dans les deux cas Apple **écrit `01`** pour désactiver le délai Verr. Maj du micrologiciel, puis applique son propre délai :
`CapsLockDelay` = **75 ms** (`0x4B`, d'où les deux `mov w2,#0x4b` de `AppleHIDKeyboardEventDriver::handleStart`, qui ne sont
**pas** des références au rapport `0x4B`). Le 598 reçoit `CapsLockDelay` = 75 par son plist, sans écriture de `0x09`.

## 5. Recherche active de ce qui manquait

Balayage **de toutes les instructions** des six kexts (15 582 opérandes immédiats retenus, `scalars.tsv`) et de leurs segments
de données, plus le pseudo-C des trois kexts du pilote.

| Recherché | Résultat | Preuve |
|---|---|---|
| IDs `0xD0 0xD4 0xD5 0xFA 0xFB` (Feature refusés) | **aucune** comparaison, aucun chargement d'ID, aucune table dans `IOBluetoothHIDDriver`, `AppleBluetoothHIDKeyboard`, `AppleHIDKeyboard`. Les seuls `cmp #0xd0/#0xfa/#0xf7` sont dans `IOBluetoothFamily` `_ParseVendorSpecificCommand` (`0xfffffe000a32ceac`…) : décodage des **commandes HCI vendeur du contrôleur du Mac** (formats `bbbbbbHHHbbb`), sans rapport avec le clavier | [désassemblage] |
| IDs `0x4B 0xD1 0xD8 0xF6 0xF7` (lus, sens inconnu) | aucune référence comme ID de rapport ; `0x4B` = 75 ms (`CapsLockDelay`) ; `0xD8` = offsets de structure | [désassemblage + décompilé] |
| Entrées `0x04`, `0x05` | non testées ; seuls `0x30` (Apple) et `0x13` (clavier) le sont | [décompilé] |
| PID `0x255-0x257`, `0x239-0x23B` | **aucune** comparaison dans le noyau : le 598 n'est désigné que par sa personnalité (plist). Les PID testés en code sont `0x22C-0x22E` (`0x09`), `0x220-0x222`/`0x24F-0x251` (`FWCapsLockDelay`), `0x309` (`IOAppleBluetoothHIDDriver::getProtocol`, protocole inversé si version < `0x40`), `0x265 0x267 0x269 0x26C 0x29A 0x29C 0x310` (filtre de réveil) | [désassemblage] |
| Fonctions oubliées par les études précédentes | `waitForHandshake`, `waitForOkToSend`, `handleTimeout`, `processControlData`, `DecodedHandshake`, `setPowerStateWL`, `powerStateHandler`, `deviceConnectTimerFired`, `getProtocol` (surcharge Apple), `IOBluetoothGamepadHIDDriver` (classe sœur, manettes), et toute la classe **`IOBluetoothHIDChannel`** (§5.1) | [symboles + décompilé] |
| Données | aucune table de PID ni d'IDs dans `__DATA`/`__DATA_CONST`/`__const` des kexts du pilote ; les deux occurrences `39 02` d'`IOHIDFamily` sont dans une table de codes de touches | [octets] |

### 5.1 `IOBluetoothHIDChannel` : un second pilote HID dans `IOBluetoothFamily`

`IOBluetoothFamily` 9.0.0 contient une classe `IOBluetoothHIDChannel` (109 fonctions, transport `KernelBluetoothHIDChannel`),
créée par `IOBluetoothL2CAPChannel::CreateHIDChannel` (`0xfffffe000a3464d0`). C'est une copie de `IOBluetoothHIDDriver`
(mêmes méthodes, mêmes commandes de `GetProtocol` à `SetAuthenticated` ; les journaux du disjoncteur « 3 consecutive timeout » y sont absents), **sans** couche Apple :
ni `ExtendedFeatures`, ni batterie, ni `WillShutdown`, ni `CapsLock` **[symboles + chaînes]**.
L'A1314 étant apparié par une personnalité `AppleBluetoothHIDKeyboard` (`IOProviderClass` `IOBluetoothL2CAPChannel`, PSM 17),
ses rapports vendeur passent par `IOAppleBluetoothHIDDriver`. La condition qui fait créer un `IOBluetoothHIDChannel` à la place
n'a pas été établie (appel virtuel, aucun appelant direct) **[non établi]**. Le chemin « HIDShim » qui relie les deux au démon
(`IOBluetoothHCIController::CreateHIDShimDevice`, appelé par `IOBluetoothHCIUserClient::DispatchCreateHIDShimDevice`, anneaux
en mémoire partagée) confirme que **`bluetoothd` crée l'appareil HID noyau** et porte le L2CAP **[décompilé]**.

## 6. Diff x86_64 (`BootKernelExtensions.kc`) ↔ arm64e (kernelcache démarré)

Même chaîne Ghidra sur le KC x86_64 (204 entrées, 167 185 symboles posés, 111 069 démanglés) **[décompilé]** :

* `IOBluetoothHIDDriver` (198 fonctions x86 / 224 arm64e), `AppleBluetoothHIDKeyboard` (25 / 30) : **mêmes méthodes nommées** ;
  l'écart de nombre vient des fonctions de liaison propres à arm64e (`_vfpthunk_`, `_OUTLINED_FUNCTION_n`, PAC).
* `AppleHIDKeyboard` : seule `AppleHIDKeyboardEventDriverV2::handleStart` n'existe qu'en arm64e (claviers intégrés, sans objet).
* Mêmes comportements relus côte à côte : `handleSleep` (aucun envoi, compteur à 1, mode après veille), disjoncteur et
  `SetHIDDriverReady(false)` dans `waitForData`/`waitForHandshake`/`waitForOkToSend`, test du drapeau dans
  `getReport`/`setReport`/`sendData`, délais `0x1194`/`0x157c`/`+1000`, cadences batterie, condition `0x22C-0x22E` ≥ `0x137`.
* Aucune différence de fond trouvée pour le 598 : les conclusions de §2-§5 valent pour les deux architectures.

## 7. Table de synthèse : registre / commande → fonction → preuve → risque

Seules les lignes **nouvelles ou corrigées** par rapport à `RE-PILOTE-MACOS.md` §8, `RE-MACOS-SILICON.md` §7 et
`RE-PILOTES-ANCIENS.md` §7.

| Registre / commande | Fonction Apple (arm64e) | Preuve | Fait nouveau | Risque d'écriture / conséquence projet |
|---|---|---|---|---|
| GET ×3 sans réponse | `waitForData` `0xfffffe000a35daa0` (+ `waitForHandshake`, `waitForOkToSend`) | [décompilé] | disjoncteur **+ demande de déconnexion à `bluetoothd`** (`SetHIDDriverReady(false)`) ; 2 expirations suffisent après une veille | à imiter dans #243 : couper les requêtes **et** laisser tomber la liaison plutôt qu'insister |
| Refus HANDSHAKE (`0x03`…) | `DecodedHandshake` `0xfffffe000a35e324` | [décompilé] | compte comme réponse, remet le compteur à 0 | nos 11 IDs « sans GET » ne sont pas la cause des coupures (#175) ; seul le silence l'est |
| Délais | `waitForData`, `waitForHandshake`, `deviceReady` | [décompilé] | GET/SET : 3 500 ms + garde 1 000 ms ; après veille 4 500/5 500 ms ; SET_PROTOCOL initial 10 000 ms | nos délais (3 s BlueZ, 5 s noyau) sont du même ordre |
| HID_CONTROL `0x13` | `hidControl` `0xfffffe000a35f488` ← `processCommandWL` (`HIDSuspend`) | [décompilé] | jamais émis par le noyau de lui-même ; seulement sur commande de `bluetoothd` | faible |
| HID_CONTROL `0x14` | `processCommandWL` (`HIDExitSuspend`) | [décompilé] | **jamais émis par le noyau** (sauf commande brute `HIDControl 4`) ; la requête de `bluetoothd` au réveil n'émet rien pour un appareil piloté par le noyau : sortie de veille implicite | #244 : `0x14` reste conforme au protocole HID, mais ce n'est pas ce que fait macOS 26.5 pour l'A1314 |
| HID_CONTROL `0x15` reçu | `processControlData` `0xfffffe000a35c898` | [décompilé] | fermeture des deux canaux puis `closeConnection` | lecture passive : à journaliser si BlueZ le remonte |
| `0x30` (Input) | `IOAppleBluetoothHIDDriver::processInterruptData` `0xfffffe000a355388` | [décompilé] | exactement 3 octets `A1 30 xx`, consommé (n'atteint pas IOHID) ; réaction **au changement** seulement | lecture passive (#189) |
| `0x13` (Input) bit 1 = 0 | `AppleBluetoothHIDKeyboard::processInterruptData` `0xfffffe00090758a0` | [décompilé] | `KeyboardOff` **et** suppression de la notification `Disconnected` qui suit | #190 : ne pas alerter « liaison perdue » après un `0x13` bit 1 = 0 |
| `0x40` `WillShutdown` | `willShutdown` `0xfffffe000a35674c` → `setExtendedReport` `0xfffffe000a356f84` | [décompilé] | seul SET Feature émis d'office par le noyau ; clé `value` absente → `53 40` | faible (#191) |
| `0x41 0x44 0x45 0x50` | `getExtendedReport` `0xfffffe000a356a78` | [décompilé] | illisibles par construction (pas de `size`) ; aucun appel noyau en écriture | inchangé : interdits / accord requis |
| `0x09` = `09 01 00 00` | `IOAppleBluetoothHIDDriver::handleStart` `0xfffffe000a35456c` ; `AppleHIDKeyboardEventDriver::turnOffCapsLockDelay` `0xfffffe000906fe08` | [décompilé + octets] | `01` = délai firmware **désactivé** ; 2007 BT (`0x22C-0x22E`, ≥ `0x137`) et USB alu. avec `FWCapsLockDelay` ; jamais le 598 | faible ; ne pas écrire |
| `0xD0 0xD4 0xD5 0xFA 0xFB`, `0x4B 0xD1 0xD8 0xF6 0xF7`, Input `0x04 0x05` | — | [désassemblage, 6 kexts] | **aucune** référence noyau (balayage exhaustif des immédiats et des données) | inchangé : ne jamais écrire les cinq premiers |
| Commandes texte | `setProperties` `0xfffffe000a35b658` → `processCommandWL` ×2 | [décompilé] | 34 commandes (§3), `HIDControl n` brut | sans objet sous Linux ; référence pour l'équivalence |

## 8. Ce qui reste inconnu

1. Sens de `0xD0 0xD4 0xD5 0xFA 0xFB` (écriture) et de `0x4B 0xD1 0xD8 0xF6 0xF7`, Input `0x04`/`0x05` : **aucune** référence dans
   le noyau 26.5, x86_64 comme arm64e, au décompilateur. Seule une lecture du micrologiciel (#184/#185) peut trancher.
2. Ce que fait `bluetoothd` à réception de `HIDShimDeviceCreated = false` (déconnexion immédiate ? délai ?) : côté démon, non décompilé ici.
3. Condition de création d'un `IOBluetoothHIDChannel` au lieu de l'appariement `AppleBluetoothHIDKeyboard` (appel virtuel non résolu).
4. Écrivain du drapeau du filtre de touches au réveil (`+0x68`) : introuvable dans les deux architectures, filtre considéré inactif.
5. Contrôle d'accès IOKit en amont de `setProperties` (qui peut envoyer `HIDControl 1` ou `VirtualCableUnplug`) : non étudié.
6. Effets sur le clavier de `WillShutdown`, `RecantConnection`, `FactoryDefault` : non observables sans écriture (#182).

## 9. Reproduire (lecture seule, sans root)

1. Mac : `sysctl kern.bootobjectspath` → copier `…/boot/<hash>/System/Library/Caches/com.apple.kernelcaches/kernelcache`
   et `/System/Library/KernelCollections/BootKernelExtensions.kc` hors de tout dépôt.
2. Poste : venv avec `pyimg4 pyliblzfse jpype1` et `pyghidra` (roue fournie dans `/opt/ghidra/Ghidra/Features/PyGhidra/pypkg/dist`),
   `GHIDRA_INSTALL_DIR=/opt/ghidra JAVA_HOME=/usr/lib/jvm/java-21-openjdk`.
3. `kc_extract.py decode kernelcache kc.arm64e` ; `ghidra_import.py kc.arm64e proj kc <kexts>` ; `ghidra_symbols.py kc.arm64e proj kc` ;
   `ghidra_dump.py kc.arm64e proj kc out '<regex>' <kexts>` ; `annotate_vcalls.py out com.apple.driver.IOBluetoothHIDDriver AppleBluetoothHIDKeyboard`.
4. Lire `out/decomp/…` localement ; **ne jamais** commiter `out/`, le projet Ghidra ni les KC.

## 10. Suivi Gitea

| Issue | Objet |
|---|---|
| #243 | disjoncteur complet (commenté : test dans `sendData`, refus HANDSHAKE = réponse) |
| #244 | veille SUSPEND/EXIT_SUSPEND (commenté : `HIDExitSuspend` n'émet pas `0x14`) |
| #190 | extinction `0x13` bit 1 (commenté : `Disconnected` supprimée ensuite) |
| #175 | coupures (commenté : seul le silence arme le disjoncteur) |
| #245 | docs — corrections issues de ce document |
