# Reconnexion, pairage et veille du clavier Bluetooth

Suivi : [#142](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/142) ·
bugs [#143](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/143) (autosuspend AX201),
[#144](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/144) (reprise de liaison),
[#145](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/145) (veille système),
[#146](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/146) (sondage UPower) ·
outils [#147](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/147) (`akmctl doctor` / `akmctl repair`).

Matériel : clavier Apple A1314 ISO « Clavier de maria #1 » `04:DB:56:CA:42:EE` (BR/EDR, pairage *legacy*
par code PIN, sans SSP), adaptateur Intel AX201 USB `8087:0026` (firmware `ibt-1040-4150`), BlueZ 5.87,
noyau 7.1.13-2-MANJARO, PC fixe PC01.

Étiquettes : **[mesuré]** observé sur cette machine (commande ou capture citée) · **[source]** lu dans le
code de BlueZ 5.87 ou du noyau · **[hypothèse]** plausible, pas encore prouvé ici.

## 1. Symptôme

Le clavier perd la liaison ; « au bout d'un moment » `bluetoothctl connect` échoue
(`org.bluez.Error.Failed br-connection-create-socket`) et seule la séquence *oublier + ré-appairer* le
ramène. Dernier épisode : redémarrage à froid du PC le 2026-10-01 à 11:02, clavier absent de 11:06 à 11:32,
« Oublier » dans le gestionnaire Bluetooth de Plasma à 11:32:35, assistant d'appairage à 11:32:52, clavier
revenu à 11:33:00 **[mesuré]** (`journalctl -b -1`, `-b 0`).

## 2. Ce que disent réellement les erreurs

| Message | Signification | Étiquette |
|---|---|---|
| `control_connect_cb() connect to …: Host is down (112)` | `EHOSTDOWN` sur le canal de contrôle HID (PSM 17) : **page timeout radio**, le clavier n'a pas répondu à l'appel de l'hôte. Ce n'est pas un refus de clé. | [source] `profiles/input/device.c`, `src/error.c` |
| `br-connection-create-socket` | Traduction BlueZ de `EIO`, code renvoyé par le profil HID quand `control_connect_cb` échoue. C'est **le même** page timeout, vu côté D-Bus. | [source] `btd_error_bredr_str()` |
| Clé refusée (ce qu'on n'a **jamais** observé) | apparaîtrait comme `br-connection-key-missing` (`EBADE`), `org.bluez.Error.AuthenticationFailed/Rejected`, ou le signal `Device1.Disconnected("org.bluez.Reason.Authentication")` | [source] `src/error.h`, `src/device.c:device_disconnected()` |
| `HIDP GET_REPORT request timed out` | Une lecture de *Feature Report* (ioctl `HIDIOCGFEATURE` → uhid → bluetoothd) est restée sans réponse ; bluetoothd renvoie `EIO` à l'appelant, **sans** couper la liaison. | [source] `hidp_report_req_timeout()` |

Conséquence : l'échec de `bluetoothctl connect` ne prouve pas que le pairage est cassé ; il prouve que le
clavier **n'écoutait pas** (il dort, ou il appelle lui-même un hôte qui ne lui répond pas).

## 3. Mesures (2026-10-01, après le ré-appairage de 11:33)

### 3.1 Pairage et clé de lien

* `Paired/Bonded/Trusted = yes`, `LegacyPairing = yes`, `WakeAllowed = yes` **[mesuré]** (`bluetoothctl info`).
* Clé noyau (`/sys/kernel/debug/bluetooth/hci0/link_keys`) et clé stockée
  (`/var/lib/bluetooth/6C:94:66:52:7C:0D/04:DB:56:CA:42:EE/info`) : **empreintes SHA-256 identiques**
  (`b4e4395e8b4b…`), type 0 (*combination key* legacy) **[mesuré]**. Aucune désynchronisation au moment de
  la mesure.
* Écart : `pin_len` = 16 côté noyau, `PINLength=0` dans le fichier **[mesuré]**. Après un redémarrage le
  noyau rechargera `pin_len = 0`. Sans effet pour un clavier : BlueZ exige le niveau de sécurité MEDIUM
  (`BT_IO_SEC_MEDIUM`), pour lequel une *combination key* suffit quel que soit `pin_len` ; seul le niveau
  HIGH rejette une clé de PIN < 16 **[source]** `hci_conn_security()`, `hci_link_key_request_evt()`.
* Le clavier est dans la liste d'acceptation du noyau (`device_list`), donc l'hôte fait du *page scan*
  quand il est déconnecté **[mesuré]** ; BlueZ l'y remet à chaque démarrage (`load_devices()` →
  `btd_device_set_temporary(false)` → `adapter_accept_list_add()`) **[source]**.
* Le 27/09 à 12:59 et 13:03, bluetoothd n'a pas pu réécrire le fichier `info` du clavier
  (`g_rename() failed: No space left on device`) **[mesuré]** (`journalctl -b -2`). L'écriture est atomique
  (fichier temporaire + rename) : l'ancien fichier reste intact. Mais pendant une saturation disque, une
  **nouvelle** clé (ré-appairage, changement de clé) vivrait seulement en mémoire et serait perdue au
  redémarrage suivant → clé refusée par le clavier → ré-appairage obligatoire **[hypothèse, mécanisme
  [source] `store_link_key()`]**. `akmctl doctor` signale désormais ces lignes.

### 3.2 Enregistrement SDP du clavier

`HIDReconnectInitiate = true`, `HIDNormallyConnectable = true`, `HIDRemoteWake = true`,
`HIDSupervisionTimeout = 0x1F40` (5 s), `HIDVirtualCable = false`, `HIDBatteryPower = true` **[mesuré]**
(cache SDP de BlueZ décodé). BlueZ en déduit le mode `any` : le clavier se reconnecte lui-même (touche) et
l'hôte peut aussi l'appeler quand il est éveillé **[source]** `hid_reconnection_mode()`.

### 3.3 Politique de reconnexion côté hôte

* Profil HID de BlueZ : après une perte de lien, **6 tentatives espacées de 30 s, puis abandon définitif**
  (« at most 3 minutes ») **[source]** `input_device_auto_reconnect()` ; observé : `Host is down` à 04:01:08,
  :38, 04:02:08, :38, 04:03:08, :38 puis plus rien **[mesuré]**.
* Plugin *policy* (`[Policy] ReconnectAttempts/Intervals`) : ne joue qu'après une déconnexion pour
  *timeout*. Jusqu'au 01/10 11:16, ces clés étaient sous `[AdvMon]` et ignorées
  (`Unknown key ReconnectUUIDs for group AdvMon`) **[mesuré]** ; corrigées à 11:16 (sauvegarde
  `main.conf.bak-20261001111627`).
* Personne ne relançait ensuite : le démon n'avait aucune logique de reprise (#144).

### 3.4 Adaptateur AX201 et autosuspend USB

* `btusb enable_autosuspend = Y`, `power/control = auto`, `autosuspend_delay_ms = 2000` **[mesuré]**. C'est
  systemd qui l'impose pour cet identifiant précis : `60-autosuspend-chromiumos.hwdb` contient
  `usb:v8087p0026*` et `60-autosuspend.rules` écrit `power/control=auto` **[mesuré]**.
* Tant que le clavier est connecté l'adaptateur reste `active` (échantillonné 8 × 2 s). Mais
  `runtime_suspended_time = 430 343 ms` : l'adaptateur a passé **430 s suspendu entre le démarrage (11:25)
  et le ré-appairage (11:33)**, c'est-à-dire précisément pendant que le clavier devait l'appeler **[mesuré]**.
  Dans cet état la *Connection Request* du clavier n'arrive à l'hôte que par le réveil distant USB
  (*remote wake-up*) du contrôleur.

### 3.5 Effet de nos propres lectures et d'UPower sur la veille du clavier

Capture btmon passive (lecture seule, 11:37 → 12:27 ; brute supprimée car elle contient les frappes) **[mesuré]** :

| Émetteur | Requêtes radio | Cadence |
|---|---|---|
| UPower (lecture de `capacity` + `status` du `power_supply` HID → `GET_REPORT 0x47`) | 2 | toutes les 30-31 s, synchrones avec le champ `updated` d'UPower |
| `apple-kb-monitord` (`acquire`) | 16 (`0x47 ×3, 0xEA, 0xF5, 0x5A, 0x4F, 0xFF, 0x51-0x53, 0x46, 0x49, 0xF4, 0x4C, 0x09`) | toutes les 15 min |
| Outils de rétro-ingénierie d'autres agents (balayage 0x00-0xFF) | 400+ en 4 s, 218 `HANDSHAKE ERR_INVALID_REPORT_ID` | ponctuel |

* Chaque requête fait sortir la liaison du mode *sniff* (intervalle 12,5 ms) puis y revenir : 22 cycles
  `Exit Sniff Mode` → `Mode Change` en 10 min.
* Notre démon représente ≈ 16 requêtes / 15 min contre ≈ 60 pour UPower (≈ 21 %).
* Effet observé : la liaison ne se coupe jamais d'elle-même — 7 h continues la nuit du 30/09 au 01/10
  (04:13 → 11:02) et 2 jours du 27 au 29/09 **[mesuré]** (`journalctl -b -2`, création des nœuds uhid).
  Le clavier ne s'endort donc pas en usage normal ; il ne se retrouve « endormi » que quand **l'hôte**
  disparaît (redémarrage, extinction) ou quand on change ses piles.
* Les 11 `GET_REPORT request timed out` du 29/09 (15:09-15:21) et du 01/10 (04:00) précèdent ou
  accompagnent des déconnexions, pendant des sessions de rétro-ingénierie **[mesuré]**. Lien de cause à
  effet non établi **[hypothèse]**.

### 3.6 Perte de lien capturée en direct (2026-10-01 12:13)

Pendant la capture btmon passive, le lien est tombé **[mesuré]** (extrait assaini sans aucune frappe : `docs/captures/2026-10-01-coupure-12h13.txt`) :

| Heure | Trame | Fait |
|---|---|---|
| 12:11:27-28 | — | un outil de rétro-ingénierie lit en rafale des rapports **d'entrée** `0xFC`-`0xFF` (`GET_REPORT` type Input, 4 à 14 fois chacun) |
| 12:12:58 → 12:13:08 | — | échantillonneur RE : ~25 `GET_REPORT Feature` à 0,4 s d'intervalle (`0x46`, `0xFF`, `0x49` … `0xF6`, `0xF7`) ; chaque requête : *Exit Sniff* → *Mode Change Active* (~35 ms) → réponse `a3 …` → retour en sniff |
| 12:13:08.797 | #31 532 | `GET_REPORT Feature 0xFE` envoyé ; acquitté en bande de base (*Number of Completed Packets* à .805) |
| 12:13:08.799 | #31 534 | *Exit Sniff Mode* accepté par le contrôleur… mais **aucun *Mode Change*, aucune réponse, plus aucune trame du clavier** |
| 12:13:12 / 12:13:25 | — | `HIDP GET_REPORT request timed out` (dont la lecture UPower `0x47` de 12:13:21) |
| 12:13:28.823 | #31 538 | `Disconnect Complete`, raison **Connection Timeout (0x08)** : délai de supervision (20 s après la dernière trame), pas une déconnexion propre |
| 12:13:30 → 12:16:34 | — | BlueZ (plugin *policy* puis profil HID) appelle 9 fois : `Page Timeout` à chaque fois ; puis plus rien (RE-LIAISON en compte 7 jusqu'à 12:15:34, fin de sa copie de capture) |
| ≥ 12:16 | — | adaptateur repassé en autosuspend USB (`usb_watch2.log`) |

Lecture : le clavier est devenu **muet radio d'un coup**, au milieu d'une salve de lectures, et ne fait
plus de *page scan* ensuite. **[hypothèse]** Un clavier qui s'endort normalement enverrait un *LMP detach* (raison
*Remote*) plutôt que de disparaître par délai de supervision (comportement de veille du A1314 jamais capturé). Même signature à 04:00:13 la nuit précédente (perte « en
pleine salve » de l'échantillonneur, `docs/HARDWARE-RAPPORTS-HID.md` §5), retour seulement 13 min plus
tard, et le 29/09 15:09-15:21 (expirations GET_REPORT puis déconnexions pendant la rétro-ingénierie)
**[mesuré]**. **[hypothèse forte]** le firmware du clavier se bloque (ou redémarre) sous certaines
rafales de `GET_REPORT` sur des rapports non documentés ; il ne reprend qu'après une touche… ou une
extinction/rallumage. La même lecture de `0xFE` à 11:47:23 (balayage complet) n'avait rien cassé : ce
n'est pas un rapport « tueur » déterministe. *Contre-audit* : RE-HID-EXHAUSTIF §0.2 retient au contraire la forme
restreinte « un GET `0xFE` peut figer le micrologiciel » (2 blocages sur ~27 lectures, toujours juste après `0xF7`).
Les deux lectures sont compatibles (déclencheur non déterministe) ; **non tranché** sans le protocole E8 / §0.2.

### 3.7 Veille système

Aucune entrée `PM: suspend entry` dans tout le journal conservé (27/09 → 01/10) **[mesuré]** : la veille
du PC n'est pas à l'origine des épisodes observés. Elle est néanmoins gérée (§6).

## 4. Cause racine

**Ce qui est prouvé** :

1. **Ce n'est pas le pairage.** Aucune trace d'échec d'authentification dans aucun incident ; clé noyau =
   clé stockée ; `br-connection-create-socket` et `Host is down` = page timeout, pas clé refusée
   (§2, §3.1).
2. **Le lien tombe par silence radio du clavier, pas par veille propre** : délai de supervision
   (raison 0x08) en pleine rafale de `GET_REPORT`, capturé trame par trame à 12:13 (§3.6), même
   signature à 04:00 et le 29/09. Les rafales viennent des outils de rétro-ingénierie lancés sur ce
   poste ces derniers jours ; le trafic de fond (UPower 2 req./30 s, démon 16 req./15 min) multiplie les
   occasions (§3.5).
3. **Ensuite le clavier n'écoute plus** (page timeout à chaque appel) et **BlueZ abandonne** au bout de
   2 à 3 min (§3.3) ; personne ne relançait.
4. **La reconnexion par la touche n'a pas abouti** dans les épisodes journalisés (11:06-11:32 : aucun nœud
   uhid ni ligne bluetoothd) **[mesuré]** ; l'adaptateur était alors en autosuspend USB (§3.4).
5. **Le ré-appairage « répare » pour une autre raison que la clé** : il impose d'éteindre/rallumer le
   clavier (mode appairage), ce qui réinitialise un firmware bloqué, puis c'est l'**hôte** qui appelle un
   clavier qui écoute. **[hypothèse forte, cohérente avec 1-4]** : un simple éteindre/rallumer, sans
   désappairer, suffit.

**Cause racine retenue** : blocage (ou redémarrage) du firmware du clavier sous rafales de lectures HID,
aggravé côté hôte par l'abandon de BlueZ après 3 min et par un adaptateur en autosuspend USB au moment où
le clavier doit se reconnecter ; le ré-appairage n'était que la manipulation qui passait par un
redémarrage du clavier.

Hypothèses écartées : clé désynchronisée (§3.1), clavier absent de la liste d'acceptation (§3.1),
`pin_len` (§3.1), veille système (§3.7), écriture SET_REPORT (aucune dans le code, vérifié par recherche).

Reste à confirmer sur le matériel (§7) : (a) après une perte de ce type, une **touche** suffit-elle, ou
faut-il **éteindre/rallumer** ? (b) avec l'adaptateur **sans** autosuspend, la *Connect Request* du
clavier arrive-t-elle ?

## 5. Correctifs

### 5.1 Configuration système (fichiers du dépôt, appliqués par l'administrateur)

| Fichier | Effet | Pourquoi |
|---|---|---|
| `udev/61-akm-bt-adapter-no-autosuspend.rules` | `power/control=on` pour **ce seul** adaptateur `8087:0026` ; doit être après `60-autosuspend.rules` | supprime le maillon 3 (§3.4) ; coût ≈ 0,1 W sur un PC fixe |
| `bluetooth/akm-conf.py bluez` | `[General] FastConnectable = true` ; `Reconnect*` garantis dans `[Policy]` | *page scan* entrelacé : l'hôte répond plus vite au train de page court du clavier ; garde-fou contre le retour des clés dans la mauvaise section |
| `bluetooth/akm-conf.py upower` (optionnel) | `NoPollBatteries = true` | supprime les 2 GET_REPORT / 30 s (§3.5) ; le clavier peut enfin s'endormir et économiser ses piles. Le démon reste la source de la batterie (BatteryProvider BlueZ, lecture toutes les 15 min) |

Non retenu : `options btusb enable_autosuspend=0` (désactive l'autosuspend de **tous** les périphériques
btusb et demande un rechargement du module) ; `Experimental = false` (sans lien avec BR/EDR) ;
`IdleTimeout` dans `input.conf` (laisser à 0 : BlueZ ne doit pas couper un clavier inactif).

### 5.2 Démon `apple-kb-monitord`

* **Politique de lecture HID sûre** (`akm_core::read_policy`, #177) : le démon ne demande plus que
  `0x47` (pourcentage, rapport déclaré), `0x46` (tension mV) et `0x49` (tension filtrée) — 3 requêtes au
  lieu de 14, plus de sonde `0xEA`, jamais `0xFE` ni aucun rapport non déclaré, jamais de balayage ;
  **seulement si une touche a été pressée dans la dernière minute** (clavier éveillé, lien actif ; seul
  l'horodatage de la dernière frappe est conservé, jamais son contenu) ; un seul lecteur (mutex + `flock`
  sur `$XDG_RUNTIME_DIR/apple-kb-monitor/hid.lock`, à utiliser aussi par les outils RE) ; 1 s entre deux
  requêtes, budget 2 s, arrêt à la première erreur. Clavier inactif : aucune requête, le pourcentage vient du
  noyau.
* Réconciliation avec BlueZ (#165) : le gardien de liaison pousse à la machine d'acquisition l'ensemble des
  claviers réellement connectés (au démarrage, au retour de bluetoothd, au réveil, toutes les 5 min) ; la
  sonde « BlueZ absent » est bornée à 12 essais.
* `akm_core::recovery` — machine d'états pure, testée en temps simulé :
  `connected` · `dormant` (clavier endormi ou lien perdu depuis peu, pas d'alerte) · `unreachable`
  (après perte de lien / réveil / démarrage : aucune réponse depuis 10 min malgré ≥ 3 appels ; **une**
  notification par épisode) · `auth-failed` (raison `Authentication`, `key-missing`, pairage supprimé :
  **arrêt** des appels, notification « ré-appairage nécessaire ») · `suspended`.
* Appels `Device1.Connect` : jamais à moins de 20 s d'intervalle ; après une perte de lien 20 s, 40 s,
  1 min, 2 min puis toutes les 5 min ; après une heure toutes les 15 min ; clavier endormi : un appel
  toutes les 5 min puis 15 min (rattrape une reconnexion par touche ratée tant que le clavier est éveillé) ;
  réponse « occupé » de BlueZ : nouvel essai 30 s plus tard, non compté comme échec.
* Veille (logind) : inhibiteur `delay` ; à `PrepareForSleep(true)` plus aucune lecture HID ni appel, attente
  de la fin d'une lecture en cours (≤ 3 s) puis libération ; au réveil délai de grâce de 4 s puis reprise
  rapide.
* État publié sur le bus de session : objet `/com/agenceapi/AppleKbMonitor1/Link`, interface
  `com.agenceapi.AppleKbMonitor1.Link`, méthodes `Status()` (JSON : `mac`, `name`, `health`, `since`,
  `attempts`, `failures`, `last_error`, `last_reason`, `updated`) et `Reconnect()` (soumis aux mêmes
  20 s minimum). Vérifié en réel le 01/10 (instance de test) : `health = connected`, inhibiteur logind
  `delay` actif.

### 5.3 Outils

* `akmctl doctor` — une commande : état BlueZ, empreinte de la clé (jamais la clé), liste d'acceptation,
  autosuspend de l'adaptateur, `main.conf`, UPower, journaux bluetoothd classés, hidraw, santé du démon,
  verdict et action conseillée.
* `akmctl repair` — d'abord sans rien casser : touche + appels espacés, puis **éteindre/rallumer le
  clavier** + appels ; ré-appairage seulement ensuite, et seulement après la saisie de `OUBLIER` sur un
  terminal ; refuse si la liaison est saine. Menu tray : « Réparer la liaison… ».

### 5.4 Oubli propre d'un clavier connecté, comme macOS (#217)

Quand `akmctl repair` doit supprimer le pairage alors que le clavier est **encore connecté** (`--force` sur une liaison
vivante, ou refus de clé constaté pendant une connexion), il reproduit ce que fait macOS 26.5 en « Oublier » :

| Étape | Ce qui est fait | Preuve |
|---|---|---|
| 1 | pré-vol : clavier connecté, santé `connected` dans `akmctl doctor` et aucun KO, disjoncteur fermé, stdin et stdout sont un terminal | — |
| 2 | **sauvegarde** côté poste : `~/.local/state/apple-kb-monitor/forget-backup-<AAAAMMJJTHHMMSSZ>.json` (0600, jamais écrasé) : MAC, nom, alias, `Paired`/`Bonded`/`Trusted`, chemin et adresse de l'adaptateur, chemin de l'appareil. **Aucune clé de lien** (jamais lue) | — |
| 3 | explication (ce qui va se passer, effet de `0x41` non mesuré, marche à suivre pour ré-appairer) puis saisie de `OUBLIER` ; jamais en non interactif | — |
| 4 | **un** SET Feature `0x41` `RecantConnection`, id seul, fil `53 41` (opération `Forget` du registre, une fois par session, porte d'un octet, octets journalisés) | [décompilé + listing] `bluetoothd` `FUN_1005a41e4` (RE-GHIDRA-IOBLUETOOTH.md §5.2) |
| 5 | le démon coupe la notification de déconnexion (`ExpectDisconnect()`, 15 s ; équivalent de `SuppressDisconnectNotifications`) | [désassemblage] `-[AppleBluetoothHIDDevice recantConnection]` |
| 6 | attente **2000 ms** de la chute de la liaison ; si elle ne tombe pas, on continue comme Apple (« timed out waiting to recant ») | [décompilé] `FUN_1005a3f64` |
| 7 | **seulement alors** `org.bluez.Adapter1.RemoveDevice` | [décompilé] `FUN_1005a3f64` (« will unpair ») |
| 8 | assistant d'appairage existant, attente du clavier appairé + connecté, `Trusted`, puis vérification finale `akmctl doctor` | — |

* Si `0x41` n'est pas envoyé (porte hidraw indisponible, verrou pris) ou n'est pas accepté (erreur, clavier muet) : **rien
  n'est supprimé**, retour à l'étape « réveil + reconnexion », aucune nouvelle tentative.
* **Disjoncteur ouvert** (règle Apple R3, #251 : trois silences consécutifs, plus aucune émission jusqu'à une nouvelle
  connexion ou une veille) : le pré-vol l'apprend du démon (D-Bus, sinon l'état publié
  `$XDG_RUNTIME_DIR/apple-kb-monitor/breaker.state`) ; `0x41` n'est **pas** écrit (le clavier est muet, l'ordre ne serait
  pas entendu), l'étape « réveil + reconnexion » existante est enchaînée, puis l'utilisateur est informé (le disjoncteur
  se referme à la reconnexion ; relancer `akmctl repair` si l'oubli reste nécessaire). La porte d'écriture elle-même
  (`hidraw::WriteDoor`) applique le même verdict : un `akmctl` lancé pendant que le démon a ouvert le disjoncteur n'écrit
  ni `0x41` ni `0x55`.
* Arrêt au premier échec ; chaque octet (`[hid-write] Forget …`) et chaque décision (`[forget] …`) sont journalisés sur stderr.
* Un clavier **non connecté** est désappairé comme avant, sans `0x41` : macOS n'envoie `RecantConnection` qu'à un appareil
  connecté.
* `0x41` n'est atteignable que par `akmctl repair` (un test refuse toute autre référence à l'opération `Forget` ou à
  `forget::run`) : ni D-Bus, ni fenêtre, ni tray (qui ne fait qu'ouvrir `akmctl repair` dans un terminal, où `OUBLIER`
  doit être tapé). Aucun test n'écrit sur le clavier ni n'appelle `RemoveDevice` : simulateur et espion uniquement.
* **Effet réel sur le clavier : non mesuré** (simple coupure ou oubli de cet hôte, RE-GHIDRA-IOBLUETOOTH.md §9 n° 3).
  `0x44` `FullFactoryDefault` (Lion, efface toutes les clés) reste interdit.

## 6. Veille du clavier et du PC : comportement attendu après correctifs

| Situation | Avant | Après |
|---|---|---|
| Lectures du démon | 14 rapports / 15 min, clavier actif ou non | 3 rapports, seulement après une frappe récente |
| Clavier inactif | jamais endormi (UPower 2 req./30 s) | s'endort si `NoPollBatteries` ; `dormant`, revient à la touche ; filet : appel toutes les 5 puis 15 min |
| PC en veille | lectures HID possibles pendant la coupure, attente passive au réveil | lectures suspendues avant la coupure ; au réveil appels à +4 s, +24 s, +64 s… |
| Redémarrage du PC | BlueZ abandonne en 3 min ; ré-appairage | adaptateur jamais suspendu ; appels espacés sans limite de durée ; alerte claire à 10 min |
| Clé réellement refusée | indiscernable d'un clavier endormi | `auth-failed`, notification, `akmctl repair` |
| Clavier muet après une rafale de lectures | ré-appairage | notification « appuyez sur une touche, sinon éteignez/rallumez » ; le pairage est conservé |

## 7. Test contrôlé sur le matériel (à faire avec le gérant)

Pré-requis : le gérant est devant le clavier, un autre moyen de saisie est disponible, **aucun outil de
rétro-ingénierie ne tourne**.

Premier test, sans aucune commande (après une coupure comme celle de 12:13) : appuyer sur une touche ;
si rien en 10 s, éteindre/rallumer le clavier **sans l'oublier** dans Plasma. S'il revient, le pairage
n'était pas en cause (§4, point 5).

```sh
tests/live/reconnect_probe.sh            # btmon passif + état USB + journal, puis :
#   1. bluetoothctl disconnect 04:DB:56:CA:42:EE   (une seule fois)
#   2. le gérant attend 60 s puis appuie sur UNE touche
#   3. le script attend 120 s la reconnexion et classe ce qu'il voit
```

Lecture du résultat : `Connect Request` du clavier reçu et accepté → la reconnexion par touche
fonctionne dans cet état ; aucun `Connect Request` alors que l'adaptateur était `suspended` → rôle de
l'autosuspend confirmé ; `Link Key Request Negative Reply` / `Authentication Failure` → clé en cause (alors
`akmctl repair`).

## 8. Risques restants

* Le blocage du firmware du clavier est déduit de trois coupures concordantes, pas démontré par une
  reproduction volontaire (qui exigerait de marteler le clavier : exclu). La politique de lecture sûre en
  supprime la cause côté démon ; UPower (2 lectures `0x47` / 30 s par le noyau) reste tant que
  `NoPollBatteries` n'est pas appliqué.
* Reconnexion par la touche avec l'adaptateur en autosuspend : non capturée tant que le test du §7 n'a pas
  été fait ; la règle udev neutralise ce chemin quoi qu'il en soit.
* Un clavier qui appaire **un autre hôte** (il n'en mémorise qu'un) se comportera exactement comme une clé
  refusée : seul `akmctl repair` le ramène.
* Les sessions de rétro-ingénierie qui balaient les Report ID (y compris les rapports d'entrée, 12:11:27)
  sont la cause la plus probable des coupures récentes : à ne lancer que sous le verrou `hid.lock`, jamais
  pendant l'usage normal du clavier.
* Le gérant a « oublié » puis ré-appairé le clavier à 11:32 et l'a de nouveau oublié à 12:24 (Plasma) :
  chaque ré-appairage crée une nouvelle clé ; aucun ne corrigeait une clé fausse.
* `~/.config/apple-kb-monitor/config.toml` contient la configuration de `mqtt-bridge` (avertissements
  `unknown key [mqtt]` au démarrage) : sans effet sur la liaison, à séparer.
