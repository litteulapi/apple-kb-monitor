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

Capture btmon passive (lecture seule, 11:37 → 12:27) **[mesuré]** :

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

### 3.6 Veille système

Aucune entrée `PM: suspend entry` dans tout le journal conservé (27/09 → 01/10) **[mesuré]** : la veille
du PC n'est pas à l'origine des épisodes observés. Elle est néanmoins gérée (§6).

## 4. Cause racine

**Chaîne prouvée** :

1. L'hôte disparaît (redémarrage à froid à 11:02) → le clavier perd le lien et, faute d'hôte, s'endort.
2. Au démarrage, BlueZ appelle le clavier : `Host is down` (page timeout) parce qu'un clavier endormi
   n'écoute pas **[mesuré + source]**. BlueZ abandonne après 3 min au plus **[source]**, rien ne relance.
3. La seule voie restante est la reconnexion **initiée par le clavier** (touche). Elle n'a pas abouti entre
   11:06 et 11:32 (aucun nœud uhid, aucune ligne bluetoothd) **[mesuré]**, alors que la clé était bonne et
   le clavier dans la liste d'acceptation.
4. Le ré-appairage « répare » parce qu'il inverse le sens : clavier en mode découvrable (il écoute) et
   **hôte** qui appelle, adaptateur réveillé par la recherche. La clé, elle, n'était pas en cause
   (aucune trace d'échec d'authentification ; clé identique après coup).

**Maillon non encore capturé (3)** : pendant toute cette fenêtre l'adaptateur était en autosuspend USB
(§3.4). **[hypothèse principale]** la *Connection Request* du clavier (train de page de quelques secondes)
n'est pas servie à temps quand l'AX201 doit d'abord réveiller le bus USB ; c'est le seul élément de
l'hôte qui diffère entre « clavier connecté » (adaptateur actif, tout marche) et « clavier à reconnecter »
(adaptateur suspendu, rien ne marche). Hypothèses écartées : clé désynchronisée (§3.1), clavier absent de
la liste d'acceptation (§3.1), `pin_len` (§3.1), veille système (§3.6), écriture SET_REPORT (aucune dans
le code, vérifié par recherche).

Le test contrôlé du §7 tranche ce maillon (capture btmon de la touche).

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
  `com.agenceapi.AppleKbMonitor1.Link` (`Health`, `Since`, `Attempts`, `Failures`, `LastError`,
  `LastReason`, méthode `Reconnect()`).

### 5.3 Outils

* `akmctl doctor` — une commande : état BlueZ, empreinte de la clé (jamais la clé), liste d'acceptation,
  autosuspend de l'adaptateur, `main.conf`, UPower, journaux bluetoothd classés, hidraw, santé du démon,
  verdict et action conseillée.
* `akmctl repair` — ré-appairage guidé ; ne supprime jamais le pairage sans la saisie explicite de
  `OUBLIER` ; refuse si la liaison est saine. Menu tray : « Réparer la liaison… ».

## 6. Veille du clavier et du PC : comportement attendu après correctifs

| Situation | Avant | Après |
|---|---|---|
| Clavier inactif | jamais endormi (UPower 2 req./30 s) | s'endort si `NoPollBatteries` ; `dormant`, revient à la touche ; filet : appel toutes les 5 puis 15 min |
| PC en veille | lectures HID possibles pendant la coupure, attente passive au réveil | lectures suspendues avant la coupure ; au réveil appels à +4 s, +24 s, +64 s… |
| Redémarrage du PC | BlueZ abandonne en 3 min ; ré-appairage | adaptateur jamais suspendu ; appels espacés sans limite de durée ; alerte claire à 10 min |
| Clé réellement refusée | indiscernable d'un clavier endormi | `auth-failed`, notification, `akmctl repair` |

## 7. Test contrôlé sur le matériel (à faire avec le gérant)

Pré-requis : le gérant est devant le clavier et un autre moyen de saisie est disponible.

```sh
tests/live/reconnect_probe.sh            # btmon passif + état USB + journal, puis :
#   1. bluetoothctl disconnect 04:DB:56:CA:42:EE   (une seule fois)
#   2. le gérant attend 60 s puis appuie sur UNE touche
#   3. le script attend 120 s la reconnexion et classe ce qu'il voit
```

Lecture du résultat : `Connection Request` du clavier reçu et accepté → la reconnexion par touche
fonctionne dans cet état ; aucun `Connection Request` alors que l'adaptateur était `suspended` → maillon 3
confirmé ; `Link Key Request Negative Reply` / `Authentication Failure` → clé en cause (alors
`akmctl repair`).

## 8. Risques restants

* Maillon 3 non capturé tant que le test du §7 n'a pas été fait ; la règle udev le neutralise quelle que
  soit sa nature exacte côté USB, pas s'il venait du firmware radio.
* Un clavier qui appaire **un autre hôte** (il n'en mémorise qu'un) se comportera exactement comme une clé
  refusée : seul `akmctl repair` le ramène.
* Les sessions de rétro-ingénierie qui balaient tous les Report ID provoquent des rafales d'erreurs HIDP ;
  à ne pas lancer pendant une mesure de stabilité.
* `~/.config/apple-kb-monitor/config.toml` contient la configuration de `mqtt-bridge` (avertissements
  `unknown key [mqtt]` au démarrage) : sans effet sur la liaison, à séparer.
