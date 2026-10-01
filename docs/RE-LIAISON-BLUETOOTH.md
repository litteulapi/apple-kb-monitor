# Rétro-ingénierie de la liaison Bluetooth du clavier Apple A1314 (BR/EDR, HID)

Clavier : Apple A1314 ISO « Clavier de maria #1 » `04:DB:56:CA:42:EE` (puce Broadcom BCM2042 selon les
sources publiques). Hôte : PC01, Intel AX201 `hci0` `6C:94:66:52:7C:0D` (USB `8087:0026`), BlueZ 5.87,
noyau 7.1.13-2-MANJARO. Mesures du 2026-10-01, liaison établie à 11:33 (ré-appairage), **en lecture seule**.

Complète [RECONNEXION-PAIRAGE.md](RECONNEXION-PAIRAGE.md) (branche `fix/reconnexion-veille`, #142) :
ce document-ci cartographie la liaison elle-même ; celui-là traite la perte de liaison et le ré-appairage.

Étiquettes : **[mesuré]** observé ici, commande citée · **[source]** spécification, code ou documentation
publique cités · **[hypothèse]** plausible, non prouvé.

Outils : `tests/live/re/link_sdp_decode.py` (décodage hors ligne du cache SDP de BlueZ),
`tests/live/re/link_btmon_stats.py` (statistiques d'une capture btmon), `tests/live/re/link_snapshot.sh`
(relevé passif complet). Données : `tests/fixtures/a1314_iso/link_*`.

> Aucune commande de ce document n'écrit vers le clavier. La capture btmon utilisée est celle, déjà en
> cours, de l'agent « reconnexion » (`btmon -w …/reconnect-diag/btmon1.snoop`, démarrée à 11:37:14) : une
> copie figée a été décodée, aucun second btmon n'a été lancé.

## 0. Résumé

| Couche | Ce qu'on sait | Statut |
|---|---|---|
| SDP | 2 enregistrements : HID (`0x10000`) et PnP/DeviceID (`0x10001`). HID 1.0, pays 13 (ISO), sous-classe 0x40, `ReconnectInitiate`, `NormallyConnectable`, `RemoteWake`, `BatteryPower`, `BootDevice` vrais, `VirtualCable` faux, `SupervisionTimeout` 8000 slots = 5 s, **pas** d'attributs SSR (0x020F/0x0210). Descripteur 224 o identique octet pour octet à celui de hidraw | **complet** [mesuré] |
| DeviceID | source USB-IF (2), VID `05AC`, PID `0256`, version `0x0050` | complet [mesuré] |
| L2CAP | contrôle PSM 0x11 et interruption PSM 0x13 (SDP). CID côté hôte : 0x0041 contrôle, 0x0042 interruption ; CID côté clavier 0x0041 contrôle | [mesuré] (CID) / [source] (PSM) |
| Mode de lien | **sniff 12,5 ms (20 slots)** demandé **par le clavier** (aucune commande `Sniff Mode` de l'hôte) ; le clavier y revient 25-55 ms après chaque échange ; les frappes partent **en sniff**, calées sur la grille de 12,5 ms ; pas de sniff subrating | [mesuré] |
| Sortie de sniff | uniquement à l'initiative de l'hôte (`Exit Sniff Mode`), une fois par requête HID Control (noyau : `force_active`) | [mesuré] + [source] |
| Keepalive | aucun paquet L2CAP émis spontanément par le clavier au repos ; les seuls silences > 2 s durent **exactement 30,0 s** = sondage UPower | [mesuré] |
| Qualité de lien | RSSI BR/EDR 0 à −3 (valeur **relative** à la plage de réception idéale, pas des dBm) ; puissance d'émission de l'**hôte** 8 dBm (max 8) | [mesuré] |
| Réveil | après ≥ 0,5 s sans trafic, le clavier met **0,5 à 1,0 s** à quitter le sniff (au lieu de 8 ms) : il saute des ancres au repos | [mesuré] / mécanisme [hypothèse] |
| Supervision | délai effectif **≈ 20 s** (défaut Core 0x7D00) mesuré sur une vraie coupure à 12:13:28 ; les 5 s du SDP ne sont appliquées par personne | [mesuré] + [source] |
| Page (hôte) | page timeout 5,12 s ; page scan 11,25 ms toutes les 1,28 s (R1, standard, pas *fast connectable*) | [mesuré] |
| LMP distant | version, sous-version, fabricant, features pages 0-2, tailles de paquets, rôle : **inconnus** (échangés à la connexion, avant le début de la capture ; aucun outil de lecture passif). Référence publique d'un autre exemplaire : LMP 2.0, sous-version 0x31C, Apple (76), pas d'EDR/hold/park/SSR/SSP | inconnu, §4 |

⚠️ Le clavier s'est **déconnecté à 12:13:28** pendant l'étude (requête vendeur `0xFE` d'un autre agent restée
sans réponse, coupure par supervision 20 s, plus aucun *page scan* ensuite) : §3.7.

## 1. SDP (couche service)

Commande : `sudo -n cat /var/lib/bluetooth/6C:94:66:52:7C:0D/cache/04:DB:56:CA:42:EE` puis
`tests/live/re/link_sdp_decode.py tests/fixtures/a1314_iso/link_sdp_cache.txt` → `link_sdp_decoded.txt`.
`sdptool` n'est plus fourni par `bluez-utils` 5.87 : le cache de BlueZ (rempli lors du ré-appairage de
11:33) est la seule source passive ; une nouvelle requête SDP (`bluetoothctl` ne sait pas en forcer une sans
reconnexion) est inutile, le cache contient les enregistrements complets. **[mesuré]**

### 1.1 Enregistrement `0x00010000` — HID

| Attribut | Valeur | Lecture |
|---|---|---|
| 0x0001 ServiceClassIDList | `0x1124` HID | |
| 0x0004 ProtocolDescriptorList | L2CAP PSM **0x0011** / HIDP | canal de **contrôle** |
| 0x000D AdditionalProtocolDescriptorLists | L2CAP PSM **0x0013** / HIDP | canal d'**interruption** |
| 0x0005 BrowseGroupList | PublicBrowseRoot | |
| 0x0006 LanguageBaseAttributeIDList | `en`, UTF-8 (MIBenum 106), base 0x0100 | |
| 0x0009 BluetoothProfileDescriptorList | HID **v1.00** | profil HID 1.0 (pas 1.1/1.1.1) |
| 0x0100 / 0x0101 / 0x0102 | « Apple Wireless Keyboard » / « Keyboard » / « Apple Inc. » | |
| 0x0201 HIDParserVersion | 0x0111 | HID 1.11 |
| 0x0202 HIDDeviceSubclass | 0x40 | clavier, sans pointage |
| 0x0203 HIDCountryCode | 0x0D (13) | « International (ISO) » ✔ modèle ISO |
| 0x0204 HIDVirtualCable | **false** | pas de câble virtuel : l'hôte n'a pas de VIRTUAL_CABLE_UNPLUG à gérer |
| 0x0205 HIDReconnectInitiate | **true** | le clavier rappelle l'hôte après une perte de lien |
| 0x0206 HIDDescriptorList | type 0x22, **224 octets** | **identique octet pour octet** à `report_descriptor.bin` (hidraw) |
| 0x0207 HIDLANGIDBaseList | 0x0409 (en-US), base 0x0100 | |
| 0x0209 HIDBatteryPower | true | |
| 0x020A HIDRemoteWake | true | peut réveiller un hôte en veille (si l'hôte l'autorise : `WakeAllowed: yes`) |
| 0x020B HIDProfileVersion | 0x0100 | (attribut déprécié depuis HID 1.1) |
| 0x020C HIDSupervisionTimeout | 0x1F40 = 8000 slots = **5,000 s** | recommandation à l'hôte |
| 0x020D HIDNormallyConnectable | **true** | l'hôte peut l'appeler (page) quand il est éveillé |
| 0x020E HIDBootDevice | true | supporte le protocole Boot |
| 0x0200, 0x0208, 0x020F, 0x0210 | **absents** | pas de `SDPDisable`, **pas de paramètres SSR** (`HIDSSRHostMaxLatency`/`MinTimeout`, apparus en HID 1.1) |

Ce que l'hôte en fait **[source]** (BlueZ `profiles/input/device.c`, noyau `net/bluetooth`) :

* `ReconnectInitiate` + `NormallyConnectable` → `hid_reconnection_mode()` = `RECONNECT_ANY` ;
  D-Bus `org.bluez.Input1.ReconnectMode = "any"` **[mesuré]** (`busctl get-property … Input1 ReconnectMode`).
* `HIDSupervisionTimeout`, `HIDRemoteWake`, `HIDBatteryPower` : **lus par personne**. Ni BlueZ ni le noyau
  n'appliquent le délai de supervision de 5 s sur un lien BR/EDR (aucun `Write Link Supervision Timeout`
  dans `hci_conn.c`/`hci_event.c`/`device.c` ; le seul `supervision_timeout` du noyau est LE). Le délai
  effectif est donc celui que le maître du piconet a négocié, par défaut 0x7D00 = **20 s** (valeur par
  défaut de la spécification Core) **[source + hypothèse : rôle maître non observé, §7]**.
* `MGMT Read Default System Configuration` type 0x0006 (*Link Supervision Timeout*) = 0 → l'hôte ne
  configure rien **[mesuré]** (`sudo -n btmgmt --index 0 read-sysconfig`).

### 1.2 Enregistrement `0x00010001` — PnP Information (Device ID)

`SpecificationID 0x0100`, `VendorIDSource 2` (USB-IF), `VendorID 0x05AC`, `ProductID 0x0256`,
`Version 0x0050`, `PrimaryRecord true` **[mesuré]**. Le noyau en tire `Modalias usb:v05ACp0256d0050` et
`hid-apple` l'associe à `USB_DEVICE_ID_APPLE_ALU_WIRELESS_2011_ISO` (`APPLE_HAS_FN`,
`APPLE_NUMLOCK_EMULATION`, …) **[source]** `drivers/hid/hid-ids.h:169`, `hid-apple.c:1087`.
Recoupement : le rapport vendeur `0x4F` vaut `50 00` = **0x0050** = `Version` SDP **[mesuré]**
(`reports_scan_20261001.json`) → `0x4F` est très probablement la version du micrologiciel **[hypothèse]**.

## 2. L2CAP et HIDP (couche transport)

Capture : copie figée de `btmon1.snoop` (11:37:14 → 12:02:42, 1 528 s) décodée par
`btmon -r snap1.snoop -T --no-pager -C 200`, statistiques par `tests/live/re/link_btmon_stats.py`
(`tests/fixtures/a1314_iso/link_btmon_stats_20261001.json`). Les octets des frappes ne sont **jamais**
recopiés (seuls nombres et horodatages servent).

| Canal (CID de destination) | Paquets | Contenu | Étiquette |
|---|---|---|---|
| hôte → clavier, CID 0x0041 | 812 | `GET_REPORT` *Feature* (`0x43 <id>`) uniquement | [mesuré] |
| clavier → hôte, CID 0x0041 | 812 | `DATA Feature` (`0xA3 <id> …`) 353 · `HANDSHAKE` 459 | [mesuré] |
| clavier → hôte, CID 0x0042 | 1 121 | `DATA Input` id 0x01 (clavier, 10 o) 1 119 · id 0x11 (Fn/éjection) 2 | [mesuré] |
| hôte → clavier, interruption | 0 | aucun rapport de sortie (LED) pendant la capture | [mesuré] |

* Les PSM ne figurent pas dans la capture (canaux ouverts avant son début, btmon affiche `PSM 0`) : 0x11
  et 0x13 viennent du SDP ; l'attribution CID 0x41 = contrôle se déduit du contenu (GET_REPORT/HANDSHAKE),
  0x42 = interruption (rapports 0xA1) **[mesuré + déduction]**.
* **Seul** `GET_REPORT` est employé par l'hôte. Aucun `GET_PROTOCOL`, `SET_PROTOCOL`, `GET_IDLE`,
  `SET_IDLE`, `HID_CONTROL` (SUSPEND / EXIT_SUSPEND / VIRTUAL_CABLE_UNPLUG) n'a été vu **[mesuré]** ; BlueZ
  n'en émet pas en fonctionnement normal **[source]** `profiles/input/device.c` (uhid : seules les requêtes
  GET/SET_REPORT des applications sont relayées). Le protocole courant reste donc *Report* (celui de la
  connexion) **[source : HID 1.1, le mode Report est le mode par défaut après connexion]**.
* Réponses d'erreur **[mesuré]** : `ERR_INVALID_REPORT_ID` (0x02) ×448 pour les identifiants inconnus ;
  `ERR_UNSUPPORTED_REQUEST` (0x03) ×11 pour exactement `0x40 0x41 0x44 0x45 0x50 0x55 0xD0 0xD4 0xD5 0xFA
  0xFB`. Le micrologiciel **distingue** donc « identifiant inexistant » de « identifiant connu mais non
  lisible » : ces 11 identifiants existent vraisemblablement en écriture seule (*Output*/*Feature SET*)
  **[hypothèse]** — à ne pas sonder en écriture (§8).
* Délai d'une requête **[mesuré]** : réponse en 7,9 ms (médiane) lien actif ; 9,8 ms médiane mais **jusqu'à
  1,0 s** quand la requête part d'un lien en sniff inactif depuis > 0,5 s (§3.3). Le délai de BlueZ est de
  3 s (`REPORT_REQ_TIMEOUT`) **[source]** `profiles/input/device.c:647`.
* Rafales observées : balayages 0x00-0xFF d'autres agents de rétro-ingénierie (dmesg :
  `apple 0005:05AC:0256.0008: pid … passed too short report` à 12:04) — ils faussent les statistiques de
  repos ; la cadence UPower (2 GET_REPORT `0x47` toutes les 30 s) est documentée dans RECONNEXION-PAIRAGE §3.5.

## 3. Couche liaison (baseband/LMP vue depuis HCI)

### 3.1 Mode sniff : demandé par le clavier, 12,5 ms

| Fait | Valeur | Étiquette / commande |
|---|---|---|
| Intervalle sniff | **12,500 ms = 0x0014 = 20 slots**, identique sur les 260 entrées en sniff | [mesuré] `Mode Change … Interval: 12.500 msec (0x0014)` |
| Initiateur | **le clavier** : aucune commande HCI `Sniff Mode` de l'hôte dans toute la capture | [mesuré] `hci_commandes` = Exit Sniff ×260, Read RSSI ×37, Read TX Power ×37 seulement |
| Politique de l'hôte | `idle_timeout = 0` (l'hôte ne passe jamais de lui-même en sniff) ; `sniff_min/max_interval = 80/800` slots (50-500 ms) **inutilisés** | [mesuré] debugfs `hci0/idle_timeout`, `sniff_*_interval` ; MGMT sysconfig 0x0008/0x0009 = 0x0050/0x0320 |
| Sortie du sniff | **uniquement l'hôte**, une `Exit Sniff Mode` avant chaque requête de contrôle isolée | [mesuré] |
| Pourquoi l'hôte sort du sniff | sniff initié par le distant → le noyau efface `HCI_CONN_POWER_SAVE` ; tout envoi ACL avec `force_active` (défaut des sockets L2CAP de bluetoothd) déclenche `HCI_OP_EXIT_SNIFF_MODE` | [source] `hci_event.c:hci_mode_change_evt()`, `hci_conn.c:hci_conn_enter_active_mode()` |
| Retour en sniff | le clavier redemande le sniff **25-55 ms** après le dernier échange (médiane 40 ms en actif) | [mesuré] `sejours_actif` n=260, min 25 ms, médiane 40 ms, max 55 ms |
| Part du temps en actif | **0,65 %** de la capture (9,9 s sur 1 525 s), alors que 812 requêtes ont été servies | [mesuré] |
| Sniff subrating | aucun évènement `Sniff Subrating` (0x2E), pas d'attribut SSR dans le SDP | [mesuré] ; cohérent avec un contrôleur Bluetooth 2.0 [source BCM2042] |
| Changement d'intervalle | jamais : aucun `Mode Change` sniff → sniff ; toujours actif ↔ sniff 12,5 ms | [mesuré] |

### 3.2 Les frappes partent en sniff, calées sur la grille de 12,5 ms

* 1 081 des 1 121 rapports d'entrée arrivent pendant que le lien est en sniff, 2 en actif (38 avant le
  premier `Mode Change`) **[mesuré]** : **taper ne fait jamais sortir le lien du sniff**.
* Écarts entre rapports successifs (< 200 ms, n = 1 018) : écart moyen à la grille de 12,5 ms = **0,53 ms**
  (3,125 ms attendu pour des instants aléatoires) **[mesuré]**. Les rapports sont émis aux ancres de sniff.
* Conséquence : latence radio ajoutée à chaque frappe ∈ [0 ; 12,5 ms], moyenne ≈ 6 ms **[déduction]**.
  Le plus court écart observé est 11,0 ms (une ancre) **[mesuré]**.

### 3.3 Après ~0,5 s sans trafic, le clavier ne répond plus qu'au bout de 0,5 à 1 s

Délai `Exit Sniff Mode` → `Mode Change (Active)`, selon le temps passé en sniff sans trafic **[mesuré]** :

| Inactivité avant la requête | n | Délai médian | Min – max |
|---|---|---|---|
| < 0,5 s | 208 | **8 ms** | 6 – 246 ms |
| 0,5 – 25 s | 20 | **≈ 690 ms** | 540 – 1 016 ms |
| ≥ 25 s (sondage UPower) | 31 | **≈ 1 000 ms** | 16 – 1 016 ms (2 valeurs < 50 ms) |

* Un esclave en sniff 12,5 ms devrait accepter `LMP_unsniff_req` à l'ancre suivante (≤ 12,5 ms). Un délai de
  0,5-1 s sans aucun `Mode Change` intermédiaire indique que **le clavier saute des ancres** une fois
  inactif : il n'écoute plus qu'environ une fois par seconde, sans renégocier l'intervalle (le BCM2042,
  Bluetooth 2.0, n'a pas de sniff subrating) **[hypothèse forte, mécanisme interne non observable]**.
* Effet énergétique **[hypothèse chiffrée]** sur la base de la fiche du module BM2042 (BCM2042) : sniff
  10 ms = 2,35 mA, 1,28 s = 0,018 mA, sommeil 50 µA. Écouter toutes les 12,5 ms coûterait ≈ 1,9 mA
  (≈ 55 jours sur 2 piles AA), écouter ≈ 1 fois/s ≈ 0,02-0,05 mA : seul le second régime est compatible avec
  l'autonomie de plusieurs mois annoncée par Apple.
* Coût d'un sondage UPower (toutes les 30 s) **[hypothèse chiffrée]** : ≈ 1 s d'attente + 40 ms actif à
  ≈ 40 mA ≈ 1,6 mA·s par cycle ≈ **0,05 mA en moyenne**, soit du **même ordre que la consommation de repos**
  estimée ci-dessus. UPower doublerait ou triplerait la consommation du clavier au repos (#146).
* Conséquence logicielle : la première requête après une pause répond en ≈ 1 s. `akm-core`
  (`DecodeOptions::slow_failure = 1 s`, `decode.rs:108`) classe un **échec** qui a duré ≥ 1 s en panne de
  liaison ; seule la sonde `0xEA` (identifiant valide) part après une pause, donc pas de faux positif
  aujourd'hui, mais la marge est nulle (1,016 s mesuré) **[mesuré + source]**.

### 3.4 Pas de keepalive applicatif

* Aucun paquet ACL n'est émis spontanément par le clavier au repos : les plus longs silences ACL de la
  capture valent **30,0 s** exactement, tous terminés par un `GET_REPORT 0x47` d'UPower **[mesuré]**
  (`plus_longs_silences_acl_s`).
* Le maintien du lien en sniff repose sur les paquets POLL/NULL du baseband aux ancres, invisibles depuis
  HCI ; la supervision de lien (§1.1) coupe si ces échanges s'arrêtent plus de *Link Supervision Timeout*
  **[source : Core Spec, Vol 2 Part B]**.
* Délai de veille propre du clavier : **non observable** tant qu'UPower sonde toutes les 30 s. Le lien a tenu
  7 h sans frappe la nuit du 30/09 (RECONNEXION-PAIRAGE §3.5) : soit le clavier n'a pas de délai de
  déconnexion, soit le trafic de l'hôte le réarme **[hypothèse]**. Le rapport vendeur `0xF5 = 0x0384 = 900`
  (lu en BE comme `0xF4 = 1740`) est candidat à « 900 s = 15 min » (HARDWARE-RAPPORTS-HID §4) **[hypothèse]**.
  Sources publiques (forums Apple) : déconnexion vers 10 min d'inactivité sur Mac, sans référence officielle
  **[source faible]**. Expérience E2 (§8).

### 3.5 Qualité de lien

| Mesure | Valeur | Étiquette |
|---|---|---|
| RSSI (`HCI Read RSSI`, via `MGMT Get Connection Information`, rssi-helper toutes les 45 s) | 0 ×30, −1 ×3, −2 ×2, −3 ×2 | [mesuré] |
| Puissance d'émission **de l'hôte** sur ce lien | 8 dBm, max 8 dBm | [mesuré] `sudo -n btmgmt --index 0 conn-info -t 0 04:DB:56:CA:42:EE` |
| Puissance d'émission du clavier | inconnue (classe 2 typique, 0 à +4 dBm) | [source BM2042] |

* En BR/EDR, `Read RSSI` ne renvoie **pas** des dBm absolus : c'est l'écart (dB) à la *Golden Receive Power
  Range* du contrôleur ; **0 = dans la plage idéale**, négatif = sous la plage **[source]** Core Spec Vol 4
  Part E §7.5.4. Le projet l'expose pourtant comme `radio.rssi_dbm` et le widget affiche « 0 dBm » ;
  le critère F25 « RSSI < −80 dBm » (#105) est inatteignable sur ce lien. → issue créée (§9).
* `MGMT Get Connection Information` n'émet que des commandes HCI **locales** (Read RSSI, Read Transmit Power
  Level) : pas de sortie de sniff, pas de trafic radio **[mesuré]** (aucun `Exit Sniff Mode` associé) **[source]**
  (l'appel à `hci_conn_enter_active_mode()` n'a lieu que sur émission ACL).

### 3.6 Paramètres de l'hôte qui gouvernent la reconnexion **[mesuré]**

`sudo -n btmgmt --index 0 read-sysconfig` (`link_mgmt_sysconfig.txt`) et `btmgmt info` :

| Paramètre | Valeur | Commentaire |
|---|---|---|
| Page Scan Type (0x0000) | 0 = standard | `FastConnectable` (balayage entrelacé) **non actif** : `current settings` ne contient pas `fast-connectable`, `main.conf` n'a pas la clé à 12:05 |
| Page Scan Interval (0x0001) | 0x0800 = 2 048 slots = **1,28 s** | |
| Page Scan Window (0x0002) | 0x0012 = 18 slots = **11,25 ms** | l'hôte n'écoute que 0,9 % du temps quand il attend le clavier |
| Link Supervision Timeout (0x0006) / Page Timeout (0x0007) | 0 / 0 | valeurs par défaut du contrôleur |
| `connectable` (réglage MGMT) | **absent** | le noyau n'active le *page scan* que s'il existe des appareils de la liste d'acceptation **déconnectés** [source] `hci_update_scan` ; le clavier y figure (`device_list`) → page scan actif seulement pendant ses absences |

Un clavier qui appelle l'hôte doit donc tomber dans une fenêtre de 11,25 ms toutes les 1,28 s (mode R1) ;
un train de page standard couvre ≥ 1,28 s, donc ça marche **si** le contrôleur est éveillé — cf. l'autosuspend
USB de l'AX201 (RECONNEXION-PAIRAGE §3.4, #143).

### 3.7 Perte de liaison capturée le 01/10 à 12:13:28 (supervision mesurée : 20 s)

Pendant cette étude, le clavier a cessé de répondre ; la capture passive en cours l'a enregistré
(`link_btmon_stats_20261001_b.json`, seconde copie de `btmon1.snoop` décodée à 12:15:52) **[mesuré]** :

| Heure | Évènement |
|---|---|
| 12:13:00.6 → 12:13:08.4 | un outil de rétro-ingénierie d'un autre agent lit les rapports vendeurs un par un, toutes les 0,42 s (`0x4A … 0x5D 0x60 0xD1 0xD8 0xEB 0xF4 0xF5 0xF6 0xF7`) ; chaque requête reçoit sa réponse en 20-50 ms |
| 12:13:08.797 | `GET_REPORT 0xFE` + `Exit Sniff Mode` : **aucune réponse**, aucun `Mode Change` |
| 12:13:12 et 12:13:25 | bluetoothd : `HIDP GET_REPORT request timed out` (×2, 0xFE puis 0x47 d'UPower) |
| 12:13:28.822 | `Mode Change` statut **Connection Timeout (0x08)** puis `Disconnect Complete` raison 0x08 |
| 12:13:28.850 | l'hôte active le *page scan* (`Write Scan Enable` = Page Scan) |
| 12:13:30 → 12:15:29 | 7 `Create Connection` de l'hôte : **Page Timeout (0x04) en 5,12 s** chacun ; écarts 7, 9, 13, 30, 30, 30 s ; bluetoothd `Host is down (112)` ; clavier toujours absent à 12:16:33 |

Ce que cela mesure :

* **Délai de supervision effectif ≈ 20 s** (dernier échange réussi 12:13:08.4, coupure 12:13:28.8 ; à la
  précision des ancres près) : c'est la valeur par défaut 0x7D00 = 20,0 s, **pas** les 5 s demandées dans le
  SDP (`HIDSupervisionTimeout`) — confirmation de §1.1 **[mesuré]**.
* **Page timeout de l'hôte = 5,12 s** (0x2000 slots, défaut) ; `Create Connection` sans cache d'horloge :
  `Page scan repetition mode R2`, `Clock offset 0x0000`, paquets DM1-DH5 autorisés, *role switch* autorisé
  (le clavier peut devenir maître) **[mesuré]**.
* Le clavier ne fait **plus de page scan** après la coupure (aucune des 7 pages n'aboutit) : il est soit
  hors tension / redémarré dans un état sans *page scan*, soit bloqué **[mesuré + hypothèse]**.
* Le clavier a cessé de répondre **au milieu d'une session de lectures vendeur** (requête `0xFE`), alors que
  `0xFE` avait été lu sans incident plus tôt dans la même capture. Lien de cause à effet **non prouvé**
  (piles, coupure manuelle ou blocage du micrologiciel restent possibles) ; c'est cependant le troisième
  épisode « GET_REPORT sans réponse puis coupure » pendant une session de rétro-ingénierie (29/09 15:09-15:21,
  01/10 04:00, 01/10 12:13 ; cf. RECONNEXION-PAIRAGE §3.5) **[hypothèse à tester, E8]**.
* Aucune action de cette étude n'a précédé la coupure : seules des lectures locales (`btmgmt conn-info`,
  `read-sysconfig`, debugfs, fichiers BlueZ) ont été faites, aucune n'émet de trafic radio **[mesuré]**
  (journal `sudo` : lecture de `device_list` à 12:13:13, **après** la dernière requête sans réponse).

### 3.8 Contrôleur local (pour mémoire)

AX201 : HCI/LMP version 11 (= Bluetooth 5.2), fabricant 2 (Intel), révision 12490, classe 0x6C0104 ;
pages de features `0: bf fe 0f fe db ff 7b 87`, `1: 0b…`, `2: 20 0b…` **[mesuré]** debugfs. Clés de
chiffrement 7-16 octets acceptées.

## 4. LMP distant : ce qu'on ne peut pas lire passivement

Version LMP, sous-version, fabricant, pages de features 0-2, tailles max de paquets ACL, rôle (maître/esclave),
*clock offset* : échangés **à l'établissement** de la connexion (`Read Remote Supported Features`,
`Read Remote Version Information`, `Read Remote Extended Features`, `Role Change`, `Max Slots Change`,
`Link Supervision Timeout Changed`). La connexion actuelle date de 11:33, la capture de 11:37 : rien.
Le noyau ne les expose ni dans debugfs (`hci0/256/` est vide) ni dans sysfs (`hci0:256/`) **[mesuré]**.
`hcitool` (obsolète) n'est plus installé ; le relancer serait de toute façon une **injection HCI** (interdit).

Référence publique (autre exemplaire « Apple Wireless Keyboard », `hcitool info`, 2012) **[source]** :
`LMP Version: 2.0 (0x3) LMP Subversion: 0x31c`, `Manufacturer: Apple, Inc. (76)`,
`Features: 0xbc 0x02 0x04 0x38 0x08 0x00 0x00 0x00`. Décodage de cette page 0 (Core Spec Vol 2 Part C
§3.3) :

| Octet | Bits à 1 | Signification |
|---|---|---|
| 0 = 0xBC | 2,3,4,5,7 | chiffrement, slot offset, timing accuracy, **role switch**, **sniff** ; **pas** de paquets 3/5 slots, **pas de hold** |
| 1 = 0x02 | 1 | RSSI ; **pas de park** |
| 2 = 0x04 | 2 | power control |
| 3 = 0x38 | 3,4,5 | enhanced inquiry scan, interlaced inquiry scan, interlaced page scan ; **pas d'EDR 2/3 Mb/s** |
| 4 = 0x08 | 3 | AFH capable (esclave) |
| 5-7 = 0 | — | **pas de sniff subrating, pas de SSP, pas de features étendues** (pages 1-2 inexistantes) |

C'est cohérent avec tout ce qu'on mesure ici : sniff seul (pas de hold/park), pas de SSR, pairage legacy
(`LegacyPairing: yes`, pas de SSP), paquets 1 slot (rapports ≤ 14 o). Notre exemplaire (version 0x0050,
PID 0x0256 « 2011 ») peut différer : à confirmer par l'expérience E1.

## 5. Sources externes

* **BCM2042** : SoC HID Broadcom (8051 + pile Bluetooth complète en ROM, profil HID 1.0 intégré),
  Bluetooth 2.0 (+EDR selon le module), AFH, *fast connect*. Fiche Broadcom (alldatasheet / digchip) ;
  module Sunitec **BM2042** (BCM2042KFB) : courants sniff 10 ms 2,35 mA / 60 ms 0,39 mA / 100 ms 0,24 mA /
  1,28 s 0,018 mA, sommeil 50 µA, sommeil profond 16 µA, émission 0 dBm typ., +4 dBm max (classe 2).
  <https://pop.fsck.pl/hardware/toshiba-n554/SPEC-BM2042-V1.0.pdf>,
  <https://www.alldatasheet.com/datasheet-pdf/pdf/175090/BOARDCOM/BCM2042.html>.
* iFixit identifie le BCM2042 dans l'A1255 (3 piles) ; aucune vue de carte A1314 ne confirme la référence
  de puce **[source partielle]** <https://www.ifixit.com/Teardown/Apple+Wireless+Keyboard+Teardown/216345>.
* Référence LMP d'un Apple Wireless Keyboard : <https://elatov.github.io/2012/09/setup-apple-wireless-keyboard-via-bluetooth-on-fedora-17/>.
* Noyau Linux (master) : `net/bluetooth/hci_conn.c` (`hci_conn_enter_active_mode`), `hci_event.c`
  (`hci_mode_change_evt`), `hci_core.c` (`sniff_min/max_interval = 80/800`, `idle_timeout = 0`),
  `net/bluetooth/hidp/core.c` (chemin HIDP noyau, non utilisé ici : BlueZ 5.87 passe par **uhid**),
  `drivers/hid/hid-apple.c` / `hid-ids.h` (0x0256 = `ALU_WIRELESS_2011_ISO`).
* BlueZ (master) : `profiles/input/device.c` (`hid_reconnection_mode`, `REPORT_REQ_TIMEOUT 3`,
  `idle_timeout` = `IdleTimeout` d'`input.conf`, `BT_IO_SEC_MEDIUM`), `input.conf` local vide → `IdleTimeout=0`.
* Spécifications : Bluetooth HID Profile 1.0/1.1.1 (attributs SDP 0x0200-0x0210, transactions HIDP),
  Core Spec Vol 2 Part B/C (sniff, supervision, features LMP), Vol 4 Part E §7.5.4 (RSSI BR/EDR).

## 6. Hypothèses confrontées aux mesures

| Hypothèse (avant) | Verdict | Preuve |
|---|---|---|
| Le clavier se met en sniff long (≥ 100 ms) au repos | **Faux** au niveau HCI : sniff **12,5 ms** permanent | §3.1 |
| … mais il dort entre les ancres au repos | **Probable** : 0,5-1 s de délai de réveil après 0,5 s d'inactivité | §3.3 |
| L'hôte gère le sniff | **Faux** : c'est le clavier ; l'hôte ne fait que le quitter | §3.1 |
| Taper fait sortir du sniff | **Faux** | §3.2 |
| Le clavier envoie des keepalives | **Faux** (rien au-dessus du baseband) | §3.4 |
| `0x46`/`0x49` = paramètres de connexion | **Faux** : lien BR/EDR, vrais paramètres = sniff 20 slots, supervision SDP 8000 slots ; aucun ne figure dans les rapports vendeurs (pas de `14 00`, `40 1F`) | §3, #132 |
| Délai de veille du clavier = 15 min (`0xF5`) | **Non testé** (UPower empêche toute veille) | §3.4, E2 |
| Le clavier ré-initie la connexion | **Annoncé** (`ReconnectInitiate`) ; non capturé avec succès (RECONNEXION-PAIRAGE §4, maillon 3) | §1.1, E1 |
| Supervision 5 s appliquée | **Non** : l'hôte ignore l'attribut ; coupure réelle mesurée à **≈ 20 s** (défaut Core) | §1.1, §3.7 |
| Lire les rapports vendeurs est sans danger pour la liaison | **Douteux** : 3 coupures sur 3 sessions de lecture intensive, dont une capturée (requête `0xFE` sans réponse) | §3.7, E8 |
| RSSI en dBm | **Faux** en BR/EDR (écart à la plage idéale) | §3.5 |

## 7. Inconnues restantes

1. Version/sous-version/fabricant LMP et features pages 0-2 **de cet exemplaire**.
2. Rôle (qui est maître) et `Max Slots` (le *Link Supervision Timeout* effectif, ≈ 20 s, est mesuré : §3.7).
2b. Pourquoi le clavier s'est tu à 12:13:08 et ne fait plus de *page scan* : blocage du micrologiciel provoqué
    par les lectures vendeur, ou cause externe (piles, interrupteur) ?
3. Mécanisme exact du saut d'ancres (période, déclencheur) — et s'il dépend de l'état « touche enfoncée ».
4. Délai d'inactivité avant déconnexion / veille profonde du clavier, et ce qui le réarme (trafic hôte ?).
5. Comportement de reconnexion initiée par le clavier : durée du train de page, nombre de tentatives,
   *page scan* du clavier après abandon.
6. Signification des 11 identifiants `ERR_UNSUPPORTED_REQUEST` (écriture seule ?).
7. Réaction du clavier à `HID_CONTROL SUSPEND` / `EXIT_SUSPEND` (que BlueZ n'envoie jamais).

## 8. Expériences proposées, classées par risque

| # | Risque | Expérience | Ce qu'elle tranche | Commande / protocole |
|---|---|---|---|---|
| E0 | nul | Prolonger l'analyse sur la capture déjà en cours (nuit, aucune frappe) | stabilité 12,5 ms, distribution du délai de réveil | `tests/live/re/link_btmon_stats.py <copie de btmon1.snoop>` |
| E1 | faible (une reconnexion) | **Profiter** du test contrôlé de RECONNEXION-PAIRAGE §7 (déconnexion + touche) : la capture contiendra `Connection Request` (initiateur), `Read Remote Supported Features/Extended/Version`, `Role Change`, `Link Supervision Timeout Changed`, `Max Slots Change`, ouverture PSM 0x11/0x13 et premier `Sniff` | inconnues 1, 2, 5 | aucune commande en plus : `btmon -r … | grep -A12 -E 'Remote (Supported\|Extended) Features\|Remote Version\|Role Change\|Supervision\|Max Slots\|Connection Request'` |
| E2 | faible (réglage système réversible) | `NoPollBatteries=true` (UPower), démon en pause de lecture, **aucune frappe**, btmon passif 60 min | délai de veille réel (900 s ?), déconnexion initiée par le clavier ou non, ce que devient le sniff | `akm-conf.py upower` (branche fix/reconnexion-veille), puis E0 ; revenir en arrière ensuite |
| E3 | faible | Même chose avec le démon actif (16 req./15 min) | le trafic hôte réarme-t-il le délai ? | idem E2 |
| E4 | moyen (sockets bluetoothd) | Passer `BT_POWER` `force_active=0` sur les sockets HID de bluetoothd (patch BlueZ) | supprime les `Exit Sniff Mode` : requêtes servies en sniff, économie d'≈ 40 ms actif par requête | **modification de BlueZ**, hors périmètre ; à ne pas faire sans décision |
| E5 | moyen | `GET_PROTOCOL` / `GET_IDLE` | protocole courant, idle rate | impossible sans canal L2CAP brut : le PSM 0x11 est tenu par bluetoothd, uhid/hidraw ne transmettent que GET/SET_REPORT ; nécessiterait un hôte de test séparé |
| E6 | **élevé** | `HID_CONTROL SUSPEND` avant la veille du PC | le clavier passe-t-il en veille profonde / allonge-t-il le sniff | écriture : **interdit ici** ; hôte de test uniquement |
| E8 | faible à moyen (peut couper la liaison) | Rejouer **seule** la séquence de lecture vendeur (mêmes ids, même cadence 0,42 s) sous btmon, clavier fraîchement connecté, puis la séquence du démon (16 ids) ; 10 répétitions chacune ; noter l'id sans réponse | la lecture vendeur bloque-t-elle le micrologiciel, et sur quel id ? Si oui : liste noire d'ids dans `akm-core` | lectures GET_REPORT seulement ; à faire avec le gérant présent (ré-appairage possible) ; jamais pendant une mesure de stabilité |
| E7 | **élevé** | `SET_REPORT` sur `0xF5` (ou les 11 ids « unsupported ») | délai de veille réglable ? | écriture dans le micrologiciel, risque de brique/désappairage : **interdit**, seulement sur un exemplaire sacrifiable |

## 9. Suites dans Gitea

Voir les issues et commentaires listés en fin de session (section mise à jour au moment de leur création).
