# Rétro-ingénierie HID exhaustive — A1314 ISO (BCM2042)

Date : 2026-10-01. Clavier : A1314 ISO « Clavier de maria #1 », `04:DB:56:CA:42:EE`, `0005:05AC:0256`,
bcdDevice `0x0050`, `/dev/hidraw7`, BlueZ 5.87, noyau 7.1.13. Suite de `HARDWARE-RAPPORTS-HID.md`
(carte des 27 rapports Feature) et de `AUDIT-DECODAGE-HID.md`.

Niveaux de preuve : **[mesuré]** observé sur ce clavier · **[source]** code ou norme cités ·
**[hypothèse]** interprétation non prouvée.

> **État au moment de la rédaction** : le clavier a perdu la liaison à 12:13:28 et ne répondait plus
> à la reconnexion (`Host is down`). Sur consigne du coordinateur, plus aucune lecture n'a été émise
> depuis 12:11:28. La passe Output n'a pas été faite : la garde l'a refusée à 12:16:43 parce que le clavier
> était déconnecté. Ce document s'appuie uniquement sur les données déjà capturées.

## 0. Conclusions (réponse à la demande du coordinateur)

### 0.1 Lectures émises avant chaque coupure

| Coupure | Émetteur | Dernières requêtes (heure, rapport, taille du tampon) | Ce qui suit |
|---|---|---|---|
| **01/10 03:25:47** (`keyd` retire le périphérique à 03:25:45) | démon `apple-kb-monitord` à **03:24:51**, 14 rapports Feature (`0xEA, 0x47, 0xF5, 0x5A, 0x4F, 0xFF, 0x51-0x53, 0x46, 0x49, 0xF4, 0x4C, 0x09`), tampon 256 | 56 s sans requête connue avant la coupure, pas de btmon | `Host is down` à 03:25:58. Les piles avaient été changées vers 03:00 : cause non déterminée **[mesuré, incomplet]** |
| **01/10 04:00:13** | `sample_reports.py` (audit voisin), Feature, tampon 64, une requête toutes les 0,4 s | 04:00:09.0 `0x46` · 09.4 `0x47` · 09.8 `0x49` · 10.3 `0x4A` · 10.7 `0x4B` · 11.1 `0xEA` · 11.6 `0xF4` · 12.0 `0xF5` · 12.4 `0xF6` · 12.8 `0xF7` (réponses en 8-42 ms) · **≈ 04:00:13.2 `0xFE` → EIO après 3 393 ms** | `0xFF`, `0x5B` et `0x09` expirent ensuite (3,6 s chacune), `keyboard disconnected` à 04:00:33 **[mesuré]** (`tests/live/re/capture-2026-10-01.jsonl`) |
| **01/10 12:13:28** | outil RE d'un autre agent (pas cette sonde), Feature, une requête toutes les 0,42 s | … 12:13:06.65 `0xEB` · 07.07 `0xF4` · 07.49 `0xF5` · 07.92 `0xF6` · 08.36 `0xF7` (réponse en 39 ms) · **12:13:08.798 `0xFE`** | Le contrôleur PC reçoit l'accusé de bande de base de la requête `0xFE` 7 ms plus tard (`Number of Completed Packets`, 12:13:08.805), donc la requête a été transmise. Le clavier n'envoie ensuite plus rien : ni réponse, ni `Mode Change` après l'`Exit Sniff`. À 12:13:21.6, un GET `0x47` (lecture batterie noyau ou UPower) reste lui aussi sans réponse. À **12:13:28.822**, `Disconnect`, raison `Connection Timeout (0x08)`, **20,0 s** après la requête `0xFE` = supervision **[mesuré]** (btmon, `exhaustive_handshake_20261001.json`) |

Les deux passes de cette étude n'ont **provoqué aucune coupure** **[mesuré]** :

* passe Feature : 12:02:30.866 → 12:04:27.275, 2 816 ioctl, dont 2 560 transmises au clavier ;
* passe Input : 12:09:35.703 → 12:11:28.493, 2 816 ioctl.

Chaque passe couvre les IDs `0x00-0xFF` × 11 tailles, une requête toutes les 40 ms environ, `bluetoothctl` « Connected: yes » à la fin.
Dernière requête de cette sonde : **12:11:28.493** (Input `0xFF`, tampon 256, HANDSHAKE), soit 100 s avant la requête `0xFE` de 12:13:08.

### 0.2 Hypothèse « rapports non déclarés / tailles invalides bloquent le BCM2042 »

* **La partie « taille invalide » est réfutée.** BlueZ n'envoie au clavier qu'un en-tête et l'ID du rapport, sans le champ
  BufferSize : le bit Size n'est jamais posé **[source]**
  ([profiles/input/device.c](https://github.com/bluez/bluez/blob/master/profiles/input/device.c), `hidp_send_get_report`).
  Le noyau tronque ensuite la réponse à la taille du tampon (`min3(count, req->size, UHID_DATA_MAX)`) **[source]**
  ([uhid.c](https://github.com/torvalds/linux/blob/master/drivers/hid/uhid.c)). Une taille de 1 est refusée localement
  (`count < 2` → EINVAL, journal « passed too short report ») **[source]**
  ([hidraw.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hidraw.c)), et 512 lignes de ce type ont été mesurées
  dans le journal, sans aucun trafic radio. Le clavier reçoit donc **la même trame quelle que soit la taille du tampon** :
  la taille ne peut pas l'affecter **[mesuré + source]**.
* **La partie « rapports non déclarés » est très affaiblie dans sa forme générale.** 4 800 requêtes vers des IDs non
  implémentés ont reçu un HANDSHAKE d'erreur en 8-10 ms (médiane), en rafale à 40 ms d'intervalle, sans aucune coupure **[mesuré]**.
* **La forme restreinte tient : « un GET_REPORT Feature `0xFE` peut figer le micrologiciel ».**
  * **[mesuré]** Dans les deux coupures instrumentées, la dernière requête est `0xFE`, juste après `0xF7`, à 0,4 s d'intervalle.
    Si l'ID fatal était tiré au hasard dans ces séquences de 14 à 27 IDs, deux fois `0xFE` aurait une probabilité de l'ordre
    de (1/27)², soit 0,14 %.
  * **[mesuré]** `0xFE` est aussi le seul rapport vide dont le contenu a changé : le dump de 03:57 donne `fe 00 04…`
    (l'outil supprime les zéros de fin, mais pas un `04` intérieur), puis des zéros. Ce n'était donc **pas** un artefact,
    contrairement à ce que dit HARDWARE-RAPPORTS-HID §2.
  * **Contre-exemples [mesuré]** : `0xFE` a été lu sans incident 2 fois à 03:58, 13 fois dans la série de 04:15 à 05:15
    (à chaque fois juste après `0xF7`), 10 fois dans la passe Feature de 12:02 (après des refus de `0xFD`) et au moins une
    fois plus tôt dans la capture de 12:13. Cela fait **2 blocages sur environ 27 lectures** : le déclencheur n'est pas
    déterministe.
  * **[hypothèse]** `0xFE` lirait une boîte de réponse ou de journal interne dont le contenu dépend de l'état
    (`00 04` = valeur de `0xF6`/`0xF7`). La lire pendant une transition, par exemple l'entrée en sous-sniff ou en veille,
    bloquerait le cœur du micrologiciel : plus de LMP, plus de *page scan*, retour seulement à la remise sous tension.
* **Autre hypothèse non exclue [hypothèse]** : une perte de liaison spontanée, comme à 03:25:47 et à 02:55 avant tout
  accès de l'audit, qui coïnciderait avec `0xFE`.

**Tester sans risque**, du moins risqué au plus risqué ; aucune étape ne sera exécutée sans accord explicite du gérant :

1. **Hors ligne, sans clavier** : ajouter la séquence et l'ID de la dernière requête de chaque coupure future au fichier
   d'incidents (`exhaustive_handshake_20261001.json`, clé `incidents`). Fait pour 04:00 et 12:13.
2. **Contrôle négatif passif, 24 h** : clavier revenu, btmon actif, aucun outil RE. Seul le démon lit, avec la liste blanche de #177
   (`0x47`/`0x46`/`0x49`, ≥ 5 min, jamais `0xFE`). **Critère** : 0 coupure en 24 h, à comparer aux 3 coupures en ≈ 9 h de session
   RE. Une coupure qui survient quand même renvoie à une cause spontanée (piles, micrologiciel) et **disculpe** `0xFE`.
3. **Contrôle négatif actif** : la séquence de 04:00 et de 12:13 **sans `0xFE`** (`…0xF6, 0xF7, 0xFF`, 0,42 s),
   10 fois, à 5 min d'intervalle, sous btmon. **Critère** : 0 coupure.
4. **Seulement si 2 et 3 sont négatifs et que le gérant l'accepte** : **une** lecture `0xFE` isolée par jour, sous btmon,
   gérant à côté du clavier pour le remettre sous tension. D'abord après `0xF7`, puis après `0x47`, pour séparer
   « `0xFE` seul » de « `0xF7` → `0xFE` ». Arrêt définitif au premier blocage. Une lecture `0xFE` présente un risque
   réel : un blocage peut obliger à remettre le clavier sous tension (#175, E8).

### 0.3 Rapports à ne JAMAIS lire en production (commentaires sur #175 et #177)

| Rapport | Raison |
|---|---|
| **`0xFE`** (Feature) | dernière requête avant les 2 coupures instrumentées ; aucune valeur utile (vide ou `00 04`) |
| **`0x4C`** (Feature) | adresse de l'hôte + 12 octets secrets ; aucune fonction n'a besoin des 12 octets (#123, #133, #140 : adresse seule, lue au plus une fois par connexion) |
| **GET Input `0x01`** | renvoie les **touches enfoncées au moment de la requête** **[mesuré]** (1 touche vue dans 6 lectures sur 10) : fuite de frappe |
| Les 11 IDs « sans GET » (`0x40, 0x41, 0x44, 0x45, 0x50, 0x55, 0xD0, 0xD4, 0xD5, 0xFA, 0xFB`) | le micrologiciel les connaît mais refuse le GET : ce sont des commandes ou des registres en écriture seule, de sens inconnu |
| Les IDs refusés (218 en Feature, 251 en Input) | aucune donnée ; ce sont des requêtes radio inutiles |
| Copies et constantes (`0x60`, `0xEB`, `0x5B`, `0x5C`, `0x5D`, `0xD1`, `0xD8`, `0xF4`, `0xF5`, `0xF6`, `0xF7`, `0x4A`, `0x4B`, `0x09`) | constantes sur toute la journée ; aucune information en régime établi. Une lecture à la connexion au plus, si une fonction en a besoin |
| Toute lecture multi-taille et tout balayage | la taille n'atteint pas le clavier (§0.2) ; un balayage ne sert qu'à la rétro-ingénierie |

Jeu de production recommandé, aligné sur #177 : `0x47` (déjà lu par le noyau), `0x46`, `0x49`, au plus une fois toutes les 5 min,
par un seul lecteur. `0x5A`, `0x4F` et `0x51-0x54` au plus une fois par connexion. La sonde `0xEA` du démon est à remplacer par `0x47`
(#136, #177).

## 1. Méthode

* `tests/live/re/exhaustive_probe.py` : une passe = un type de rapport, IDs `0x00-0xFF` × tampons
  `1, 2, 3, 4, 8, 9, 16, 20, 32, 64, 256`, ioctl `HIDIOCGFEATURE` (nr 7), `HIDIOCGINPUT` (nr 0x0A) ou `HIDIOCGOUTPUT`
  (nr 0x0C). Le nœud est ouvert en `O_RDONLY` : les GET sont permis en lecture seule (`hidraw_ro_variable_size_ioctl`) **[source]**.
* Garde-fous appliqués par le code :
  * `bluetoothctl info` doit indiquer « Connected: yes » ;
  * `HID_UNIQ` du nœud = adresse du clavier ;
  * verrou de 300 s entre deux passes, 30 ms entre requêtes ;
  * arrêt sur un errno autre que EIO/EINVAL, ou sur une erreur de plus de 1 s : une expiration BlueZ prend 3 s
    (`REPORT_REQ_TIMEOUT 3`, BlueZ), une expiration uhid 5 s **[source]**.
* Masquage :
  * `0x4C` : longueur, 2 premiers octets, empreinte SHA-256 tronquée **du rapport complet (20 octets) seulement**.
    L'empreinte d'un préfixe court (9 octets = 8 connus + 1 secret) serait réversible ;
  * Input `0x01` : seulement le nombre de touches.
* La sonde ne distingue pas les codes HANDSHAKE (uhid rend `-EIO` pour toute erreur) **[source]**. Ils ont été lus dans un btmon
  concomitant (session reconnect-diag), en appariant chaque requête du canal de contrôle à la réponse suivante.
* Fichiers : `tests/fixtures/a1314_iso/exhaustive_feature_20261001.json`, `exhaustive_input_20261001.json`,
  `exhaustive_handshake_20261001.json` (carte HANDSHAKE + 2 incidents).

## 2. Résultats exhaustifs

### 2.1 Feature (GET_REPORT type 3) — suivi #181

| Réponse du clavier | IDs | Nombre |
|---|---|---|
| DATA (`0xA3`) | `09 46 47 49 4A 4B 4C 4F 51 52 53 54 5A 5B 5C 5D 60 D1 D8 EA EB F4 F5 F6 F7 FE FF` | 27 (identiques à la carte précédente) |
| HANDSHAKE **ERR_UNSUPPORTED_REQUEST** (`0x03`) | **`40 41 44 45 50 55 D0 D4 D5 FA FB`** | **11, nouveau** |
| HANDSHAKE ERR_INVALID_REPORT_ID (`0x02`) | tous les autres, y compris `0x00` | 218 |

* **[mesuré]** Un ID inconnu reçoit `0x02`. Les 11 IDs ci-dessus reçoivent `0x03` : le micrologiciel a une entrée pour eux mais
  refuse le GET **[source : sens des codes, Bluetooth HID Profile 1.1.1 §7.4.1]**. Ils sont regroupés autour des familles connues :
  `0x40-0x45` près de `0x46-0x4C`, `0x50`/`0x55` autour du nom `0x51-0x54`, `0xD0`/`0xD4`/`0xD5` près de `0xD1`/`0xD8`,
  `0xFA`/`0xFB` près de `0xF4-0xFF`. **[hypothèse]** Ce sont des registres en écriture seule ou des commandes ; `0x50` et `0x55`
  pourraient servir à écrire et valider le nom.
* **[mesuré]** La taille de la réponse ne dépend pas du tampon : le clavier rend toujours sa longueur naturelle
  (2, 3, 4, 9 ou 20 octets), le noyau tronque, et les préfixes sont cohérents. **Aucun rapport ne répond « seulement avec la bonne
  taille »**, ce qui s'explique par le fait que la taille n'est jamais transmise (§0.2).
* Latence : DATA en 8,9 ms (médiane), refus en 8,7 ms. La **première requête après un repos** prend 680 à 985 ms, le temps de
  sortir d'un sniff profond ou d'un sous-sniff **[mesuré ; cause : hypothèse]**.
* Valeurs à 12:02-12:04 **[mesuré]**, piles neuves depuis ≈ 9 h :

  | Rapport | Valeur | Rappel du matin (04:15-05:15) |
  |---|---|---|
  | `0x46` | 2974-2982 mV | 2986-2991 mV |
  | `0xFF` | 2969-2978 mV | idem `0x46` |
  | `0x49` | **2945 mV**, stable sur 11 lectures | 2950 mV |
  | `0x47` | 98 | 99 |
  | `0xEA` | 98 | 98 |
  | BlueZ | 98 % | 99 % |

  * La tension baisse : la décharge est mesurable en quelques heures.
  * `0x47` vaut désormais `0xEA` : l'écart d'un point du matin s'est résorbé.
  * La relation « `0x47` = troncature de l'interpolation de `0x49` sur `0x5A` » est **mise en défaut** : elle donne 99,5 → 99, le clavier rend 98 (#179).
* Constants **[mesuré]** : `0x09`, `0x4A`, `0x4B`, `0x4C` (même empreinte qu'à 03:57 : `f34626564fca9676`), `0x4F`, `0x51-0x54`,
  `0x5A`/`0x60`/`0xEB`, `0x5B`, `0x5C`/`0x5D`, `0xD1`/`0xD8`, `0xF4`-`0xF7`, `0xFE` (zéros).

### 2.2 Input (GET_REPORT type 1)

| ID | Réponse | Déclaré dans le descripteur ? | Lecture |
|---|---|---|---|
| `0x01` | 9 octets : état clavier de démarrage **en direct** | oui (Input/Output) | touches enfoncées pendant la requête **[mesuré]**, masquées dans les fixtures |
| `0x04` | `04 00` | **non** | inconnu |
| `0x05` | `05 02` | **non** | inconnu ; **[hypothèse]** état ou mode (2 = ?) |
| `0x11` | `11 00` | oui (Éjection + Fn vendeur) | 0 = aucune touche **[mesuré]** |
| `0x30` | `30 00` | **non** | inconnu |
| `0x12`, `0x13`, **`0x47`** | HANDSHAKE `0x02` | **oui** (Input) | refusés |

* **[mesuré]** `0x47` est déclaré Input, mais le clavier **refuse le GET Input `0x47`** et ne le sert qu'en Feature.
  C'est exactement ce que corrige le quirk `HID_BATTERY_QUIRK_FEATURE` de `hid-input.c` pour le 0x0256 **[source]**.
  Le noyau lit `0x47` avec un tampon de `max(len, 4)` octets **[source]**
  ([hid-input.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-input.c), `hidinput_query_battery_capacity`).
* **[mesuré]** Tous les refus Input sont `0x02` : il n'y a pas d'équivalent Input aux 11 IDs `0x03` du Feature.
* `0x04`, `0x05` et `0x30` sont des rapports Input non déclarés qui répondent au GET. **[hypothèse]** Ce sont des tampons
  d'entrée internes que le micrologiciel n'émet pas en interruption. Le démon pourrait les surveiller **passivement** (lecture
  de `/dev/hidraw`, rapport d'interruption) sans aucun GET. Seuls des horodatages et des IDs ont été mesurés à ce jour.

### 2.3 Output (GET_REPORT type 2)

**Non mesuré** : la passe a été refusée par la garde, parce que le clavier était déconnecté depuis 12:13:28. Les GET Output
passent par `HIDIOCGOUTPUT` → `UHID_OUTPUT_REPORT` → `HIDP_DATA_RTYPE_OUTPUT` **[source]** et sont en lecture seule.
Prévision **[hypothèse]** : `0x01` rendrait l'état des 5 LED (1 octet) et tout le reste `0x02`. La passe ne présente pas
de risque particulier, mais elle n'est pas prioritaire : à reprendre seulement après l'élucidation de #175.

### 2.4 Comportement de la liaison pendant les passes

* **[mesuré]** Le clavier reste en **sniff à 12,5 ms** au repos. Chaque requête déclenche `Exit Sniff` côté PC, puis le clavier
  revient en sniff 30 à 50 ms après la réponse. On compte 1 372 et 1 380 cycles Active/Sniff par passe, soit environ un cycle
  pour deux requêtes, sans aucune erreur.
* **[mesuré]** 5 632 requêtes en ≈ 4 min ont été encaissées sans incident. La cadence n'est donc pas, en soi, la cause des coupures.
* **[mesuré]** Effet de bord : 512 lignes « passed too short report » dans le journal noyau, dues au tampon de 1 octet.
  Ce test est sans valeur, puisqu'il n'y a pas de trafic : il est à retirer des futures passes.
* **[mesuré]** Pendant la passe Feature, le démon a fait une acquisition (12:03:03) sans conflit visible, alors que BlueZ
  n'accepte qu'une requête en cours **[source]**.
* Veille et reconnexion : aucune observation propre ; la série passive (`exhaustive_timeseries.py`) n'a pas été lancée,
  à cause de l'arrêt des lectures. Pour la veille, voir #173 et #176 ; pour la reconnexion, #142 et #175.

## 3. Croisement avec les sources

| Source | Apport | Statut |
|---|---|---|
| BlueZ `profiles/input/device.c` | GET_REPORT = en-tête + ID, **sans BufferSize** ; correspondance des types de rapport uhid → HIDP ; `REPORT_REQ_TIMEOUT 3` s | [source] confirmé par la mesure : réponse indépendante du tampon, EIO après ≈ 3,4 s |
| Noyau `hidraw.c` | `count < 2` → EINVAL ; `HIDIOCGFEATURE`/`HIDIOCGINPUT`/`HIDIOCGOUTPUT` | [source] confirmé |
| Noyau `uhid.c` | attente de 5 s, `req->err` → `-EIO`, copie `min3(...)` | [source] confirmé |
| Noyau `hid-input.c` | quirk `PERCENT \| FEATURE` pour 0x0255/0x0256 ; tampon `max(len, 4)` ; lecture limitée à une toutes les 30 s | [source], expliqué par le refus du GET Input `0x47` [mesuré] |
| Noyau `hid-apple.c` | 0x0256 : `APPLE_NUMLOCK_EMULATION \| APPLE_HAS_FN \| APPLE_ISO_TILDE_QUIRK`, aucune correction de descripteur, aucun rapport vendeur (`0xB0`/`0xBF` = rétroéclairage, d'autres modèles) | [source] |
| Bluetooth HID Profile 1.1.1 §7.4 | codes HANDSHAKE, bit Size de GET_REPORT | [source] |
| Fiche du module BM2042 (tiers, à base de BCM2042) ([PDF](https://pop.fsck.pl/hardware/toshiba-n554/SPEC-BM2042-V1.0.pdf)) | VBAT de **1,7 à 3,6 V**, sniff de 10 ms à 1,28 s, veille profonde réveillée par interruption | [source] ; **appuie l'hypothèse `0xF4` = 1740 mV = tension minimale de fonctionnement** |
| macOS IOKit `AppleBluetoothHIDKeyboard` ([managingosx, 2014](https://managingosx.wordpress.com/2014/04/23/reporting-on-bluetooth-mousekeyboard-battery-status/)) | propriétés `BatteryPercent`, `BatteryLow`, `BatteryPanic` et un bloc binaire `"Battery" = <"MVLT…` | [source] ; **[hypothèse]** `MVLT` serait une étiquette « millivolts » : macOS lirait une tension en mV, comme `0x46` |
| Projets publics | le seul résultat qui documente ces IDs est **ce projet lui-même** (miroir GitHub `litteulapi/apple-kb-monitor`, source circulaire, non retenue). Aucune autre carte des registres BCM2042 n'a été trouvée | — |

## 4. Classement octet par octet

Ce tableau ne liste que les ajouts et corrections ; pour le reste, voir `HARDWARE-RAPPORTS-HID.md` §2.

| Rapport | Octet(s) | Sens | Preuve |
|---|---|---|---|
| Feature `0x40, 0x41, 0x44, 0x45, 0x50, 0x55, 0xD0, 0xD4, 0xD5, 0xFA, 0xFB` | — | ID connu du micrologiciel, GET non pris en charge | [mesuré] ERR_UNSUPPORTED_REQUEST ; sens [hypothèse] |
| Feature `0xFE` | 1-8 | normalement nuls ; `00 04` une fois (03:57) | [mesuré] valeur vivante ; sens [hypothèse] boîte de réponse ; **dangereux** |
| Feature `0xF4` | 1-2 (BE) | 1740 = tension minimale en mV (la fiche BM2042 donne VBAT ≥ 1,7 V) | valeur [mesuré], borne [source], lien [hypothèse renforcée] |
| Feature `0x47` | 1 | % ; **ne suit pas** tronc(interp(`0x49`)) | [mesuré] (#179) |
| Feature `0x49` | 1-2 (LE) | mV lissés : 2953 → 2950 → 2945 sur 8 h | [mesuré] |
| Feature `0x46` / `0xFF` | 1-2 | mV instantanés, en baisse de 2991 à 2982 entre le matin et midi | [mesuré] |
| Feature `0x4C` | 0-19 | inchangé : `0x03`, l'hôte, 12 octets secrets, même empreinte toute la journée | [mesuré] |
| Input `0x01` | 1-8 | état clavier de démarrage en direct | [mesuré] + descripteur |
| Input `0x04` | 1 | `0x00` | [mesuré], sens inconnu |
| Input `0x05` | 1 | `0x02` | [mesuré], sens inconnu |
| Input `0x11` | 1 | Éjection/Fn, 0 au repos | [mesuré] + descripteur |
| Input `0x30` | 1 | `0x00` | [mesuré], sens inconnu |

## 5. Inconnues restantes

1. La cause des coupures. Est-ce `0xFE`, et dans quel état ? Les tests 2 à 4 du §0.2 doivent trancher, en lien avec #175 et #177.
2. Le sens de `0x09`, `0x4A` (18), `0x4B` (`00 08`), `0xD1`/`0xD8`, `0xF6`/`0xF7` (4), `0xF5` (900 ; voir #173 pour l'hypothèse
   de la veille), des 12 octets de `0x4C`, des Input `0x04`/`0x05`/`0x30`, et le rôle des 11 IDs refusés au GET.
3. La fonction % = f(mV) (#179).
4. Output : non mesuré.
5. La reconnexion : aucune lecture de comparaison avant/après n'a pu être faite.

## 6. Plan d'expériences d'écriture, par paliers (NON EXÉCUTÉ, suivi #182)

**Préalables à tout palier :**

* #175 résolu, et le clavier stable pendant 24 h sans outil RE ;
* gérant présent, clavier à portée pour le remettre sous tension ;
* btmon actif pendant tout le palier ;
* une seule écriture par session ;
* avant l'écriture : relire le registre ciblé et ses copies, et noter l'empreinte de `0x4C` ;
* après l'écriture : la même relecture, plus `bluetoothctl info`.

**Critère d'arrêt commun** :

* tout HANDSHAKE autre que SUCCESSFUL ;
* une relecture différente de la valeur attendue ;
* une empreinte de `0x4C` modifiée ;
* une coupure ou une expiration.

En cas d'arrêt : retour arrière immédiat, puis fin définitive du palier.
`0x4C` n'est **jamais** écrit, quel que soit le palier : risque de perte de l'appairage.

| Palier | Expérience | Gain attendu | Risque | Retour arrière |
|---|---|---|---|---|
| **W0** (sans écriture) | GET Output (§2.3) ; série passive (`exhaustive_timeseries.py`) ; capture des rapports d'interruption `0x04`/`0x05`/`0x30` | carte Output ; sens des Input non déclarés ; veille | lectures seulement, sans `0xFE` | — |
| **W1** identité, rapport déclaré | SET_FEATURE `0x09` = `09 01 00 00` (valeur lue) | valider que le seul Feature déclaré est accepté en écriture et voir le HANDSHAKE ; préalable au sens de `FF01:0x0B` | faible : valeur identique, rapport prévu par le descripteur | aucun nécessaire |
| **W2** identité, nom | SET_FEATURE `0x51`, `0x52`, `0x53`, `0x54` = valeurs lues, un rapport par session | savoir si le nom s'écrit par ces rapports (#141, renommer le clavier) ; `0x50`/`0x55` restent à identifier | faible à moyen : écriture en mémoire non volatile possible, effet sur le nom annoncé (SDP) | réécrire la valeur lue |
| **W3** identité, configuration | SET_FEATURE `0xF5` = `f5 03 84`, puis `0xF4`, `0xF6`, `0xF7` | savoir si les constantes sont modifiables (préalable à W5) | moyen : paramètres d'alimentation ou de radio ; écriture en NVRAM possible | réécrire la valeur lue |
| **W4** identité, étalonnage | SET_FEATURE `0x5A` = valeur lue ; vérifier ensuite `0x60` et `0xEB`. Ne jamais écrire les copies directement | savoir si les 3 copies se resynchronisent (journal ou miroir) | moyen : un % faux si la table est corrompue | réécrire la valeur lue dans les 3 IDs |
| **W5** valeur modifiée, réversible | renommer : `0x51-0x54` = nouveau nom ASCII ≤ 32 o, padding NUL. Puis `0xF5` 900 → 1200, en mesurant le délai de veille de façon passive (#173) | fonction « renommer » sans ré-appairage ; réglage du délai de veille | moyen : nom illisible ou veille trop longue (autonomie) | réécrire l'ancien nom ou 900 |
| **W6** valeur modifiée, état | `0x09` : 01 → 00, avec observation des LED, du % et de la veille | sens de `FF01:0x0B` | moyen à élevé : mode inconnu | réécrire 01 ; à défaut, remise sous tension |
| **W7** — **déconseillé** | toute écriture vers les 11 IDs « sans GET » (`0x40` … `0xFB`) | les identifier | **élevé** : commandes inconnues, possiblement réinitialisation, désappairage ou mise à jour du micrologiciel | aucun garanti |
| **Interdit** | `0x4C` (appairage) ; `0xFE` ; `0x46`, `0x47`, `0x49`, `0xFF`, `0xEA` (mesures) ; Output `0x01` (LED, hors du champ de la RE) | — | perte de l'appairage, blocage, aucun gain | — |

## 7. Reproduire (après accord, clavier stable)

```bash
python3 tests/live/re/exhaustive_probe.py --type feature --out f.jsonl   # garde : connecté, 300 s, arrêt sur erreur lente
python3 tests/live/re/exhaustive_probe.py --summarize f.jsonl > f.json
python3 tests/live/re/exhaustive_timeseries.py --out ts.jsonl --period 300 --duration 3600   # 26 Feature, sans 0xFE
```

Les deux outils excluent désormais `0xFE` (`NEVER_READ`), et la taille 1 a été retirée de `LENGTHS` (§0.2-0.3).
Les fixtures de ce document ont été produites avec l'ancienne version, qui lisait `0xFE` et testait la taille 1.
