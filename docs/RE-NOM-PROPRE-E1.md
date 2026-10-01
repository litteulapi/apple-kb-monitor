# E1 — Ce que Lion envoie réellement pour renommer un clavier Apple (SET Feature `0x55`)

Expérience E1 de `RENOMMER-CLAVIER.md` §5.4 : lever les inconnues U1, U2, U7 (et éclairer U4, U6) par le
désassemblage de `-[AppleBluetoothHIDDevice setDeviceName:]` dans `IOBluetooth.framework` de Mac OS X 10.7.5,
comparé à 10.5.8. Suivi : #248 (écriture du nom propre), #192 (F40).

Cadre : analyse de logiciels Apple publiquement distribués, pour l'interopérabilité avec le clavier du gérant.
Aucun binaire, aucun pseudo-C ni listing n'est commité : seulement des faits courts avec adresses.
Aucun accès au clavier, aucune écriture HID.

Niveaux de preuve : **[désassemblage]** instruction lue au listing (adresse donnée), **[décompilé]** structure lue au
décompilateur Ghidra 12.1.2 et recontrôlée au listing, **[plist]** personnalité du kext, **[déduction]** conséquence
non lue dans le code, **[inconnu]**.

## 1. Sources et méthode

| Élément | Origine (serveurs publics Apple, lus en flux, rien de conservé après analyse) | Empreinte |
|---|---|---|
| `MacOSXUpdCombo10.7.5.pkg` (produit `041-98155`, 2 041 831 704 o) | `http://swcdn.apple.com/content/downloads/30/25/041-98155-A_E2HC6UPBOQ/y8k2fq4q0u2jo4ci8txpxkw96l9dzfsi19/MacOSXUpdCombo10.7.5.pkg` | TOC xar SHA-1 OK ; 130 169 entrées cpio, 7 conservées |
| `IOBluetooth.framework/Versions/A/IOBluetooth` 10.7.5 (universel i386 + x86_64, 2 686 592 o) | idem | sha256 `c41ed5a7…cdab4` ; tranche x86_64 `db58c4e4…c61c3` (1 433 744 o) |
| `Bluetooth.prefPane/Contents/MacOS/Bluetooth` 10.7.5 | idem | sha256 `bb208a96…a8c1` |
| `IOBluetoothHIDDriver.kext` 10.7.5 | idem | sha256 `cff55159…d706` |
| `AppleBluetoothHIDKeyboard.kext/Contents/Info.plist` 160.7 | idem | personnalité « Wireless Keyboard 2009 B ISO » PID 598 |
| `MacOSXUpdCombo10.5.8.pkg` (produit `041-98113`) → `IOBluetooth` 10.5.8 (universel, tranche i386) | `http://swcdn.apple.com/content/downloads/33/32/041-98113-A_TI33BOXTMO/dqoewhlu9sd8s38j8who477umcenckft3q/MacOSXUpdCombo10.5.8.pkg` | sha256 `68fc9c08…b4b1` ; tranche i386 `c7f2e14e…63d7` |
| 10.6.8 | `MacOSXClientCombo10.6.8.pkg` et `SUBaseSystemCombo10.6.8.pkg` ne contiennent **pas** le binaire `IOBluetooth.framework` (vérifié sur les 2 flux cpio) : pas de comparaison 10.6.8 possible depuis les serveurs publics | — |
| macOS 26.5 | cache dyld non extrait sur ce poste : non refait ici ; `RE-MACOS-SILICON.md` §3.4 reste la référence (`setDeviceName:` n'écrit plus rien) | — |

Outils : extracteur xar/cpio en flux (`curl -r` + `gzip -dc`, filtre sur le chemin), `llvm-lipo`-équivalent (découpe de la
tranche), Ghidra 12.1.2 headless (analyse complète + script `DumpFuncs` : décompilé, listing, appelants, références de
chaînes), `llvm-objdump --macho -d` pour recontrôler chaque instruction citée, `llvm-nm` (symboles locaux présents :
les frameworks 10.5/10.7 ne sont pas dépouillés). Les adresses ci-dessous sont celles de la tranche x86_64 10.7.5
(base `0x0`, `LC_VERSION_MIN_MACOSX 10.7`), sauf mention « 10.5.8 ».

## 2. Appelant : Préférences Système → Bluetooth → « Renommer »

`Bluetooth.prefPane` 10.7.5 **[décompilé]** :

* `-[BluetoothPref handleDeviceRename:]` (`0xb438`) : construit un `AppleBluetoothHIDDevice` depuis l'appareil
  sélectionné (`initWithBluetoothDevice:`) ; si c'est un HID Apple, la limite du champ de saisie est
  `getMaxDeviceNameLength` (**64** pour le 598), sinon **32** ; posée par `+[HIDNameFormatter maxLengthFormatter:]`.
* `-[BluetoothPref handleDeviceRenameOK:]` (`0xb2a9`) : nom non vide **et** différent de `-[IOBluetoothDevice name]`,
  puis `-[AppleBluetoothHIDDevice setDeviceName:]` (sélecteur à `0xb3b8`) ; pour un appareil non Apple,
  `-[IOBluetoothDevice setDisplayName:]` seulement (cache hôte).
* Aucun autre appelant de `setDeviceName:` dans `IOBluetoothUI` 10.7.5 ni dans `blued` 10.7.5 (références de
  chaînes et de sélecteurs balayées) ; `BluetoothSetupAssistant` n'est pas dans le Combo.
* `-[BluetoothPref handleDeviceNameChanged:]` (`0x9f1b`) écoute la notification `IOBluetoothDeviceNameChanged`
  (déjà présente en 10.5.8, `Bluetooth` i386 `0x1a43c`).

## 3. `-[AppleBluetoothHIDDevice setDeviceName:]` 10.7.5 (`0x4d2fe`, 1 211 o)

### 3.1 Choix du chemin

| Étape | Fait | Preuve |
|---|---|---|
| 0 | si le nouveau nom `isEqualToString:` `-[BluetoothHIDDevice name]` → retour `0`, **aucune trame** | [désassemblage] `0x4d33a`, `0x4d34c` |
| 1 | `reportIDForReportKey:@"LongDeviceName"` (`0x4c7bc` : `featureDict[clé][@"id"] intValue`, −1 si absent) ; `> 0` → chemin **`0x55`** (§3.2) ; sinon chemin 4 fragments (§3.4) | [désassemblage] `0x4d375`-`0x4d37f` (cfstring `LongDeviceName` à `0x1072c0`) |

Pour le PID 598, `LongDeviceName` = `{id 85, size 64, type 2}` **[plist]** : c'est le chemin `0x55`.

### 3.2 Chemin `0x55` : construction du tampon

| # | Fait | Preuve |
|---|---|---|
| a | `maxLen = getMaxDeviceNameLength` (`0x4c6e4`) : `report:info:` (`0x4c8f2`) lit les clés `id`, `size`, `min`, `max` (défauts −1, 0, 0, 0) → **64** ; 64 aussi si `report:info:` échoue ; 32 si `LongDeviceName` est absent | [désassemblage] `0x4d3b9`, `movzbl %al,%r15d` à `0x4d3be` ; `0x4c963`-`0x4c9b3` |
| b | `[nom length]` (unités UTF-16) **> 64** → retour `0xE00002C2` (`kIOReturnBadArgument`), rien envoyé ; `== 0` → idem | [désassemblage] `cmpq %r15,%rax ; jbe` à `0x4d3d3`-`0x4d3d6` ; `0x4d3d8` ; `testq ; je` à `0x4d3f3` |
| c | **`calloc(maxLen + 1, 1)` = 65 octets à zéro** | [désassemblage] `incb %r12b` `0x4d3fb`, `movzbl %r12b,%edi ; movl $1,%esi ; call _calloc` `0x4d3fe`-`0x4d407` |
| d | `_UTF8StringFromString(nom, 64)` (`0x4d9be`) : `[NSMutableString stringWithString:]`, `UTF8String`, puis **tant que `strlen > 64`, supprime le dernier caractère** (`deleteCharactersInRange:`) → chaîne **UTF-8**, ≤ 64 octets, jamais coupée au milieu d'un caractère | [désassemblage] `movl %r15d,%esi ; call _UTF8StringFromString` `0x4d413`-`0x4d416` ; [décompilé] boucle `strlen`/`length`/`deleteCharactersInRange:` |
| e | `strncpy(buf + 1, utf8, strlen(utf8))` : copie exactement les octets du nom, **sans terminateur** (le NUL vient du `calloc`) | [désassemblage] `call _strlen` `0x4d430`, `leaq 1(%r13),%rdi ; movq %rax,%rdx ; call _strncpy` `0x4d435`-`0x4d43f` |
| f | `buf[0] = id` (**`0x55`**) | [désassemblage] `movb %bl,(%r13)` `0x4d444` |
| g | `hidDeviceInterfaceOpen` (`0x49168` : `open(0)` une fois) ; `hidDeviceInterface` (`0x491d5`, `CreateHIDDeviceInterface`) | [désassemblage] `0x4d452`, `0x4d477` |
| h | **`setReport(iface, type 2 = Feature, id 0x55, buf, 65, 1000 ms, NULL, NULL, NULL)`** via la vtable `+0x90` d'`IOHIDDeviceInterface` | [désassemblage] `movq 0x90(%rax),%r15` `0x4d47f` ; `movzbl %r12b,%r8d` (65) `0x4d4b1` ; `movl $2,%esi` `0x4d4b5` ; `movl $0x3e8,%r9d` `0x4d4ba` ; `movl %ebx,%edx` (id) `0x4d4c3` ; `movq %r13,%rcx` (buf) `0x4d4c5` ; 3 × `movq $0,(%rsp…)` `0x4d497`-`0x4d4a9` ; `call *%r15` `0x4d4c8` |

Le délai de **1000 ms** est le délai d'attente du `setReport` (attente du HANDSHAKE par le noyau), pas un espacement
entre trames : il n'y a qu'**une** trame.

### 3.3 Chemin `0x55` : après l'écriture

| # | Fait | Preuve |
|---|---|---|
| i | `setReport ≠ 0` → `NSLog("[setDeviceName] Could not set DeviceName")`, retour `0xE00002BC` (`kIOReturnError`) ; `free(buf)` | [désassemblage] `testl %eax,%eax ; je` `0x4d4cb`, cfstring `0x107820` à `0x4d4d1` |
| j | `setReport == 0` → `-[IOBluetoothDevice remoteNameRequest:self]` (`0x17091` → `remoteNameRequest:withPageTimeout:0` `0x16eba` : **HCI Remote Name Request**, délégué `remoteNameRequestComplete:status:`) ; erreur → `NSLog("[setDeviceName] Error returned from remoteNameRequest")`, retour du code | [désassemblage] `0x4d4e6`-`0x4d4f8`, cfstring `0x107840` |
| k | puis `-[IOBluetoothDevice setDisplayName:nom]` (cache hôte) ; retour `0` | [désassemblage] `0x4d751`-`0x4d764` |
| l | à la fin de la requête HCI : `-[AppleBluetoothHIDDevice remoteNameRequestComplete:status:]` (`0x4da65`) pose la propriété `ForceReadDeviceName` sur le service HID (`IORegistryEntrySetCFProperties`) ; dans le kext, `setProperties` (`0x75ba`) la route vers l'interface de commandes texte (`processCommandWL`, chaîne comparée à `0x495c`) qui appelle `readDeviceName` (vtable `+0xae0`) : **`IOBluetoothHIDDriver::readDeviceName` renvoie 1 sans rien faire** en 10.5.8 (`0x3fe8` i386), 10.6.8 (`0x3006` x86_64) et 10.7.5 (`0x344c`) ; aucune surcharge dans `AppleBluetoothHIDKeyboard` (10.6.8 vérifié ; 10.7.5 : kext non extrait, [déduction]) | [décompilé] + [désassemblage] |

**Il n'y a ni lecture de `0x51`-`0x54` ni écriture de `0x50` sur ce chemin, ni avant ni après**, aucun délai
inter-trames, aucune condition sur le PID ou la version du micrologiciel au-delà de la présence de la clé
`LongDeviceName` dans la personnalité. La confirmation attendue par Apple est le **nom distant HCI** (nom convivial
Bluetooth) demandé immédiatement après le HANDSHAKE, sans reconnexion.

### 3.4 Chemin sans `0x55` (claviers 2007/2009 « A », PID 0x022C-0x022E, 0x0239-0x023B) — pour mémoire

| # | Fait | Preuve |
|---|---|---|
| 1 | `[nom length] > 32` ou `== 0` → `0xE00002C2` | [désassemblage] `cmpq $0x20,%rax` `0x4d52b` |
| 2 | `_UTF8StringFromString(nom, 32)` ; `strncpy(local, utf8, 32)` (bourrage NUL par `strncpy`) | [désassemblage] `movl $0x20,%esi` `0x4d54f`, `0x4d558`, `movl $0x20,%edx` `0x4d56a`, `0x4d575` |
| 3 | pour `n` = 1..4 : clé `DeviceName%d` (cfstring `0x107760`) → id (`0x51`…`0x54`) ; `strncpy(frag, local + 8(n−1), 8)` ; `frag[−1] = id` ; `setReport(2, id, frag, 9, 1000)` ; arrêt au premier échec | [désassemblage] `0x4d598`-`0x4d62c` (`movl $2,%esi` `0x4d611`, `movl $9,%r8d` `0x4d620`, `movl $0x3e8,%r9d` `0x4d626`) |
| 4 | puis `setFeatureReport:@"DeviceNameChange" value:0` → `size` absent = 0 → **1 octet : `50`** (`setReport(2, 0x50, {0x50}, 1, 1000)`) ; échec → « Could not commit name change in device » | [désassemblage] `0x4d6bf`-`0x4d6cb` ; `setFeatureReport:value:` `0x4caf0` : octet 0 = id, `size + 1` octets |
| 5 | puis `remoteNameRequest:` et `setDisplayName:` comme en §3.3 | [désassemblage] `0x4d703`-`0x4d79e` |

Donc **U7** : `0x50` `DeviceNameChange` ne sert qu'à valider les 4 fragments ; il n'est **jamais** envoyé à un clavier
qui déclare `LongDeviceName`. Rien dans ces binaires ne l'envoie à notre PID.

### 3.5 Accesseurs voisins (10.7.5), pour l'inventaire

* `setFeatureReport:value:` (`0x4caf0`) : `report:info:` ; `size > 4` → « Report size not suitable » ; octet 0 = id,
  puis la valeur en **petit-boutiste** sur `size` octets (`switch` 4→1, octet 1 = poids faible) ; `setReport(2, id, buf,
  size + 1, 1000)`. `size` = 0 → id seul. Pas d'exception par PID (celles de 26.5 n'existent pas encore). [décompilé]
* `getFeatureReport:` (`0x4cc3c`) : `size` 1..4 ; tampon de 10, longueur entrée/sortie 10 ; `getReport` vtable `+0x98`,
  1000 ms ; vérifie `octet0 == id` ; assemble en **gros-boutiste** (`v = v·256 + octet`). Même asymétrie qu'en 26.5. [décompilé]
* `setFloatFeatureReport:value:` (`0x4cdc8`), `getFloatFeatureReport:` (`0x4ce7c`) : normalisation `min`/`max`,
  appellent les deux précédents. Aucun lien avec le nom.
* `sendCommandFeatureReport:` **n'existe pas** en 10.7.5 (aucun symbole ni chaîne ; ajout postérieur) :
  `fullFactoryDefault` (`0x4d0ed`), `factoryDefault` (`0x4d0b7`), `recantConnection` (`0x4d058`) appellent
  directement `setFeatureReport:value:0` (appels à `0x4d101`, `0x4d0cb`, `0x4d090`). `deleteAllLinkKeys` (`0x4d238`)
  et `setFeatureWithReportID:value:` (`0x4ca33`) existent aussi ; sans lien avec le nom.
* `deviceNameFromHardware` (`0x4d7b9`) : GET `DeviceName1..4`, longueur entrée/sortie initialisée à 10 **une seule
  fois**, copie `longueur − 2` octets par morceau depuis l'octet 1 (comme en 26.5) ; ses appelants ne sont pas dans
  `setDeviceName:`.

### 3.6 Noyau 10.7.5 : `IOBluetoothHIDDriver::setReportWL` (`0x5ad6`) — ce qui part sur le fil

* Rapport Feature (type 2) → canal de **contrôle**, octet de tête **`0x53`** (SET_REPORT | Feature) ; autres types →
  canal d'interruption, tête `0xA0 | (type+1)` (DATA). [décompilé]
* Taille d'un morceau = **MTU sortante du canal − 1** (`getOutgoingMTU`, vtable `+0x9f0`) ; au-delà, continuation(s)
  `0xB0 | (type+1)` (DATC). Avec 65 octets utiles, une seule trame de **66 octets** dès que la MTU du canal de contrôle
  est ≥ 66 ; sinon Apple fragmentait en DATC. [décompilé]
* Pour un Feature, attente du HANDSHAKE (`+0xa90`) après envoi ; le `1000 ms` du framework borne cette attente. [décompilé]

## 4. Comparaison 10.5.8 (`IOBluetooth` i386, `-[AppleBluetoothHIDDevice setDeviceName:]` à `0x4fc10`, 1 062 o)

| Point | 10.5.8 | 10.7.5 |
|---|---|---|
| test `LongDeviceName`, chemin `0x55` | identique (`0x4fc66`-`0x4fc9b`) | identique |
| `getMaxDeviceNameLength` | `0x50036` : 64 / 32, même logique | `0x4c6e4` |
| tampon | **`calloc(maxLen + 2, 1)` = 66 octets** à zéro | `calloc(maxLen + 1, 1)` = 65 |
| conversion | `_UTF8StringFromString(nom, 64)` (`0x4fb57`, `regparm`) : même troncature UTF-8 par caractères | `0x4d9be` |
| copie | `strncpy(buf + 1, utf8, strlen)` ; `buf[0] = id` | identique |
| `setReport` | vtable **`+0x48`** (i386, même entrée 18), `(2, id, buf, maxLen + 1 = 65, 1000, 0, 0, 0)` à `0x4fde0` | `+0x90`, `0x4d4c8` |
| après | `remoteNameRequest:` ; pas de `setDisplayName:`, pas de `NSLog` | `remoteNameRequest:` puis `setDisplayName:` |
| chemin 4 fragments | `0x4fe33` `cmp 0x20`, 4 × 9 octets à `0x4ff8c`, puis `DeviceNameChange` `0x4ffa7` | identique |

Les **65 octets envoyés sont identiques** d'un système à l'autre ; seule la taille de l'allocation hôte diffère
(sans effet sur le fil). Le protocole est donc stable de 10.5 (2009) à 10.7.5 (2012).

## 5. Les sept inconnues (`RENOMMER-CLAVIER.md` §5.3)

| # | Inconnue | Verdict | Preuve |
|---|---|---|---|
| U1 | contenu des 64 octets | **tranchée** : octet 0 = `0x55` ; octets 1-64 = nom en **UTF-8** (≤ 64 o, tronqué par caractères), **bourré de `0x00`** (`calloc`) ; **pas** de préfixe de longueur, **pas** de terminateur explicite (un nom de 64 octets n'a aucun NUL) ; pas de MacRoman, pas d'espaces de bourrage | §3.2 c-f [désassemblage] |
| U2 | ordre des trames | **tranchée** : **une seule trame** SET Feature `0x55` ; puis HCI Remote Name Request (pas HID) ; aucun GET avant/après ; `0x50`, `0x51`-`0x54`, `0x44`, `0x41` jamais envoyés sur ce chemin | §3.2-3.3 [désassemblage] |
| U7 | rôle de `0x50` | **tranchée** : validation des 4 fragments uniquement ; jamais envoyé quand `LongDeviceName` existe | §3.4 [désassemblage] |
| U4 | `0x51`-`0x54` reflètent-ils `0x55`, et quand | **partiellement** : Apple ne relit **pas** `0x51`-`0x54` après l'écriture ; il attend le nouveau **nom HCI** immédiatement (sans reconnexion). Que les fragments reflètent `0x55` reste **[inconnu]** (mesure E3/relecture après écriture) | §3.3 j, l |
| U3 | persistance (piles) | **non tranchée** par le logiciel : aucune trace d'une réécriture à la reconnexion dans le framework, le prefPane ni `blued` (Apple n'écrit qu'à la demande de l'utilisateur) → [déduction] le clavier mémorise ; à mesurer (E3) | §2, §3 |
| U5 | réponse du micrologiciel `0x0050` à un SET `0x55` | **non tranchée** (matériel) ; côté hôte, Apple attend un HANDSHAKE dans les 1000 ms et traite tout autre résultat comme un échec sans réessai | §3.3 i |
| U6 | MTU / fragmentation | **côté Apple tranchée** : 1 trame de 66 o si MTU du canal de contrôle ≥ 66, sinon DATC par morceaux de MTU − 1 ; **côté Linux** (`hidp` n'écrit qu'un seul skb) : MTU négociée à mesurer (E2, `btmon`) | §3.6 |

Autres faits demandés : longueur maximale **64** (unités UTF-16 avant conversion, octets UTF-8 après) ; direction
**SET Feature** (type 2) ; gestion d'erreur : `kIOReturnBadArgument` (`0xE00002C2`) avant toute trame si longueur
invalide, `kIOReturnError` (`0xE00002BC`) si l'interface HID ne s'ouvre pas ou si `setReport` échoue, code HCI si la
requête de nom échoue ; conditions : seule la présence de `LongDeviceName` dans la personnalité (pas de test de PID ni
de version de micrologiciel) ; notification de fin : retour HCI du nom distant → `ForceReadDeviceName` (sans effet
noyau) et `IOBluetoothDeviceNameChanged` côté hôte.

## 6. Trames de référence (fichier `tests/fixtures/devname/lion_setdevicename_frames.json`)

Octets remis au noyau (65) puis sur le fil (66). Preuve par octet : `55` [désassemblage] `0x4d444` ; nom UTF-8
[désassemblage] `0x4d416`, `0x4d43f` ; bourrage `00` [désassemblage] `0x4d407` (`calloc`) ; longueur 65
[désassemblage] `0x4d4b1` ; `53` [décompilé] kext `0x5ad6`.

**« alex »** (4 o) :

```
report : 55 61 6c 65 78 00 × 60
wire   : 53 55 61 6c 65 78 00 × 60
```

**« Clavier Apple A1314 du gerant 01 »** (32 o exactement) :

```
report : 55 43 6c 61 76 69 65 72 20 41 70 70 6c 65 20 41 31 33 31 34 20 64 75 20 67 65 72 61 6e 74 20 30 31 00 × 32
wire   : 53 55 43 6c 61 76 69 65 72 20 41 70 70 6c 65 20 41 31 33 31 34 20 64 75 20 67 65 72 61 6e 74 20 30 31 00 × 32
```

La trame d'hypothèse U1 de `RENOMMER-CLAVIER.md` §5.2 (`Clavier de maria #1`, NUL-bourrée) est **confirmée octet pour
octet** par ce désassemblage : l'hypothèse devient un fait établi pour la construction hôte.

## 7. Conséquence pour `NotProven`

* **Construction de la trame** : établie sans ambiguïté par deux systèmes (10.5.8, 10.7.5) → le fichier de trames de
  référence peut servir de test octet pour octet à `devname::frames_for` et lever le refus **pour la construction**.
* **Ce qui reste matériel, hors de portée d'un désassemblage** : réponse HANDSHAKE du micrologiciel `0x0050` à un SET
  `0x55` (U5), reflet dans `0x51`-`0x54` (U4), persistance (U3), MTU Linux (U6, E2). Apple ne vérifiait rien de cela :
  une seule écriture, puis confiance au nom HCI. La séquence gardée de `RENOMMER-CLAVIER.md` §4 (sauvegarde, une
  écriture, relecture après reconnexion, retour arrière) reste donc la bonne enveloppe autour de la trame Apple.
* Recommandation : passer la preuve de **construction** à « établie » ; conserver les garde-fous d'exécution et
  réaliser E2 (MTU) avant la première écriture réelle, sur accord écrit du gérant. L'implémentation relève d'un autre
  agent (#248).

## 8. Reproduire (lecture seule, sans clavier, ~15 min)

1. Lire la TOC xar du Combo 10.7.5 (URL §1) par requêtes `Range`, vérifier son SHA-1, puis le `Payload` en flux
   (`gzip -dc` → cpio `odc`) en ne gardant que `IOBluetooth.framework/Versions/A/IOBluetooth`,
   `Bluetooth.prefPane/Contents/MacOS/Bluetooth`, `IOBluetoothHIDDriver.kext`.
2. Extraire la tranche x86_64 (`cputype 0x01000007`) ; `llvm-nm -n` donne `-[AppleBluetoothHIDDevice setDeviceName:]`
   à `0x4d2fe` et `deviceNameFromHardware` à `0x4d7b9` (fin).
3. `llvm-objdump --macho -d --no-show-raw-insn` : vérifier les instructions citées en §3.2 (`0x4d3fe`-`0x4d4c8`).
4. Ghidra 12.1.2 headless, analyse complète, décompiler `setDeviceName:`, `_UTF8StringFromString` (`0x4d9be`),
   `report:info:` (`0x4c8f2`), `setFeatureReport:value:` (`0x4caf0`) ; kext : `setReportWL` (`0x5ad6`),
   `readDeviceName` (`0x344c`).
5. Supprimer les binaires après lecture ; ne rien commiter d'autre que des faits.

## 9. Suivi Gitea

* #248 : commentaire avec ce document et le fichier de trames ; l'écriture réelle reste subordonnée aux garde-fous §7.
* #192 (F40) : `0x50` jamais envoyé à un clavier déclarant `LongDeviceName` (U7).
