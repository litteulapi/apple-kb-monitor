# Revue d'architecture — driver clavier Bluetooth Apple

> Branche `audit/debug-complet`, revue du 2026-10-01. Périmètre : **le clavier uniquement** (HID/hidraw, batterie, BlueZ, reconnexion, RSSI, LED, touches F, udev/permissions). L'écran (DDC/CI) est hors périmètre.
> Code relu : `apihub-app/src/keyboard.rs` (641 l.), `bluez.rs` (352), `rssi.rs` (227), `brightness.rs` (298, côté entrée seulement), la boucle de polling de `main.rs` (l. 357-572), `rssi-helper.c`, `udev/`, `modprobe/`, `keyd/`, `dbus/`, et le CLI Python `apple-kb-monitor` (parties clavier).
> Toutes les affirmations marquées **[mesuré]** ont été vérifiées sur le poste (Manjaro, noyau 7.1.13, BlueZ 5.87, UPower 1.91.3, systemd 261, keyd 2.6.0), avec un A1314 ISO (`05AC:0256`, « Clavier de maria #1 ») **connecté** pendant la revue. Celles marquées **[source]** renvoient au code amont (noyau `torvalds/linux` master, BlueZ master, UPower master, keyd master), téléchargé et relu.

---

## 1. Verdict

**Garder le principe, refaire la colonne vertébrale.**

| Brique | Verdict | Raison principale |
|---|---|---|
| Lecture des Feature Reports par `hidraw` + `HIDIOCGFEATURE` | **Garder**, comme diagnostic secondaire | Seul moyen d'obtenir tension, ADC, firmware et calibration du BCM2042. `hidraw` est la bonne interface (libusb ne voit pas les périphériques Bluetooth). Mais ce ne doit plus être la source du pourcentage. |
| Pourcentage batterie calculé par l'app | **Refaire** : lire le `power_supply` du noyau | Le noyau expose déjà la batterie de ce clavier (`hid-04:db:56:ca:42:ee-battery-71`, 90 %) et UPower l'affiche déjà sans l'app **[mesuré]**. |
| Battery Provider BlueZ (`bluez.rs`) | **Garder mais refondre** | UPower **masque** la batterie BlueZ quand une batterie noyau a le même numéro de série (le MAC) **[source]**. Le provider ne sert donc qu'aux clients qui lisent `org.bluez.Battery1` directement (applet Bluetooth de Plasma). Il ne fonctionne aujourd'hui que grâce à une politique D-Bus modifiée à la main dans `/etc`, différente de celle du dépôt **[mesuré]**. |
| Polling toutes les ~40 s de 14 Feature Reports | **Refaire** en événementiel + cadence lente | Chaque lecture réveille le lien radio : 629 ms pour la première lecture, ~17 ms ensuite **[mesuré]**. Cela fait 30 240 requêtes par jour, en plus des 2 880 d'UPower (une toutes les 30 s **[mesuré]**). Pour des piles AA, une mesure toutes les 10-15 min suffit. |
| Détection de modèle (table des 10 PID) | **Refaire** | 6 des 10 entrées sont fausses par rapport à `hid-ids.h`. Les Magic Keyboard en Bluetooth ont le vendor `0x004C`, pas `0x05AC` : ils sont tous invisibles pour l'app, le Python, la règle udev et keyd **[source]**. |
| Permissions (groupe `input`, règle `99-…`) | **Refaire** | Le groupe `input` donne à l'utilisateur la lecture de **toutes** les frappes de tous les claviers. `TAG+="uaccess"` dans une règle `70-…` suffit et a été validé avec `udevadm test` **[mesuré]**. Aujourd'hui, `/dev/hidraw7` est en `root:root 0600` sur le poste : l'app ne lit **rien** en utilisateur **[mesuré]**. |
| RSSI via MGMT `GET_CONN_INFO` | **Garder le protocole, changer le porteur** | L'implémentation est correcte, mais l'appel non privilégié est refusé avec le statut `0x14 PERMISSION_DENIED` **[mesuré]**. Dans l'app GUI, le RSSI vaut donc toujours `None`. Il faut un petit helper avec `cap_net_admin`, ou abandonner le RSSI. |
| Moniteur de réveil (Input Report 0x13) | **Garder**, corriger le démarrage | Le report 0x13 (vendor `FF01`) est bien déclaré dans le descripteur **[mesuré]**. Mais le thread n'est lancé que si le clavier est présent au démarrage de l'app (`main.rs:363`). |
| LED (clignotement CapsLock) | **Refaire** | keyd fait un `EVIOCGRAB` sur le clavier, et le noyau ignore alors les événements injectés par une autre poignée (`input_inject_event`) **[source]**. `set_led()` « réussit » (24 octets écrits) mais rien ne s'allume. |
| F1/F2 par lecture evdev du clavier virtuel keyd | **Refaire** côté entrée | Cette lecture impose le groupe `input`, qui équivaut à un keylogger. Un raccourci global KDE (KGlobalAccel), déjà amorcé dans `kde/shortcuts/`, n'a besoin d'aucun droit. |
| `modprobe/hid_apple.conf` (`fnmode=1`) | **Supprimer** (redondant) | Le défaut du noyau est `fnmode=3` (auto), qui donne `1` pour un clavier Apple d'origine **[source]**. |

En une phrase : **le noyau et UPower doivent être la source de vérité pour l'état et la batterie, BlueZ (signaux D-Bus) pour la connexion, et `hidraw` un complément de diagnostic lu rarement, par un seul thread dédié au clavier, avec des droits `uaccess` et sans nom D-Bus réservé.**

---

## 2. Faits établis

### 2.1 Le noyau gère déjà la batterie de ces claviers

- `drivers/hid/hid-input.c`, table `hid_battery_quirks[]` **[source]** : les A1314 (`ALU_WIRELESS_2011_ANSI/ISO` = `0x0255/0x0256`), les 2009 (`0x0239/0x023a`) et les A1255 (`ALU_WIRELESS_ANSI` = `0x022c`) ont `HID_BATTERY_QUIRK_PERCENT | HID_BATTERY_QUIRK_FEATURE`. Le noyau crée un `power_supply` et, à chaque lecture de `capacity`, envoie un `GET_REPORT` *Feature* sur le report qui porte l'usage *Battery Strength* (page `0x06`, usage `0x20`).
- `drivers/hid/hid-apple.c` **[source]** : tous les Magic Keyboard (2015 `0x0267`, numpad 2015 `0x026c`, 2021 `0x029c/0x029a/0x029f`, 2024 `0x0320-0x0322`) ont `APPLE_RDESC_BATTERY`. Le pilote corrige leur descripteur et interroge la batterie lui-même toutes les 60 s (`APPLE_BATTERY_TIMEOUT_SEC`).
- Sur le poste **[mesuré]** :
  ```
  /sys/class/power_supply/hid-04:db:56:ca:42:ee-battery-71 -> …/uhid/0005:05AC:0256.0014/power_supply/…
  type=Battery scope=Device present=1 online=1 status=Discharging capacity=90
  upower : battery_hid_04odbo56ocao42oee_battery_71, serial 04:db:56:ca:42:ee, type keyboard, 90 %
  ```
  Le suffixe `-71` correspond à `0x47` : c'est **exactement** le report `HID_BATTERY_STANDARD` que lit `keyboard.rs`. Le nommage `hid-%s-battery-%d` est récent. Le CLI Python cherche encore `hid-{mac}-battery` (sans suffixe) : ce chemin est **périmé** sur ce noyau.
- UPower relit cette batterie **toutes les 30 s** **[mesuré]** (`upower --monitor-detail` : 02:00:02, 02:00:32, 02:01:02, 02:01:33).

### 2.2 UPower masque le doublon BlueZ

`src/linux/up-backend.c`, `update_added_duplicate_device()` **[source]** : quand deux périphériques ont le même `serial` et que l'un vient de BlueZ, **c'est le périphérique BlueZ qui est désenregistré** (« Hiding duplicate device »). La batterie que `bluez.rs` pousse vers BlueZ n'atteint donc jamais UPower, PowerDevil ni l'applet batterie de Plasma pour un A1314. Elle n'apparaît que dans les clients qui lisent `org.bluez.Battery1` en direct, comme l'applet Bluetooth (bluez-qt) et `DeviceItem.qml`.

Conséquence actuelle : deux chiffres différents s'affichent sur le même bureau. L'applet batterie montre la valeur noyau `0x47`, l'applet Bluetooth la valeur fine `0xEA` du provider.

### 2.3 Chemin d'une Feature Report sur ce poste

BlueZ 5.87 : `UserspaceHID` vaut `true` par défaut (`/etc/bluetooth/input.conf`) **[mesuré]**. Le clavier passe donc par **uhid**, pas par `hidp` (`/sys/devices/virtual/misc/uhid/0005:05AC:0256.0014`). Une lecture suit ce chemin : `ioctl(HIDIOCGFEATURE)` → `hidraw_get_report` → `uhid` (`report_lock`, attente maximale **5 s**, `uhid.c:196`) → `bluetoothd` (une seule requête en cours, `REPORT_REQ_TIMEOUT 3` s, `EBUSY` sinon, `profiles/input/device.c`) → radio.

- Les lectures concurrentes (UPower et l'app) sont **sérialisées par le noyau** (`report_lock`). Trois `cat capacity` lancés en parallèle × 3 ont donné 9/9 succès, sans `EBUSY` **[mesuré]**. Le risque n'est pas l'échec mais **l'attente** : jusqu'à 5 s par report quand le clavier ne répond pas.
- `hidraw` ne vérifie pas le report ID par rapport au descripteur. Or le descripteur réel du A1314 (224 octets, analysé) ne déclare que les reports `0x01, 0x09 (Feature FF01), 0x11, 0x12, 0x13 (Input FF01), 0x47 (Battery Strength)` **[mesuré]**. Les reports `0xEA, 0xF5, 0x5A, 0x4F, 0xFF, 0x51-0x53, 0x46, 0x49, 0xF4, 0x4C` sont **non déclarés**. Ce sont des registres internes du BCM2042 qui répondent en pratique, mais rien ne garantit qu'un autre firmware ou une autre puce (BCM20733 des Magic Keyboard) y réponde de la même façon.

### 2.4 Table des modèles : 6 entrées sur 10 fausses

Comparaison de `keyboard.rs::APPLE_PIDS` avec `drivers/hid/hid-ids.h` **[source]** :

| PID | Le code dit | Le noyau dit |
|---|---|---|
| `0x0220` | A1016 sans fil blanc | `ALU_ANSI` (clavier alu **filaire** USB) |
| `0x0229` | A1255 ANSI | `GEYSER4_HF_ANSI` (clavier **interne** MacBook) |
| `0x022C` | A1255 JIS | `ALU_WIRELESS_ANSI` (A1255 **ANSI**) ; ISO = `0x022d`, JIS = `0x022e` |
| `0x024F`/`0x0250` | Magic Keyboard A1644 | `ALU_REVB_ANSI/ISO` (clavier alu **filaire** rév. B) |
| `0x0267` | Magic Keyboard Touch ID A2449 | `MAGIC_KEYBOARD_2015` (A1644) |
| `0x026C` | Touch ID ISO | `MAGIC_KEYBOARD_NUMPAD_2015` (A1843) |
| `0x0255-0x0257` | A1314 | correct |

Il manque `0x0239-0x023b` (A1314 2009), `0x022d/0x022e`, `0x029a/0x029c/0x029f` (2021, dont Touch ID) et `0x0320-0x0322` (2024).

Les Magic Keyboard s'annoncent en Bluetooth avec le **vendor `0x004C`** (`BT_VENDOR_ID_APPLE`, utilisé par `hid-apple.c` dans `HID_BLUETOOTH_DEVICE(BT_VENDOR_ID_APPLE, …)`). Le filtre `05AC` exclut donc tous les claviers récents, à la fois dans `apple_model_from_uevent`, dans le Python (`APPLE_VID`), dans la règle udev (`DEVPATH=="*05AC*"`) et dans keyd (`[ids] 05ac:0256`, un seul modèle). Ce constat complète l'issue #23.

### 2.5 Permissions et D-Bus

- Sur le poste, `/dev/hidraw7` est en `crw------- root root`, avec `TAGS=:seat:` **[mesuré]**. La règle du dépôt n'est pas installée, donc **aucune télémétrie HID n'est possible en utilisateur aujourd'hui**.
- `udevadm test --extra-rules-dir` a été lancé sur `hidraw7` avec la règle proposée (§4.3) **[mesuré]**. Elle ajoute `uaccess` à `CURRENT_TAGS` et `73-seat-late.rules:16 RUN{builtin}+="uaccess"` est planifié. Elle doit porter un numéro **< 73**, sinon le tag arrive après le builtin qui pose l'ACL. Avec `99-…`, `TAG+="uaccess"` serait sans effet.
- La politique D-Bus du dépôt (`dbus/…conf`) n'autorise `own` que pour `root`. Celle de `/etc/dbus-1/system.d/` sur le poste, qui n'appartient à aucun paquet, l'autorise aussi en `context="default"`. Un `RequestName` en utilisateur réussit donc ici (`uint32 1`), alors qu'un nom non autorisé est refusé (`AccessDenied`) **[mesuré]**. **Installée depuis le PKGBUILD, l'app lancée en utilisateur échoue au `build()` zbus** (`[bluez] D-Bus setup failed`) et le provider ne démarre jamais. Or BlueZ n'a **pas besoin** d'un nom bien connu : il mémorise l'adresse unique de l'appelant de `RegisterBatteryProvider` (`g_dbus_client_new_full(conn, sender, …)` dans `src/battery.c`) **[source]**. `bluetoothd` tourne en root et a le droit d'appeler `ObjectManager`/`Properties` chez n'importe qui (`bluetooth.conf`) **[mesuré]**.
- `BatteryProviderManager1` n'est plus expérimental dans BlueZ master : il est créé sans condition dans `adapter.c` **[source]** et présent sur `/org/bluez/hci0` du poste **[mesuré]**.

### 2.6 RSSI

- `GET_CONN_INFO` (`0x0031`) ne figure pas dans `mgmt_untrusted_commands` (`net/bluetooth/mgmt.c`) **[source]**. Un socket de contrôle non privilégié reçoit `CMD_STATUS`, opcode `0x0031`, statut `0x14` **[mesuré]** : `rssi-helper` compilé depuis le dépôt et lancé en utilisateur sous `strace` reçoit `"\2\0\0\0\3\0001\0\24"`. L'app GUI n'a ni `setcap` ni helper installé par le PKGBUILD : **le RSSI est toujours absent**.
- D-Bus n'est pas une solution : BlueZ ne publie pas `Device1.RSSI` pour un périphérique BR/EDR connecté (seulement pendant la découverte). MGMT reste donc le bon protocole.
- Le noyau met déjà la réponse en cache 1 à 3 s (`conn_info_min_age/max_age`). Interroger plus d'une fois toutes les 10 s n'a pas de sens.

### 2.7 LED et touches

- `input_inject_event()` (`drivers/input/input.c:412`) **[source]** : `if (!grab || grab == handle) input_handle_event(…)`. keyd saisit le clavier (`EVIOCGRAB` dans `src/device.c:382`), donc les écritures `EV_LED` de `set_led()` sur l'evdev Apple sont **ignorées en silence**. keyd **propage** en revanche les `EV_LED` reçus par son clavier virtuel vers tous les claviers qu'il saisit (`daemon.c:572`) **[source]** : c'est la voie correcte.
- `read_led_state()` parcourt **toutes** les LED `*capslock*` du système, et la dernière lue l'emporte. Ce n'est pas forcément celle du clavier Apple.
- `fnmode` : défaut `3` (auto). Pour un clavier Apple d'origine, `real_fnmode = 1` (`hid-apple.c:436-446`) **[source]**. Le paramètre actuel est `1` **[mesuré]**. Le fichier modprobe ne change donc rien.

### 2.8 Revue du code existant (sûreté, threads, cycle de vie)

`keyboard.rs`
- `HIDIOCGFEATURE` vaut `0xC1004807` = `_IOWR('H', 0x07, 256)`. La constante est **correcte** et le buffer fait bien 256 octets. Le noyau renvoie au plus `count` octets, donc la tranche `buf[..ret]` est sûre.
- Le fd brut vit dans un `static Mutex<Option<(c_int, String)>>`. `fcntl(F_GETFD)` ne détecte **pas** un périphérique retiré : le fd reste valide côté noyau. Seul l'échec de la sonde `0xEA` provoque la fermeture. C'est fonctionnel, mais fragile ; `OwnedFd` serait préférable.
- La sonde `0xEA` sert de test de connexion, alors qu'il s'agit d'un report **non déclaré**. Le jour où un firmware ne le connaît pas, le clavier est déclaré « not found » en permanence.
- Le cycle lit 14 reports, pas les « 21 » annoncés par `ARCHITECTURE.md`. Le nom, le firmware, la calibration, l'identité et la référence ADC sont statiques mais relus à chaque cycle.
- `0x46` (`bluetooth.connected = true`) : la « connexion » est déduite de la réussite d'une lecture HID, et non de `Device1.Connected`.
- Un seul clavier (le premier trouvé), un seul adaptateur (`hci0` en dur dans `bluez.rs`, index MGMT `0` en dur dans `rssi.rs` et `rssi-helper.c`).

`main.rs` (boucle de polling)
- La lecture HID clavier tourne **dans le même thread** que le DDC et les sous-processus KWin. Quand le clavier ne répond plus, un cycle peut bloquer jusqu'à 5 s par report.
- Les commentaires de cadence sont faux : « every 2nd cycle (~10s) » pour `cycle % 4` avec un sommeil de 10 s donne 40 s ; « ~15s » pour l'historique donne 300 s.
- Le wake-monitor n'est lancé que si `find_apple_hidraw()` réussit au démarrage.
- Le provider est créé une seule fois avec le MAC du premier clavier, et n'est jamais retiré à la déconnexion. `Battery1` reste alors affiché avec une valeur figée.

`bluez.rs`
- Le `ObjectManager` est écrit à la main alors que zbus 4.4 fournit `zbus::fdo::ObjectManager` (`fdo.rs:290`), qui émet `InterfacesAdded/Removed` tout seul. La boucle de 1 s compare le pourcentage pour émettre `PropertiesChanged` ; il suffit d'émettre le signal au moment de la mise à jour.
- Aucun test ne couvre l'enregistrement réel auprès de BlueZ. Un test « live » temporaire (`tmp_live`, ajouté puis retiré par un autre agent pendant la revue) a montré le besoin : à transformer en test `#[ignore]` documenté plutôt qu'en ajout ponctuel.

`rssi.rs` / `rssi-helper.c` : le protocole est correct depuis `af0a4a4` (réponse appariée, `127` rejeté, MAC stricte). Seul le porteur pose problème (§2.6).

`brightness.rs` (entrée seulement) : `EVIOCGNAME(256)` = `0x81004506`, correct. Le `input_event` de 24 octets est propre à x86_64, ce qui est cohérent avec `arch=('x86_64')`. `SYN_DROPPED` n'est pas traité. Le vrai problème est le droit d'accès (§2.7).

Python `apple-kb-monitor` : il contient un second provider BlueZ sur **le même chemin** (`PROVIDER_ROOT`). Si le démon systemd et l'app tournent ensemble, BlueZ refuse le second (`error registering battery: path exists`, `src/battery.c:262`) **[source]**, donc le gagnant dépend de l'ordre de lancement. `hid_get_feature` renvoie le buffer complet au lieu de la longueur rendue par l'ioctl. Le chemin `power_supply` est périmé (§2.1).

---

## 3. Architecture cible

```
                ┌──────────────── thread « kb-actor » (unique propriétaire du clavier) ───────────────┐
 BlueZ D-Bus ──►│ signaux Device1.PropertiesChanged(Connected)  ─┐                                     │
 (zbus)         │ InterfacesAdded/Removed (appairage, adaptateur)│                                     │
                │                                                ▼                                     │
 sysfs ────────►│ resolve(MAC) → { hidraw, power_supply, evdev, leds }  (scan ciblé, déclenché par     │
                │                                                       événement, pas par minuterie)  │
                │  ├─ batterie  : power_supply/capacity (noyau)   ← source de vérité, toutes les 5 min │
                │  ├─ diag HID  : 0xF5/0xEA/0x09 toutes les 15 min ; statiques 1×/connexion            │
                │  ├─ réveil    : read() bloquant sur hidraw, report 0x13 → relecture immédiate        │
                │  └─ RSSI      : D-Bus → helper privilégié (optionnel), 1×/60 s si l'UI est visible   │
                │                                                                                      │
                │  publie KbSnapshot (Arc<Mutex>/canal) ──► UI, tray, MQTT, historique                 │
                │  publie pourcentage noyau ──► BatteryProvider (un objet par clavier connecté)        │
                └──────────────────────────────────────────────────────────────────────────────────────┘
```

Principes :
1. **Une seule source pour le pourcentage** : le `power_supply` du noyau, que lisent aussi UPower et Plasma. Si l'utilisateur choisit la valeur fine `0xEA`, elle est affichée comme un **détail** dans l'UI, jamais publiée vers BlueZ. On supprime ainsi l'écart entre les deux applets.
2. **La connexion est signalée par BlueZ**, pas déduite d'un échec d'ioctl. Avec `Connected=false`, aucune lecture HID n'est faite, donc aucun blocage de 5 s.
3. **HID brut en diagnostic** : 3 reports dynamiques toutes les 15 min, et les reports statiques (`0x4F, 0xFF, 0x51-53, 0x5A, 0xF4, 0x4C, 0x46, 0x49`) **une fois par connexion**. Le passage de 30 240 à environ 288 lectures par jour soulage la radio du clavier.
4. **Gating par famille** : la télémétrie BCM2042 ne s'applique qu'aux PID `0x022c-0x022e`, `0x0239-0x023b` et `0x0255-0x0257`. Pour les Magic Keyboard (`0x004C:*`), seuls le noyau et BlueZ sont utilisés, sans aucun report non déclaré.
5. **Multi-clavier, multi-adaptateur** : la clé est le MAC, et le chemin BlueZ est résolu par `GetManagedObjects` au lieu de `hci0` en dur.
6. **Droits minimaux** : `uaccess` sur le `hidraw` des claviers Apple seulement, pas de groupe `input`, pas de nom D-Bus réservé, et un `cap_net_admin` confiné à un binaire de 100 lignes.

Faut-il passer par UPower en D-Bus plutôt que par sysfs ? Lire `capacity` en sysfs déclenche un `GET_REPORT`, comme le fait déjà UPower. S'abonner à `org.freedesktop.UPower.Device` (`PropertiesChanged` toutes les 30 s) coûte **zéro** requête radio supplémentaire. **Recommandation : UPower en priorité, sysfs en repli** si UPower est absent.

---

## 4. Code d'exemple (parties critiques)

### 4.1 Résoudre les nœuds d'un clavier depuis son MAC (sysfs, sans `unsafe`)

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Family { Bcm2042, MagicKeyboard, Unknown }

pub struct KbNodes {
    pub mac: String,            // "04:DB:56:CA:42:EE"
    pub vid: u32, pub pid: u32,
    pub family: Family,
    pub hidraw: Option<PathBuf>,       // /dev/hidrawN
    pub power_supply: Option<PathBuf>, // /sys/class/power_supply/hid-<mac>-battery[-<id>]
    pub capslock_led: Option<PathBuf>, // /sys/class/leds/inputN::capslock
}

pub fn family(vid: u32, pid: u32) -> Family {
    match (vid, pid) {
        (0x05ac, 0x022c..=0x022e) | (0x05ac, 0x0239..=0x023b) | (0x05ac, 0x0255..=0x0257) => Family::Bcm2042,
        (0x004c | 0x05ac, 0x0267 | 0x026c | 0x029a | 0x029c | 0x029f | 0x0320..=0x0322) => Family::MagicKeyboard,
        _ => Family::Unknown,
    }
}

/// HID device dirs live under …/uhid/0005:VVVV:PPPP.NNNN or …/hci0:…/0005:VVVV:PPPP.NNNN
pub fn resolve(mac: &str) -> Option<KbNodes> {
    for e in std::fs::read_dir("/sys/bus/hid/devices").ok()?.flatten() {
        let dev = e.path();
        let ue = std::fs::read_to_string(dev.join("uevent")).unwrap_or_default();
        let get = |k: &str| ue.lines().find_map(|l| l.strip_prefix(k)).map(str::trim);
        if !get("HID_UNIQ=").is_some_and(|u| u.eq_ignore_ascii_case(mac)) { continue; }
        let mut id = get("HID_ID=")?.split(':');
        let (bus, vid, pid) = (id.next()?, id.next()?, id.next()?);
        if bus != "0005" { continue; } // Bluetooth only
        let (vid, pid) = (u32::from_str_radix(vid, 16).ok()?, u32::from_str_radix(pid, 16).ok()?);
        let first = |sub: &str| std::fs::read_dir(dev.join(sub)).ok()
            .and_then(|mut d| d.find_map(|x| x.ok().map(|x| x.file_name())));
        let hidraw = first("hidraw").map(|n| Path::new("/dev").join(n));
        let power_supply = first("power_supply").map(|n| Path::new("/sys/class/power_supply").join(n));
        let capslock_led = first("input").and_then(|inp| {
            std::fs::read_dir(dev.join("input").join(&inp)).ok()?.flatten()
                .map(|x| x.path()).find(|p| p.to_string_lossy().ends_with("::capslock"))
        });
        return Some(KbNodes { mac: mac.to_uppercase(), vid, pid, family: family(vid, pid),
                              hidraw, power_supply, capslock_led });
    }
    None
}

pub fn kernel_capacity(n: &KbNodes) -> Option<u8> {
    let p = n.power_supply.as_ref()?;
    std::fs::read_to_string(p.join("capacity")).ok()?.trim().parse().ok()
}
```

### 4.2 Lecture Feature Report sûre (`OwnedFd`, ioctl typé, sans état global)

```rust
use std::os::fd::{AsRawFd, OwnedFd};
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;

nix::ioctl_readwrite_buf!(hidiocgfeature, b'H', 0x07, u8); // _IOWR('H', 7, len) — longueur prise du slice

pub struct Hidraw { fd: OwnedFd }

impl Hidraw {
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        let f = OpenOptions::new().read(true).write(true)
            .custom_flags(libc::O_CLOEXEC).open(path)?;
        Ok(Self { fd: f.into() })
    }
    /// Returns the report payload WITHOUT the leading report id.
    pub fn feature(&self, id: u8, max_len: usize) -> std::io::Result<Vec<u8>> {
        let mut buf = vec![0u8; max_len.clamp(2, 4096)]; // HID_MAX_BUFFER_SIZE = 16384
        buf[0] = id;
        let n = unsafe { hidiocgfeature(self.fd.as_raw_fd(), &mut buf) }
            .map_err(std::io::Error::from)? as usize;
        if n < 2 || buf[0] != id { return Err(std::io::ErrorKind::InvalidData.into()); }
        buf.truncate(n);
        Ok(buf.split_off(1))
    }
} // close() automatique au drop, même en cas de panique
```

`nix` 0.29 est déjà dans `Cargo.lock` (dépendance transitive) : il suffit de l'activer en dépendance directe avec la feature `ioctl`. `hidapi` serait possible (backend `linux-native`), mais il ajoute une bibliothèque C pour deux appels : ce n'est pas justifié.

### 4.3 udev : accès au siège actif, Apple Bluetooth uniquement

`udev/70-apple-kb-hidraw.rules` (remplace `99-apple-kb-hidraw.rules`) :

```udev
# Apple Bluetooth keyboards: hidraw access for the active seat user only (logind ACL).
# Must sort before 73-seat-late.rules, which applies the uaccess tag.
# 05AC = Apple USB vendor (A1255/A1314), 004C = Apple Bluetooth SIG vendor (Magic Keyboard 2015+).
ACTION=="remove", GOTO="apple_kb_end"
SUBSYSTEM=="hidraw", KERNELS=="0005:05AC:*|0005:004C:*", TAG+="uaccess"
LABEL="apple_kb_end"
```

Validé par `udevadm test --action=add --extra-rules-dir=… /sys/class/hidraw/hidraw7` : `CURRENT_TAGS=:seat:uaccess:` et `73-seat-late.rules:16 RUN{builtin}+="uaccess"`. Dans `post_install`, ajouter `udevadm trigger --subsystem-match=hidraw` après le `reload` : sans cela, le clavier déjà connecté garde `0600` jusqu'à sa prochaine reconnexion. Retirer de `apple-kb-monitor.install` la consigne `usermod -aG input`.

### 4.4 Battery Provider sans nom réservé, un objet par clavier, `fdo::ObjectManager`

```rust
use zbus::{blocking::Connection, fdo::ObjectManager, interface, zvariant::OwnedObjectPath};

const ROOT: &str = "/com/agenceapi/AppleKbMonitor";

struct Bat { device: OwnedObjectPath, pct: u8 }

#[interface(name = "org.bluez.BatteryProvider1")]
impl Bat {
    #[zbus(property)] fn percentage(&self) -> u8 { self.pct }
    #[zbus(property)] fn device(&self) -> OwnedObjectPath { self.device.clone() }
    #[zbus(property)] fn source(&self) -> &str { "kernel power_supply" }
}

pub struct Provider { conn: Connection }

impl Provider {
    /// No well-known name: BlueZ tracks the caller's unique name, and the
    /// system bus denies `own` to unprivileged users anyway.
    pub fn new(adapter: &str /* "/org/bluez/hci0" from GetManagedObjects */) -> zbus::Result<Self> {
        let conn = zbus::blocking::connection::Builder::system()?
            .serve_at(ROOT, ObjectManager)?          // InterfacesAdded/Removed handled by zbus
            .build()?;
        conn.call_method(Some("org.bluez"), adapter, Some("org.bluez.BatteryProviderManager1"),
                         "RegisterBatteryProvider", &(zbus::zvariant::ObjectPath::try_from(ROOT)?,))?;
        Ok(Self { conn })
    }
    fn child(mac: &str) -> String { format!("{ROOT}/dev_{}", mac.replace(':', "_")) }

    /// Called on Connected=true and on each kernel capacity change.
    pub fn set(&self, mac: &str, device: OwnedObjectPath, pct: u8) -> zbus::Result<()> {
        let path = Self::child(mac);
        let srv = self.conn.object_server();
        match srv.interface::<_, Bat>(path.as_str()) {
            Ok(iref) => {
                let mut b = iref.get_mut();
                if b.pct != pct { b.pct = pct.min(100);
                    zbus::block_on(b.percentage_changed(iref.signal_context()))?; }
                Ok(())
            }
            Err(_) => srv.at(path, Bat { device, pct: pct.min(100) }).map(|_| ()),
        }
    }
    /// Called on Connected=false: BlueZ drops Battery1 instead of showing a frozen value.
    pub fn remove(&self, mac: &str) { let _ = self.conn.object_server().remove::<Bat, _>(Self::child(mac)); }
}
```

À la réapparition de `org.bluez` (`NameOwnerChanged`), refaire `RegisterBatteryProvider`, comme le fait déjà la boucle actuelle. Prévoir aussi un fichier de politique `dbus/` minimal, ou aucun : avec cette conception, aucune règle `own` n'est nécessaire.

### 4.5 Connexion et reconnexion par signaux BlueZ (aucun polling)

```rust
use zbus::blocking::{Connection, fdo::ObjectManagerProxy, fdo::PropertiesProxy};

fn watch_connections(tx: std::sync::mpsc::Sender<KbEvent>) -> zbus::Result<()> {
    let conn = Connection::system()?;
    // Initial state: every Device1 whose Modalias is an Apple keyboard.
    let om = ObjectManagerProxy::builder(&conn).destination("org.bluez")?.path("/")?.build()?;
    for (path, ifaces) in om.get_managed_objects()? {
        if let Some(d) = ifaces.get("org.bluez.Device1") {
            // Modalias "usb:v05ACp0256d0050" / "bluetooth:v004Cp029Cd…"; Address; Connected
            /* … filtre famille, tx.send(KbEvent::Known{path, mac, connected}) … */
        }
    }
    // Then: PropertiesChanged on org.bluez.Device1 (match rule on sender org.bluez,
    // interface org.freedesktop.DBus.Properties, arg0=org.bluez.Device1).
    let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal)
        .sender("org.bluez")?.interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?.add_arg("org.bluez.Device1")?.build();
    for msg in zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(64))? {
        /* Connected=true  → resolve() avec 10 tentatives × 300 ms (le nœud hidraw/uhid
                              apparaît quelques centaines de ms après), lecture statique, provider.set()
           Connected=false → fermer le Hidraw, provider.remove(), UI « endormi » (pas « absent ») */
        let _ = (&msg, &tx);
    }
    Ok(())
}
```

Le wake-monitor devient une boucle `poll()` sur le `Hidraw` **possédé par l'acteur**. On abandonne le second `open()` concurrent et le `find_apple_hidraw()` à 5 s. Un report `0x13` déclenche une relecture immédiate de la batterie.

### 4.6 LED à travers keyd

```rust
/// keyd grabs the physical keyboard (EVIOCGRAB): EV_LED written to the Apple evdev is
/// dropped by input_inject_event(). keyd forwards EV_LED received on its *virtual*
/// keyboard to every grabbed device (daemon.c: "Propagate LED events").
fn led_target() -> Option<std::path::PathBuf> {
    keyd_virtual_keyboard()                    // EVIOCGNAME == "keyd virtual keyboard"
        .or_else(apple_evdev_if_not_grabbed)   // without keyd: direct path
}
```

Lire l'état d'origine depuis `KbNodes::capslock_led` (la LED du clavier Apple) au lieu de la première LED `*capslock*` trouvée dans le système. L'écriture sur le clavier virtuel keyd nécessite un accès en écriture à cet evdev, donc la même ACL `uaccess` (règle sur `ATTRS{name}=="keyd virtual keyboard"`). Sinon, la fonction se dégrade en notification seule.

### 4.7 Touches F1/F2 sans lire evdev

Les touches F1/F2 arrivent déjà dans KWin sous la forme `KEY_BRIGHTNESSDOWN/UP`, car keyd les laisse passer. Il suffit d'enregistrer deux actions KGlobalAccel (`org.kde.kglobalaccel`, composant `apihub-app`), ou de réutiliser `kde/shortcuts/apple-brightness-{up,down}.desktop` pointant vers une IPC de l'app (D-Bus de session `com.agenceapi.ApiHub.Brightness.Step(i)`). On supprime ainsi le dernier besoin du groupe `input`. Attention : PowerDevil peut lui aussi capter ces touches. Il faut alors désactiver ses raccourcis par défaut, sinon une pression déclenche deux actions. Ce dernier point touche au DDC : à trancher hors de cette revue.

### 4.8 RSSI : helper privilégié minimal

Installer `rssi-helper` dans `/usr/lib/apple-kb-monitor/` avec `setcap cap_net_admin+ep` (dans `post_install`, car `pacman` ne conserve pas les capacités du paquet), en `0750 root:<groupe>` ou exécutable par tous puisque la commande est en lecture seule. L'app l'appelle avec un timeout de 1 s. Ajouter un argument d'index d'adaptateur, aujourd'hui fixé à `0`. **Ne jamais mettre `cap_net_admin` sur `apihub-app`** : un binaire GUI à capacités passe en mode `AT_SECURE`, ce qui ignore `LD_LIBRARY_PATH` et certaines variables GTK/Qt, et lui donnerait la main sur tout l'adaptateur.

---

## 5. Plan d'implémentation chiffré (ordre imposé)

| # | Chantier | Contenu | Effort | Jalon | Dépend de |
|---|---|---|---|---|---|
| 1 | Permissions | Règle `70-…` `uaccess` (§4.3) + `udevadm trigger`. Suppression de `usermod -aG input` pour le hidraw. Politique D-Bus du dépôt alignée (plus de `own` requis). `/etc/dbus-1/system.d/com.agenceapi.AppleKbMonitor.conf` hors paquet à supprimer du poste. | 0,5 j | v3.1 | — |
| 2 | Modèles | Table PID corrigée d'après `hid-ids.h` + vendor `0x004C` + `Family` (§4.1), en Rust **et** en Python. keyd `[ids]` étendu. Tests unitaires sur les uevents réels (A1314 relevé ici, Magic Keyboard synthétique). | 0,5 j | v3.1 | — |
| 3 | Source batterie | Pourcentage = UPower (D-Bus) ou `power_supply/capacity`. `0xEA`/`0xF5` deviennent des détails. Le Python suit le nouveau nommage `hid-<mac>-battery[-N]` (glob). Alertes et historique fondés sur la valeur noyau, plus la tension quand elle est disponible. | 1 j | v3.1 | 2 |
| 4 | Acteur clavier | Thread dédié, sorti de la boucle DDC. Signaux BlueZ (§4.5). `Hidraw`/`OwnedFd` (§4.2). Lecture statique une fois par connexion, dynamique toutes les 15 min, relecture sur `0x13`. Multi-clavier, adaptateur dynamique. Wake-monitor intégré, lancé même si le clavier est absent au démarrage. | 2,5 j | v3.1 | 1, 2, 3 |
| 5 | Provider BlueZ | Refonte §4.4 : sans nom, `fdo::ObjectManager`, un objet par clavier connecté, retrait à la déconnexion, valeur noyau. Suppression du provider Python ou option exclusive. Test d'intégration `#[ignore]` contre BlueZ. | 1 j | v3.1 | 3, 4 |
| 6 | LED + touches | LED via le clavier virtuel keyd (§4.6), LED ciblée sur le clavier Apple. F1/F2 via KGlobalAccel (§4.7), sans evdev. Suppression de `modprobe/hid_apple.conf`. | 1 j | v3.2 | 1, 4 |
| 7 | RSSI | Helper `cap_net_admin` empaqueté (§4.8), index d'adaptateur, cadence de 60 s seulement quand l'UI est visible. Sinon, retrait de la fonction et de l'entité MQTT. | 0,5 j | v3.2 | 4 |
| | **Total** | | **7 j** | | |

Critères de sortie par chantier, mesurables sans intervention manuelle :
- (1) `getfacl /dev/hidrawN` montre `user:<utilisateur>:rw-` ; `id` ne contient plus `input`.
- (3) L'UI, `upower -i` et l'applet Bluetooth affichent **le même** pourcentage.
- (4) Avec le clavier éteint, aucun `ioctl` n'est émis (`strace -e ioctl -p` muet), et la reconnexion est vue en moins de 2 s après `Connected=true`.
- (5) Après `systemctl restart bluetooth`, `Battery1` réapparaît en moins de 10 s ; à la déconnexion, `Battery1` disparaît.
- (6) Sous keyd, la LED CapsLock clignote réellement (vérification visuelle, une fois).
- (7) `rssi-helper <MAC>` renvoie une valeur différente de `null` en utilisateur.

---

## 6. Risques

| Risque | Probabilité | Impact | Parade |
|---|---|---|---|
| Les reports non déclarés (`0xEA`, `0xF5`…) ne répondent pas sur un autre firmware ou une autre puce | Moyenne (certaine pour les Magic Keyboard) | Diagnostic absent | Gating par famille (chantier 2). Le pourcentage ne dépend plus d'eux (chantier 3). |
| Clavier endormi ou hors de portée pendant une lecture | Fréquente | Jusqu'à 5 s bloquées par report (uhid) | Aucune lecture si `Connected=false` ; acteur isolé de l'UI et du DDC (chantier 4). |
| Nommage `power_supply` qui change selon le noyau (`-battery` puis `-battery-<id>`) | Avérée | Chemin introuvable | Résolution par le parent HID (`<hid>/power_supply/*`), jamais par un nom construit (§4.1). |
| `UserspaceHID=persist` choisi par un utilisateur | Faible | Le nœud hidraw survit à la déconnexion et les ioctl attendent 3 à 5 s | Ce cas est couvert par la règle « BlueZ dit connecté, sinon pas d'ioctl ». |
| `uaccess` sans session graphique (démon système) | Faible | Pas d'accès | Le démon Python doit tourner en service **utilisateur**, ce qui est déjà le cas. Documenter. |
| Double provider (Python + Rust) | Avérée si les deux tournent | Valeur du premier lancé | Un seul provider (chantier 5). |
| Fichiers modifiés sur le poste mais absents du dépôt (`/etc/dbus-1/…conf`) | Avérée | Comportement non reproductible depuis le paquet | Chantier 1 + issue #14 (packaging). |
| `post_install` qui écrase `DeviceItem.qml` de Plasma (fichier d'un autre paquet) | Avérée | Écrasé à chaque mise à jour de Plasma, conflit `pacman` possible | Hors chantier clavier strict ; à traiter dans #14 (passer par un plasmoïde propre, déjà présent). |
| Test live sur le bus système lancé par `cargo test` | Faible | Provider fantôme `AA:BB:…` enregistré dans BlueZ | Tests live toujours `#[ignore]` et lancés à la demande. |

---

## 7. Ce qui reste juste et ne doit pas être réécrit

- La constante et l'usage de `HIDIOCGFEATURE`, le décodage ADC 10 bits, la calibration validée (`calibration_valid`) et l'interpolation bornée, avec leurs tests.
- `rssi.rs` : construction MGMT, appariement de la réponse, rejet de `127`, MAC stricte.
- `apple_model_from_uevent` : correspondance exacte sur `HID_ID`. Seuls la table et le vendor sont à corriger.
- La boucle `poll()` du wake-monitor, qui gère déjà `POLLHUP/POLLERR`. Elle sera seulement déplacée dans l'acteur.
- Le choix de `hidraw` plutôt que libusb : c'est le seul chemin pour un périphérique Bluetooth.
