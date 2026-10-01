# RE — Obtenir et désassembler le firmware du A1314 (BCM2042) : faisabilité et plan

Recherche **documentaire uniquement**, **sans aucun accès au clavier** (aucune lecture/écriture HID,
aucune commande Bluetooth émise vers lui). Objet : estimer de façon crédible et chiffrée ce qu'il
faudrait pour **obtenir le firmware** du clavier Apple A1314 ISO (puce Broadcom **BCM2042**) et le
**désassembler**, ce qui est lisible **sans ouvrir** le clavier, et ce qui exige une intervention
matérielle. Complète `RE-FIRMWARE-MAINTENANCE.md` (mise à jour / bootloader), `HARDWARE-RAPPORTS-HID.md`
et `RE-HID-EXHAUSTIF.md` (carte des rapports HID), qui restent les sources internes de référence.

Matériel visé : A1314 ISO « Clavier de maria #1 », `04:DB:56:CA:42:EE`, `0005:05AC:0256`,
bcdDevice `0x0050`, propriété du gérant → analyse d'interopérabilité **licite** sur SON matériel.
Aucune opération de ce document ne vise à contourner la protection d'un tiers ; rien n'y est exécuté.

Convention de preuve, appliquée à chaque ligne :
- **[source publique]** — attesté par une source citée (datasheet, teardown, article, code, exposé) ;
- **[déduction]** — tiré logiquement de sources publiques + de nos mesures internes ;
- **[spéculation]** — hypothèse plausible, non prouvée ; jamais présentée comme un fait.

---

## 0. Verdict de faisabilité (lire d'abord)

| Cible | Faisabilité | Effort | Ouvrir le clavier ? |
|---|---|---|---|
| **EEPROM I²C de config** (contenu : matrice, nom, paramètres, peut-être appairage) | **Réaliste** | 1–3 jours | **Oui** (desouder ou pince in-circuit) |
| **Boot ROM 20 Ko + ROM masquée 108 Ko** (firmware applicatif 8051) | **Difficile à très difficile** ; pas de dump public connu | semaines, incertain | Oui, + matériel/technique avancés |
| **Dump via radio Bluetooth (sans ouvrir)** | **Quasi nul** (voir §3.3) | — | Non, mais impasse |
| **Désassemblage 8051 une fois un binaire obtenu** | **Réaliste** | jours | Non (travail hors-ligne) |

**Verdict [déduction].** Le chemin crédible et à moindre risque est **l'EEPROM I²C externe** : c'est la
seule mémoire du clavier qu'on sait lire avec des outils courants, et son contenu est le plus utile pour
notre projet (matrice, nom, étalonnage, éventuels paramètres de veille). **La ROM/Boot ROM internes du
BCM2042 n'ont aucun dump public connu [déduction]** ; les sortir suppose soit un canal de debug matériel
(UART/HCI série sur le PCB, non documenté pour le A1314), soit des techniques invasives (glitch,
decap/microscope) coûteuses et risquées. **Il n'existe pas de voie radio** : les commandes Broadcom de
lecture mémoire (Read_RAM) sont des commandes **HCI côté contrôleur hôte**, pas atteignables sur un
clavier qui ne parle qu'HID par-dessus L2CAP (§3). **Le désassemblage 8051 est, lui, du ressort de
Ghidra** et ne pose pas de difficulté de principe (§4).

> ⚠️ **Politique projet inchangée : lecture seule, aucune écriture, aucun flash.** Ce document chiffre un
> chantier matériel que le **gérant seul** peut décider et exécuter physiquement. Rien ici ne doit être
> tenté sans sauvegarde préalable (dump EEPROM intégral **avant** toute écriture).

---

## 1. Identification matérielle (teardowns et références publiques)

### 1.1 Démontage du A1314 **[source publique]**

- Le A1314 est un empilement collé de **six couches** (de bas en haut) : embase ABS, couche adhésive,
  plaque d'acier fine, membrane/flex PCB des touches, dômes caoutchouc sur feuille plastique, coque
  aluminium supérieure. Ouverture : **ramollir l'adhésif à l'isopropanol** puis séparer l'embase de la
  coque alu au spatule/médiator. Il y a une **carte logique** (sous capot plastique) et une **carte
  antenne** reliée par un **câble plat à connecteur ZIF**. Le retrait de la carte logique est décrit comme
  « finicky and unnecessary » → **destructif si fait sans précaution**. **[source publique]** geekhack
  « Disassembling an Apple Wireless » (topic 31981) ; guide iFixit 45257 (remplacement carte logique).
- **[déduction]** L'ouverture est **irréversible proprement** : l'adhésif ne se recolle pas à l'identique.
  C'est le premier coût matériel (le clavier reste fonctionnel mais le boîtier est marqué).

### 1.2 Puces sur le PCB **[source publique]** + **[déduction]**

| Composant | Ce qui est attesté | Preuve |
|---|---|---|
| **SoC Bluetooth** | **Broadcom BCM2042**, boîtier **BGA**, intègre radio/transceiver + scanner de matrice clavier + baseband + cœur **8051** | [source publique] teardown iFixit (A1255, PCB quasi identique), product brief BCM2042 |
| **Mémoire interne** (dans le SoC) | **108 Ko ROM + 22 Ko RAM + 20 Ko Boot ROM**, design *ROM-based* « eliminates external flash » (« masquée » = déduction ; le brief offre aussi une « Flash option ») | [source publique] product brief `2042-PB03-R` / datasheet digchip |
| **Mémoire externe de config** | **EEPROM série I²C** (famille 24Cxx) ; sur modules BCM2042 tiers observés : **FM24C64** (PA-48-V3), **FM24C32** (geekhack « Wireless Model M ») ; teardown A1255 : **EEPROM/flash STMicroelectronics** | [source publique] iFixit A1255, keyglove, geekhack |

- **[déduction]** Le A1314 (2009) partage l'architecture du A1255/A1016 : **BCM2042 BGA + une petite EEPROM
  I²C externe**. La référence exacte de l'EEPROM du A1314 **n'est pas publiée** ; attendre un boîtier
  **SOIC-8 ou SOT-23** marqué d'un fabricant d'EEPROM (ST, Fairchild/FM24Cxx, Microchip 24LCxx) → **à
  confirmer visuellement à l'ouverture**. **[spéculation]** capacité probable 4–8 Ko (32–64 Kibit).

### 1.3 Points de test, debug, JTAG/UART, brochage **[déduction]** + **[spéculation]**

- **Aucun brochage ni pastille de debug publique n'existe pour le PCB A1314** (recherche 2026-10-01 :
  teardowns centrés sur la réparation, pas sur le circuit). **[déduction]**
- Le BCM2042 en BGA n'expose **pas** de bus externe parallèle pour un dump ROM « à la 8051dumper »
  (qui exige le bus d'adresses/données d'un 8051 classique). **[déduction]**
- **[source publique, ajouté au contre-audit]** La fiche du module Sunitec BM2042 (BCM2042KFB) documente une
  **UART de debug** (`UP_TX`/`UP_RX`) : « after power on reset, if UP_RX = 1, Boot-ROM waits for download of firmware ;
  UP_RX = 0, Boot-ROM launches firmware image in the external Flash or internal ROM ». Le mode téléchargement du Boot-ROM
  existe donc sur la puce. **[spéculation]** Sur le PCB du A1314 ces broches ne sont pas câblées sur un connecteur et
  leur emplacement n'est pas publié. Les
  retrouver supposerait de tracer des pastilles nues au microscope/multimètre, sans garantie.
- **Ce qui est réellement accessible au fer** : **les 2 lignes I²C (SDA/SCL) + VCC/GND de l'EEPROM**, plus
  éventuellement la broche **WP** (write-protect, vue sur les variantes 48 broches). **[source publique]**
  keyglove/geekhack. → c'est le seul point d'entrée matériel crédible (§2).

---

## 2. Ce qui est lisible **sans toucher à la puce** : l'EEPROM I²C externe

### 2.1 Contenu probable **[source publique]** + **[spéculation]**

- **Matrice de scan** : sur un module BCM2042 8×12, un contributeur a situé la table de matrice **« around
  0x148D »** de l'EEPROM. **[source publique]** keyglove. → l'EEPROM contient bien la **config clavier**.
- **[déduction]** Config BCM2042 typique en EEPROM : table de matrice/hot-keys, **nom du périphérique**
  (cohérent avec nos rapports `0x51-0x54` « Clavier de maria #1 »), paramètres radio, et souvent
  **BD_ADDR** et d'éventuels **patchs RAM** chargés au boot.
- **Appairage / clés de lien** : **[spéculation]**. Le rapport HID `0x4C` expose l'adresse de l'hôte +
  12 octets de forte entropie (voir `HARDWARE-RAPPORTS-HID.md` §2bis) ; **si** ce bond est persistant, il
  réside soit en zone NVRAM interne, soit en EEPROM. **Non prouvé.** → **risque d'effacer l'appairage** en
  cas d'écriture ; en **lecture seule** le dump EEPROM révélerait où il est stocké. **Ne jamais réécrire
  l'EEPROM sans dump intégral préalable.**

### 2.2 Méthodes de lecture, du moins au plus risqué

| Méthode | Principe | Risque | Remarque |
|---|---|---|---|
| **Pince SOIC-8 (in-circuit)** | clipser la pince sur l'EEPROM sans dessouder, lire via SDA/SCL | **moyen** : le BCM2042 peut piloter le bus en parallèle (conflit), VCC injecté peut réveiller le SoC | tenter **clavier hors tension (piles ôtées)** ; alimenter l'EEPROM seule en 3,3 V |
| **Dessoudage + lecture sur banc** | retirer l'EEPROM, la lire isolée, la ressouder | **élevé mécanique** (BGA voisin, pastilles fines) mais **lecture propre** | méthode des contributeurs geekhack/keyglove ; reste réversible si ressoudée |
| **Fil volant au fer** | souder 2 fils sur SDA/SCL (+ GND/VCC) | moyen | décrit par un contributeur (« 2 wires + I2C converter box ») [source publique] |

**[source publique]** keyglove (« I desoldered the chip and read out the memory with I2C »), geekhack
(dump FM24C32 au fer + microscope), Hackaday « Fix A Keyboard's Firmware With Trial, Error, And I2C ».

### 2.3 Matériel de lecture EEPROM

- **CH341A** (clé USB ~5 €) : lecteur I²C/SPI universel, support EEPROM 24Cxx dans `flashrom`/AsProgrammer.
  ⚠️ **beaucoup de CH341A sortent 5 V sur les I/O** → **risque de griller une EEPROM 3,3 V** : vérifier/mod
  3,3 V. **[source publique]** (notoriété flashrom/CH341A).
- **Bus Pirate** ou **Raspberry Pi** (I²C natif en 3,3 V, plus sûr côté tension) + `i2cdetect`/`i2cdump`.
- **Pince de test SOIC-8** (~10 €), **alim de labo 3,3 V**, **fer fine panne + flux + loupe/microscope**.
- **[déduction]** Budget total **~30–60 €** ; tout est générique et déjà courant en hardware hacking.

### 2.4 Risques spécifiques EEPROM

- **Brûler l'EEPROM** (surtension 5 V d'un CH341A non modifié). **[source publique]**
- **Effacer / corrompre l'appairage ou la matrice** en cas d'écriture accidentelle → clavier muet ou
  désappairé. **Mitigé** par : lecture seule, WP à la masse, **dump intégral conservé avant toute écriture**.
- **Réveiller le SoC** en alimentant le bus in-circuit → consommation/état indéfini : préférer **piles
  retirées**. **[déduction]**

---

## 3. La ROM interne du BCM2042 : dumps publics, SDK, et voies d'extraction

### 3.1 Existe-t-il des dumps publics ? **[déduction]**

- **Aucun dump ni désassemblage public du firmware BCM2042 d'un clavier Apple n'a été trouvé**
  (recherches 2026-10-01). La seule « source » qui documente les registres HID est **notre propre projet**
  (miroir `litteulapi/apple-kb-monitor`) — source circulaire, non retenue comme externe. **[déduction]**
- Les firmwares Broadcom publics concernent les **contrôleurs hôtes** (Wi-Fi/BT des Mac/téléphones :
  `NoaHimesaka1873/apple-bcm-firmware`, patchram `.hcd` de BrcmPatchRAM), **pas** le 8051 d'un clavier.
  **[source publique]**
- **[source non revérifiée au contre-audit]** (le *product brief* `2042-PB03-R` ne le mentionne pas ; la datasheet
  complète n'était pas accessible : alldatasheet 403) La **datasheet BCM2042** documenterait le
  **bank switching** du code 8051 — **zone commune `0x0000–0x7FFF`, zone bankée `0x8000–0xFFFF`,
  sélection de banque par les bits 3–2 du port P1**. C'est la carte mémoire à reproduire dans l'outil (§4).

### 3.2 Commandes vendeur Broadcom de lecture/écriture mémoire **[source publique]**

InternalBlue et BrcmPatchRAM documentent des **HCI Vendor Specific Commands** Broadcom :

| VSC | Opcode | Rôle |
|---|---|---|
| **Read_RAM** | `0xFC4D` | lire la RAM du contrôleur |
| **Write_RAM** | `0xFC4C` | écrire la RAM (appliquer un patch ; **non signé**) |
| **Launch_RAM** | `0xFC4E` | sortir du mode download / sauter en RAM |
| **Download_Minidriver** | `0xFC2E` | entrer en mode download (HCI réduit : Read/Write/Launch RAM) |

**[source publique]** InternalBlue (arXiv 1905.00631), `BrcmPatchRAM/hci.h`, naehrdine.blogspot
(« Unpatching the unpatchable »), Infineon community (0xfc2e/0xfc4c au boot du 43438).

### 3.3 Ces commandes sont-elles atteignables sur NOTRE clavier ? **[déduction]** — NON par radio

- **Point décisif.** Read_RAM/Write_RAM/Launch_RAM/Download_Minidriver sont des **commandes HCI**, émises
  sur le **transport contrôleur** (UART/USB/SDIO) **entre un hôte et SON propre contrôleur Bluetooth**.
  InternalBlue/Frankenstein pilotent **le contrôleur BT du téléphone/PC** (Nexus 5, RPi 3, dongles) — la
  puce **locale** dont on possède le transport HCI. **[source publique]**
- Notre A1314 est un **périphérique** : vis-à-vis du Mac/PC il ne parle **que l'HID par-dessus L2CAP**
  après appairage. **Il n'expose aucun canal HCI à l'hôte distant.** Sa propre couche HCI tourne en interne
  sur le 8051 et **n'est pas routée sur l'air**. **[déduction]** (cohérent avec `RE-FIRMWARE-MAINTENANCE.md`
  §2.5 : pas de HCI/DUT exposé sur un clavier HID appairé.)
- **Conséquence [déduction] :** aucune commande « Read RAM » n'est atteignable **depuis l'hôte via
  HID/L2CAP**. Pour parler HCI au BCM2042 du clavier il faudrait **son UART/transport HCI interne,
  physiquement sur le PCB** (non documenté, §1.3). → **pas de dump sans ouvrir, et même ouvert, canal HCI
  non localisé.**
- **ESP32 / Frankenstein / InternalBlue sur d'autres puces** : pertinents **méthodologiquement** (émulation,
  patch RAM Broadcom) mais ils ciblent des puces dont on **contrôle le transport HCI**, ce qui n'est pas le
  cas d'un clavier. **[déduction]**

### 3.4 Voies d'extraction réellement envisageables pour la ROM (toutes invasives)

| Voie | Principe | Faisabilité | Risque |
|---|---|---|---|
| **UART/HCI série sur le PCB** | retrouver les pastilles HCI internes, parler Download_Minidriver + Read_RAM | **spéculative** : pastilles non documentées, peut être désactivé en prod | ouverture + traçage microscope |
| **Boot ROM via UART (`UP_RX = 1`)** | mode téléchargement documenté par la fiche BM2042 ; reste à savoir s'il permet de **lire** | **spéculative** : broche non localisée sur le A1314, protocole non publié | risque de brique |
| **Glitching (fault injection)** | glitch d'alim pour contourner une éventuelle protection de lecture | **recherche**, semaines, matériel ChipWhisperer (~250 €+) | élevé |
| **Decap + lecture optique de la ROM masquée** | retirer le die, lire la ROM au microscope | **laboratoire**, destructif, coûteux | perte définitive de la puce |
| **EEPROM + patchs RAM** | si l'EEPROM contient des patchs RAM, on lit **une partie** du code exécuté | **réaliste partiel** (§2) | celui de l'EEPROM |

**[déduction]** La **ROM masquée 108 Ko** est le vrai cœur applicatif ; la sortir proprement est
**hors de portée** d'un effort raisonnable sans équipement de labo. **L'EEPROM (§2) donne le meilleur
rapport information/risque** et peut contenir des **patchs RAM** = fragments de code lisibles.

---

## 4. Outils d'analyse et désassemblage (hors-ligne, sans clavier)

### 4.1 Ghidra + 8051 **[source publique]**

- Ghidra fournit un **module processeur 8051 (MCS-51) natif** (`Ghidra/Processors/8051`). **[source publique]**
- **Bank switching** : le module stock **gère mal** le code banké >64 Ko ; précédent concret de contournement :
  `cyrozap/ghidra-asmedia-8051` (SFR DPX/PSBANK). Pour le BCM2042 il faut **modéliser la carte §3.1**
  (commun `0x0000–0x7FFF`, banké `0x8000–0xFFFF`, banque = bits 3–2 de P1). **[source publique]** issue
  Ghidra #7052.
- **SFR Broadcom** : définir les registres spéciaux propres au BCM2042 (ports GPIO LED, contrôleur I²C,
  matrice, radio) dans le `.sinc`/table SFR — **[spéculation]** cartographie à bâtir empiriquement, pas de
  table publique.

### 4.2 Émulation

- **Unicorn Engine** : pas de cœur 8051 en amont → passer par **MAME/MCS51** ou un émulateur 8051 dédié
  pour rejouer des handlers. **[déduction]** Émuler aide à comprendre les routines batterie/veille
  isolées, pas à tout exécuter (I/O radio non modélisée).
- **Frankenstein** (seemoo) : émulation de firmware Broadcom — **méthodologie** réutilisable si un binaire
  est obtenu, **pas** un dump prêt à l'emploi. **[source publique]**

### 4.3 Cartographie des handlers de rapports HID dans le firmware **[déduction]**

- Une fois un binaire (EEPROM et/ou ROM) chargé, chercher les **constantes de report ID déjà connues par
  mesure** (`0x47` batterie, `0x46/0x49/0x5A` tensions, `0x4C` appairage, `0x4F` version, `0x51-0x54` nom,
  `0xF4/0xF5`) comme **points d'ancrage** : repérer les tables de dispatch GET_REPORT/SET_REPORT, puis
  remonter aux routines de scan matrice et de gestion batterie. **[déduction]** — nos mesures HID
  (`RE-HID-EXHAUSTIF.md`) servent de **vérité terrain** pour valider le désassemblage.
- Analogues méthodologiques **[source publique]** : jamchamb « Dumping K360 firmware with a GreatFET »
  (dump + RE d'un clavier sans fil), 8051dumper (NF6X), SparkFun/Wiimote (8051 en clair sur EEPROM M24128).

---

## 5. Retour arrière, risques, difficulté, matériel, répartition des tâches

### 5.1 Risques et retour arrière

| Risque | Gravité | Retour arrière |
|---|---|---|
| Boîtier marqué à l'ouverture (adhésif) | cosmétique | aucun (irréversible proprement) [source publique] |
| Pastille/piste I²C arrachée au dessoudage | moyen-élevé | re-souder ; sinon EEPROM morte |
| EEPROM grillée (5 V CH341A) | élevé | remplacer par EEPROM vierge **reprogrammée avec le dump** (d'où dump **avant**) |
| Appairage effacé | moyen | ré-appairer (bouton power) ; perte des hôtes mémorisés |
| Brique SoC (glitch/mode test/flash) | **très élevé** | **aucun** garanti (ROM-based, pas de flash applicatif) |
| Garantie | — | A1314 hors garantie (notre exemplaire : PID 0x0256, génération 2011) ; ouverture = fin de toute prise en charge |

**[déduction]** **Le seul vrai filet de sécurité est le dump EEPROM intégral conservé avant toute
écriture.** La ROM interne n'a aucun retour arrière.

### 5.2 Difficulté et durée **[déduction]**

- **EEPROM, lecture seule** : **1–3 jours** (ouverture, identification, clip/dessoudage, dump, premières
  analyses). Compétence : soudure fine + lecture flashrom.
- **Désassemblage 8051 du dump EEPROM + patchs RAM** : **3–10 jours** (modélisation banque/SFR, ancrage sur
  nos report IDs).
- **ROM masquée 108 Ko** : **semaines, résultat incertain**, matériel de labo (glitch/decap) → **non
  recommandé**.

### 5.3 Matériel à acheter (chemin EEPROM) **[déduction]**

- Pince de test **SOIC-8** (~10 €), **CH341A** (3,3 V modifié) **ou Raspberry Pi/Bus Pirate** (~5–40 €),
  **alim labo 3,3 V**, fer à panne fine + flux + tresse, **loupe binoculaire/microscope USB**, médiators
  d'ouverture + isopropanol. **Total ~30–60 €.** `flashrom` / `i2cdump` / Ghidra = gratuits.
- Chemin ROM avancé (optionnel, déconseillé) : **ChipWhisperer** (~250 €+) pour le glitch.

### 5.4 Ce que le gérant doit faire physiquement vs ce qui reste faisable sans ouvrir

- **Nécessite le gérant (physique) :** ouvrir le clavier (isopropanol + médiators), identifier l'EEPROM à la
  loupe, clipser/dessouder, dumper l'EEPROM, éventuellement ressouder. **Lui seul décide** (son matériel,
  son risque).
- **Faisable sans ouvrir, sans toucher le clavier :**
  - préparer **l'environnement Ghidra 8051** (module + carte mémoire banque/SFR du §3.1/§4.1) **à vide** ;
  - documenter la **procédure de dump** et la liste de courses (§5.3) ;
  - **une fois un dump fourni**, tout le désassemblage/cartographie est hors-ligne, zéro contact clavier ;
  - poursuivre la RE **non invasive** déjà en cours (rapports HID en lecture seule, `RE-HID-EXHAUSTIF.md`),
    seul chemin qui n'exige **ni ouverture ni risque**.
- **Non faisable :** dumper la ROM/RAM **par radio** (pas de canal HCI exposé, §3.3). À écarter d'emblée.

---

## 6. Légalité et périmètre

- Analyse d'interopérabilité sur **le matériel du gérant** → **licite**. Aucune protection de tiers n'est
  contournée : la gén. 2009 n'a **pas de signature de firmware** (`RE-FIRMWARE-MAINTENANCE.md` §1.2, Chen
  BH USA 2009), et l'objectif est de **comprendre**, pas de redistribuer du code Apple. **[déduction]**
- **Ne pas** publier de binaire Apple extrait ; se limiter à la **documentation** des structures utiles au
  projet (matrice, batterie, nom, veille). **[déduction]**

---

## 7. Étapes → issues Gitea proposées (label `feature`, sans doublon)

Non couvertes par les issues existantes (#139/#140 décodage HID, #173/#176 veille, #175/#177 coupures,
#181 carte HANDSHAKE, #182 plan d'écriture SET_REPORT) : celles-ci portent sur la RE **HID logicielle**,
pas sur l'**extraction de firmware matérielle**. Étapes proposées :

1. **E1 — Préparer l'environnement de désassemblage 8051 (hors-ligne, sans clavier).** Module Ghidra 8051 +
   carte mémoire banque (`0x0000-0x7FFF` commun / `0x8000-0xFFFF` banké, P1 bits 3-2) + table SFR BCM2042
   initiale. Livrable : projet Ghidra vide + `docs/` procédure. Aucun contact clavier.
2. **E2 — Procédure et liste de courses pour dumper l'EEPROM I²C (lecture seule).** Identification visuelle,
   pince SOIC/dessoudage, CH341A 3,3 V ou RPi, `flashrom`/`i2cdump`, garde WP/alim/piles ôtées, **dump
   intégral conservé avant toute écriture**. Décision/exécution physique = gérant.
3. **E3 — Analyser le dump EEPROM** (une fois fourni) : localiser matrice, nom, paramètres, repérer
   d'éventuels patchs RAM et le stockage d'appairage ; ancrer sur les report IDs mesurés. Hors-ligne.
4. **E4 — Étude de faisabilité de l'extraction ROM interne** (UART/HCI PCB, glitch) : documentaire,
   **sans exécution**, décision go/no-go au gérant (recommandation par défaut : no-go).

---

## Sources

- geekhack « Disassembling an Apple Wireless » (topic 31981) ; « Wireless Model M » (topic 10371).
- iFixit *Apple Wireless Keyboard Teardown* (216345) ; guide A1314 45257.
- keyglove.net « Interfacing with a BCM2042/BP20422 Bluetooth HID Module » (2011-03-15, + commentaires FM24C64, offset matrice 0x148D, broche WP).
- Hackaday « Fix A Keyboard's Firmware With Trial, Error, And I2C » (2013-08-20) ; tag 8051.
- Broadcom *BCM2042 Product Brief* `2042-PB03-R` / datasheet (digchip, alldatasheet) : 8051, 108K ROM / 22K RAM / 20K Boot ROM, bank switching P1[3:2].
- InternalBlue (arXiv 1905.00631 ; seemoo-lab/internalblue) ; `BrcmPatchRAM/hci.h` ; naehrdine.blogspot « Unpatching the unpatchable » ; Infineon community 43438 (0xfc2e/0xfc4c).
- Ghidra `Processors/8051` ; issue NationalSecurityAgency/ghidra #7052 (bank switching) ; `cyrozap/ghidra-asmedia-8051`.
- jamchamb « Dumping K360 wireless keyboard firmware with a GreatFET » (2021) ; `JohnDMcMaster/8051dumper` (NF6X).
- Docs internes : `RE-FIRMWARE-MAINTENANCE.md`, `HARDWARE-RAPPORTS-HID.md`, `RE-HID-EXHAUSTIF.md`.
