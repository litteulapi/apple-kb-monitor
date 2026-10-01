# Audit sécurité 2 : contre-audit indépendant

Date : 2026-10-01. Base : `main` @ `c54c502`. Branche du rapport : `audit/securite-2`.
Posture : adversariale. On suppose le code vulnérable et on cherche à le prouver. Le premier audit (#152, #154, #155) sert de point de départ : on vérifie que ses correctifs tiennent et on cherche ce qu'il a raté.

Règles suivies : aucune écriture dans `/etc` ou `/sys`, pas de `sudo`, aucun accès au matériel, paquet et service installés non touchés. Toutes les preuves tournent sur un bus de session privé (`dbus-run-session`), avec un faux `pkexec` et dans des répertoires temporaires.

## 1. Synthèse

| # | Gravité | Défaut | Issue |
|---|---|---|---|
| S2-01 | **Moyenne** | Démon : `pkexec` est résolu par `$PATH` et le programme lancé est choisi par `APPLE_KB_SETTINGS_HELPER`. Le correctif #154 ne couvre que `akmctl`. | #202 |
| S2-02 | **Moyenne** | API D-Bus `SetFnMode`/`SetSwapOptCmd`/`SetIsoLayout` : n'importe quelle application de la session ouvre autant de dialogues d'authentification administrateur qu'elle veut, sans limite ni file. Chaque appel échoue ensuite, car le démon envoie `set <nom> <v>` alors que le helper attend `set-fnmode`. | #203 |
| S2-03 | **Moyenne** (poste multi-utilisateur) | uaccess hidraw : un descripteur ouvert survit au changement de session. Un compte inactif peut donc lire les frappes de l'utilisateur actif. | #204 |
| S2-04 | Basse-moyenne | Widget Plasma : le nom (alias BlueZ) est rendu en texte riche. `<img src="http://…">` déclenche une requête réseau de plasmashell. L'alias est global à la machine, donc l'effet traverse les comptes. | #205 |
| S2-05 | Basse-moyenne | `Refresh()` (D-Bus) n'impose aucun espacement et contourne la période de 4 h (#177). Un appel par seconde produit une salve de GET_REPORT par seconde pendant la frappe, ce qui gèle ou coupe le clavier. | #206 |
| S2-06 | Basse | Renommage depuis le tray : l'alias est passé en argument à `kdialog`/`zenity` sans `--`. Avec l'alias `--version`, le clavier est renommé en silence (« kdialog 26.08.0 »). | #207 |
| S2-07 | Basse | `hid.lock` : sans `XDG_RUNTIME_DIR`, repli sur `/tmp/apple-kb-monitor/` (chemin partagé entre comptes), ouvert en `O_CREAT` sans `O_NOFOLLOW` ni contrôle de propriétaire. C'est une régression par rapport au correctif Python #49. | #208 |
| S2-08 | Basse | `rssi-helper` porte `cap_net_admin+ep` en mode 0755 : tout compte local peut sonder le RSSI (et donc la présence) de n'importe quel appareil BR/EDR connecté. | #209 |
| S2-09 | Basse | Résidu de #123 : le CLI Python livré (`/usr/bin/apple-kb-monitor --json/--status`) publie toujours `0x4C` (`identity_key`). Une charge 0x4C d'apparence réelle (adresse de l'hôte réel et 12 octets) se trouve dans `tests/test_apple_kb.py`, présent aussi sur le remote GitHub `origin`. | #210 |
| S2-10 | Basse | Unités systemd sans aucun durcissement (`systemd-analyze security` : 9,2 UNSAFE). Le compromis `NoNewPrivileges` est documenté de façon incomplète : `pkexec` (setuid) est lui aussi concerné. | #211 |
| S2-11 | Basse | `cargo audit` : quick-xml (RUSTSEC-2026-0194/0195), webbrowser (RUSTSEC-2026-0257), paste et ttf-parser non maintenus. Aucun n'est exploitable en l'état. | #212 |

Correctifs du premier audit :

* **#152 tient.** Les 11 tests du helper passent, dont la reproduction sous `umask 000`. Sous `umask 000` comme sous `umask 077`, le résultat est un fichier 0644 root:root. Une destination en lien symbolique est refusée, et un fichier temporaire en lien symbolique n'est pas suivi. Reste un défaut mineur, non exploitable : le nom temporaire est fixe (`hid_apple.conf.akm-tmp`). Deux exécutions concurrentes (root contre root) peuvent laisser un fichier vide pendant un court instant (§3.1).
* **#154 tient pour `akmctl`, pas pour le démon** (S2-01).
* **#155** : le risque est accepté et documenté pour « tout processus de l'utilisateur ». Le cas des autres comptes d'un même siège (S2-03) n'est pas couvert.

## 2. Modèle de menace retenu

* **A1, application de la session** (même uid, hors bac à sable). Elle peut déjà presque tout faire en tant qu'utilisateur. Seuls comptent donc les défauts qui franchissent une frontière : passage à root par polkit, autres comptes, matériel partagé, BlueZ (système), ou qui créent une confusion d'interface exploitable (dialogue d'authentification légitime).
* **A2, autre compte local** (autre utilisateur du siège, compte de service).
* **A3, appareil Bluetooth hostile** se présentant avec un VID/PID Apple après un appairage accepté par l'utilisateur. Il contrôle son nom, ses rapports HID et ses descripteurs.
* **Hors périmètre** : root, applications Flatpak (bus filtré par xdg-dbus-proxy, pas d'accès à `com.agenceapi.*` sans `--talk-name`).

## 3. Constats détaillés

### 3.1 akm-helper et polkit

Vérifié et conforme :

* Liste blanche stricte : `set-fnmode`, un seul chiffre ASCII entre 0 et 3, `--persist` optionnel.
* Aucune variable d'environnement n'est lue. `pkexec` nettoie l'environnement et le helper ne s'en sert pas.
* `umask 077` est appliqué dès l'entrée. Fichier temporaire ouvert en `O_EXCL|O_NOFOLLOW` 0600, puis `fchmod 0644` et `fchown 0:0`, puis `fsync`, puis `rename`, puis `fsync` du répertoire.
* La lecture du fichier existant se fait en `O_NOFOLLOW` et ne s'arrête qu'aux fichiers réguliers.
* `/etc/modprobe.d` appartient à root : aucune course TOCTOU n'est atteignable sans root.
* Politique polkit : `allow_any=no`, `allow_inactive=no`, `allow_active=auth_admin_keep`, `exec.path` épinglé. La règle `wheel` n'est fournie qu'en exemple.

Remarques sans issue :

* Nom temporaire fixe. Deux `akm-helper --persist` simultanés : B supprime le fichier temporaire de A, A renomme le fichier vide 0600 de B en `hid_apple.conf`, puis B le remplit. L'état final est correct (0644, contenu de B). Une fenêtre de quelques millisecondes laisse un fichier vide. Impact nul côté sécurité. Correctif : un nom unique (`mkstemp`) ou un `flock` sur le répertoire.
* `merge_fnmode` ne gère pas la continuation `\` de modprobe.d. C'est un défaut fonctionnel mineur, sans impact sécurité.

**S2-01 : le démon contourne le correctif #154.** Fichier `apple-kb-monitord/src/settings.rs:83-110` :

```rust
std::env::var_os(HELPER_ENV)            // "APPLE_KB_SETTINGS_HELPER"
...
Command::new("pkexec")                  // résolu par $PATH
    .arg(&helper).args(["set", p.name(), &v.to_string()])
```

* L'environnement du démon vient du gestionnaire systemd utilisateur. N'importe quel processus de la session peut le modifier avec `systemctl --user set-environment`, `~/.config/environment.d`, `PATH`, puis `systemctl --user restart apple-kb-monitord`.
* Conséquence : le dialogue polkit est ouvert par le démon légitime, mais pour un programme choisi par l'attaquant, ou par un faux `pkexec` placé dans `PATH` qui imite le dialogue et récupère le mot de passe administrateur. C'est exactement le scénario que #154 a fermé dans `akmctl`.
* Preuve PoC-A (`apple-kb-monitord/tests/audit_sec2_poc.rs`) : avec le vrai `HelperBackend`, un faux `pkexec` placé en tête de `PATH` reçoit `<programme choisi> set fnmode 2`.
* Correctif : `/usr/bin/pkexec` en chemin absolu, constante `HELPER_PATH` uniquement (variable réservée à `cfg(test)`, comme dans `akmctl`).

**S2-02 : dialogues d'authentification à volonté et API inopérante.** Fichiers `devices.rs:201-206`, `settings.rs:96-135`, `devices.rs:376` (`unblock` crée un thread par appel).

* Aucun contrôle de l'appelant, aucune file unique, aucun débit maximal. Chaque appel D-Bus lance un `pkexec`, et le délai d'attente est de 120 s.
* PoC-B : 12 appels `SetFnMode(2)` simultanés depuis un client quelconque produisent 12 `pkexec` actifs au bout de 1,5 s (`12/12`).
* polkit vérifie l'autorisation **avant** d'exécuter le programme. L'utilisateur voit donc un vrai dialogue « Authentication is required to change the Fn key mode », saisit son mot de passe administrateur, puis le helper refuse l'appel :

  ```
  $ akm-helper set fnmode 2      ->  akm-helper: unknown command "set"   (rc=64)
  $ akm-helper set swap_opt_cmd 1 -> rc=64
  ```

  La politique polkit ne déclare de toute façon que `set-fnmode`. `SetSwapOptCmd` et `SetIsoLayout` ne peuvent jamais fonctionner, et le commentaire cite une action `…configure` qui n'existe pas.
* Adjoint confus : une fois l'argv corrigé, c'est le démon (processus de longue durée) que polkit voit comme sujet, pas l'appelant D-Bus. Avec `auth_admin_keep`, après une authentification légitime depuis le tray, n'importe quelle application peut changer les paramètres de `hid_apple` sans nouveau dialogue pendant la durée de rétention. Avec la règle `wheel` (exemple), c'est permanent.
* Non vérifié : le service utilisateur n'est pas dans une session logind. Selon la version de polkit, `allow_active` peut ne pas s'appliquer, et l'appel est alors refusé d'office.
* Correctifs :
  1. aligner l'argv sur `set-fnmode <n>` et retirer les méthodes sans action polkit ;
  2. une seule demande à la fois (refuser `Busy` si une demande est en cours) et un débit maximal ;
  3. n'accepter les écritures que d'un appelant identifié, ou ne pas exposer ces méthodes sur le bus et ne garder que `akmctl`/le tray, qui appellent `pkexec` eux-mêmes ;
  4. envisager `auth_admin` (sans `keep`) quand le sujet est le démon.

### 3.2 API D-Bus de session

* `SetAlias` valide correctement : caractères de contrôle, bidi et largeur nulle refusés, 64 caractères et 248 octets au plus, MAC stricte, appareil Apple uniquement. En revanche, le **contenu** n'est pas neutralisé pour ses consommateurs (S2-04, S2-06). PoC-C : `SetAlias("--version")` et `SetAlias("<img src=\"http://127.0.0.1:9/x\">")` sont tous deux acceptés.
* `Reconnect` (Link) est limité à une tentative toutes les 20 s : conforme.
* **S2-05** : `Refresh()` appelle `Machine::force_refresh` (`akm-core/src/machine.rs:122`), qui remet `next_slow` à maintenant sans aucun plancher. PoC-D (`akm-core/tests/audit_sec2_poc.rs`) : 60 appels en 60 s produisent 60 lectures HID, contre 0 attendue (période de 4 h). La politique de lecture (`read_policy`) ne lit que si le clavier a servi dans les 60 dernières secondes : la salve tombe donc **pendant la frappe**, c'est-à-dire précisément la situation qui coupe la liaison selon #175/#177. Correctif : un plancher (par exemple 5 min) entre deux lectures forcées, appliqué dans `Machine`.
* Fuites de lecture, informatives : `Json`/`GetState` exposent la MAC du clavier et `bluetooth.paired_host_addr` (MAC de l'adaptateur de l'hôte). `History` renvoie tout l'historique. L'interface `Input` diffuse en temps réel (PropertiesChanged) `FnPressed`, `EjectPressed` et `MediaKeys`, ce qui donne l'horodatage d'appui de Fn et des touches média. Ce ne sont pas des codes de touches, mais c'est un canal temporel. Dans le modèle A1 (même uid), c'est acceptable. Il vaut mieux le documenter, ou ne publier `FnPressed`/`MediaKeys` qu'à la demande.
* Usurpation du nom bien connu : le bus de session n'a pas de politique de propriété, donc un processus A1 peut prendre `com.agenceapi.AppleKbMonitor1` avant le démon. Impact limité au modèle A1, sans issue.

### 3.3 udev, hidraw et écoute passive

* Règle `70-apple-kb-hidraw.rules` : valide (`udevadm verify` : OK), restreinte aux PID claviers en BT (bus 0005), Magic Mouse et Trackpad exclus. Le correctif #155 tient sur ce point.
* Aucune frappe stockée ni journalisée. Dans `akm-core/src/passive.rs` et `hidraw.rs`, chaque rapport ne met à jour qu'un horodatage (`note_input`). `decode` ne garde que 0x04, 0x05, 0x11, 0x12, 0x13 et 0x30, et aucune trace (`tracing`) ne contient de contenu. Dans le Python historique, les rapports inconnus (`raw: data.hex()`, donc les frappes 0x01) sont transmis au rappel, qui les ignore : rien n'est écrit. Conforme, avec une réserve : le code transporte les frappes jusqu'au rappel, alors qu'il serait plus sûr de les jeter dans `decode_input_report`.
* **S2-03** : l'ACL `uaccess` est recalculée au changement de session, mais **les descripteurs déjà ouverts restent valides**. logind ne révoque hidraw (`HIDIOCREVOKE`) que pour les périphériques obtenus par `TakeDevice` : les chaînes « Failed to revoke hidraw device » de `systemd-logind` (v261) appartiennent à ce seul chemin. Le démon, lui, ouvre `/dev/hidrawN` directement et le garde ouvert en permanence (écoute passive). Un utilisateur A qui ouvre le nœud (`cat /dev/hidrawN`, ou un démon modifié) puis passe la main à B continue de recevoir les rapports d'entrée de B, donc ses frappes et ses mots de passe.
  * Niveau de preuve : analyse du code et des binaires. Pas de démonstration en direct, faute de second compte et de matériel dans cet audit.
  * Correctifs : (a) documenter ce cas dans `udev/README.md` ; (b) le démon ferme le nœud quand sa session devient inactive (`org.freedesktop.login1.Session.Active`) ; (c) la variante durcie (groupe dédié et service système) ou un passage par `TakeDevice`, qui offre la révocation.

### 3.4 Fichiers

* `$XDG_STATE_HOME/apple-kb-monitor/history.jsonl` est créé avec l'umask par défaut (0644, répertoire 0755 via `StateDirectory=`). Il ne contient que des niveaux de batterie. Constat informatif : `UMask=0077` est conseillé.
* `history.jsonl` : lignes analysées par serde avec validation des plages, rotation bornée. Le fichier appartient à l'utilisateur, donc il n'y a pas d'injection inter-comptes.
* **S2-07** : `read_policy::lock_path()` utilise `XDG_RUNTIME_DIR`, sinon `std::env::temp_dir()`, c'est-à-dire `/tmp/apple-kb-monitor/hid.lock`. `try_lock` fait `create_dir_all` puis `open(create, write)`, sans `O_NOFOLLOW` ni contrôle de propriétaire. Un autre compte (A2) qui a créé `/tmp/apple-kb-monitor` peut :
  * poser un lien symbolique, ce qui fait créer un fichier vide au chemin de son choix avec les droits de la victime. `fs.protected_symlinks` ne protège pas ici : le répertoire appartient à l'attaquant et n'est pas sticky ;
  * ou garder le verrou, ce qui bloque toutes les lectures de batterie (déni de service).

  PoC-E (TMPDIR privé) : le lien est suivi et la cible est créée. Les cas concernés sont `akmctl` sous `sudo -u`, ssh sans `pam_systemd`, cron. Correctif : comme pour le Python #49, un nom par uid, un répertoire 0700 dont le propriétaire est vérifié, et `O_NOFOLLOW` ; ou renoncer à verrouiller quand `XDG_RUNTIME_DIR` est absent.
* Python historique : `ensure_state_dir` (0700, propriétaire vérifié) est conforme.

### 3.5 Unités systemd (S2-10)

`systemd-analyze security --offline=true --user` donne 9,2 UNSAFE pour les deux unités. Évaluation du compromis :

* Le commentaire de l'unité justifie l'absence de `NoNewPrivileges=` par la capacité de fichier de `rssi-helper`. Il oublie que `pkexec` (setuid) serait lui aussi neutralisé, ce qui casserait S2-01/S2-02.
* Dans une unité **utilisateur**, `ProtectSystem=`, `ProtectHome=` et `PrivateTmp=` imposent un espace de noms utilisateur (`PrivateUsers=` implicite). Une capacité de fichier y est alors accordée dans l'espace de noms fils, ce qui est inutile pour le contrôle `capable(CAP_NET_ADMIN)` du socket MGMT, et le bit setuid ne correspond plus à root. `SystemCallFilter=` et `RestrictAddressFamilies=` impliquent `NoNewPrivileges=`. **Le compromis actuel interdit donc tout durcissement significatif.**
* Recommandation : sortir les deux opérations privilégiées du démon. Le RSSI passerait par un petit service système activé par D-Bus (bus système, polkit `allow_active=yes`, MAC validée). fnmode resterait limité à `akmctl`/tray via `pkexec`, sans méthode D-Bus. Le démon pourrait alors recevoir `NoNewPrivileges=yes`, `UMask=0077`, `LockPersonality=yes`, `MemoryDenyWriteExecute=yes`, `RestrictRealtime=yes`, `SystemCallArchitectures=native`, `SystemCallFilter=@system-service`, `RestrictAddressFamilies=AF_UNIX AF_NETLINK AF_BLUETOOTH`, `ProtectSystem=strict` avec `ReadWritePaths=%S/apple-kb-monitor %t/apple-kb-monitor`, et `DeviceAllow` n'est pas applicable en unité utilisateur.

### 3.6 rssi-helper (S2-08)

* Entrées : MAC stricte (17 caractères, chiffres hexadécimaux, `:`), index ≤ 0xFFFE, sortie formatée par `printf` à partir d'entiers. Réponse MGMT appariée par opcode, longueur vérifiée (`n >= 19`), délai de 400 ms. La capacité de fichier entraîne `AT_SECURE`, donc `LD_PRELOAD` est ignoré. Conforme.
* En revanche, le binaire est installé en `0755` avec `cap_net_admin+ep` (`PKGBUILD`, `_akm_setcap`). **Tout compte local**, y compris les comptes de service et les autres utilisateurs, peut interroger le RSSI et la puissance d'émission de n'importe quelle connexion BR/EDR (téléphone, casque…), ce qui révèle présence et proximité. Le socket de contrôle de confiance reçoit aussi pendant 400 ms des événements MGMT (dont `NEW_LINK_KEY`), aujourd'hui jetés. La surface est minimale, mais elle est privilégiée.
* Correctif : `root:<groupe dédié> 0750`, ou un service système (§3.5). À défaut, vérifier que l'appelant possède une session active sur le siège.

### 3.7 Paquet (PKGBUILD et .install)

* Scriptlets root : `udevadm control/trigger` (sans danger), `keyd reload` uniquement si keyd est actif, `echo 1 > fnmode` à la première installation seulement, `setcap` sur un chemin fixe. Aucune entrée utilisateur, aucun fichier temporaire. Conforme.
* Aucun fichier inscriptible par tous dans le dépôt (`git ls-files -s` : uniquement 100644 et 100755). `install -Dm644` ou `-Dm755` partout.
* `backup=` protège `/etc/modprobe.d/hid_apple.conf` modifié par le helper. Conforme.
* La CI Gitea n'utilise aucune expression `${{ … }}` dans `run:` : pas d'injection depuis les PR.

### 3.8 Commandes externes

* `kdialog`/`zenity` (**S2-06**) : `rename_dialog_argv` place `current` (l'alias, ou le nom propre du clavier, que contrôle A3) en argument positionnel, après des options et sans `--`. QCommandLineParser et GOption analysent les options partout dans la ligne. Démonstration :

  ```
  $ kdialog --title T --inputbox P --version    ->  "kdialog 26.08.0", rc=0
  ```

  `ask_name` prend alors cette sortie pour la saisie de l'utilisateur et renomme le clavier sans qu'il ait rien tapé. D'autres options (`--getopenfilename`, `--textbox <fichier>`, `--geometry`…) détournent le dialogue. Il n'y a pas d'exécution de code. Correctif : insérer `--` avant les arguments positionnels, ou refuser un alias qui commence par `-` (validation côté `akm_core::alias`).
* `systemd-run`, `kcmshell6`, terminaux, `wl-copy`/`xclip` : argv fixes, aucune donnée externe, recherche dans `PATH` de l'utilisateur (modèle A1). Conforme.
* `akmctl` : `/usr/bin/pkexec` en chemin absolu, helper constant. #154 tient.
* `journalctl`, `bluetoothctl` (affichés et non exécutés dans `doctor`) : conformes.
* `bluetooth/akm-conf.py` (non installé, lancé à la main sous `sudo`) : il suit les liens dans `/etc/bluetooth`, répertoire qui appartient à root. Conforme.

### 3.9 Widget QML (S2-04)

* Le widget n'exécute plus de commande (#150 corrigé) : il ne fait que des appels D-Bus par le module `org.kde.plasma.workspace.dbus`.
* En revanche, `FullRepresentation.qml:30-32` affiche `root.kbName` (alias, sinon nom du clavier) dans un `Kirigami.Heading`, dont le `textFormat` par défaut est `AutoText`. `toolTipMainText` et l'infobulle SNI du démon (`tray/view.rs`, non échappée) reçoivent le même nom.
* PoC : un `Kirigami.Heading` dont le texte vaut `<img src="http://127.0.0.1:18457/alias-heading">Clavier`, rendu par `qml6` hors écran, provoque `HIT /alias-heading UA=Mozilla/5.0` sur le serveur local.
* L'alias BlueZ est stocké pour toute la machine (`/var/lib/bluetooth`). Un compte, ou un appareil hostile (A3) par son nom, peut donc faire émettre des requêtes réseau (balise de suivi, fuite d'IP) ou injecter du formatage trompeur dans la session d'un autre utilisateur.
* Correctif : `textFormat: Text.PlainText` sur tous les `Label`/`Heading` qui affichent des données (nom, modèle, firmware, `lastError`, `renameError`), et échappement HTML dans l'infobulle SNI.

### 3.10 Résidu #123 (S2-09)

* `apple-kb-monitor` (Python, installé dans `/usr/bin`) lit toujours 0x4C. Il publie `identity.key` dans `--json` (lignes 1308-1310) et affiche « Key (128-bit) » dans `--status` (lignes 1444-1448). `apihub-settings` appelle `apple-kb-monitor --json`.
* `tests/test_apple_kb.py:83` contient `4c030d7c5266946c` suivi de 12 octets de forte entropie. Les octets 2 à 7 correspondent à l'adresse réelle de l'adaptateur de l'hôte (`6c:94:66:52:7c:0d`, cf. `docs/HARDWARE-RAPPORTS-HID.md`). L'empreinte SHA-256 (20 octets : `4396240337911cdf`) diffère de la valeur actuelle mesurée (`f34626564fca9676`) : il s'agit probablement d'un appairage antérieur (commit `98f90c8`, 2026-04-01), donc périmé. Il est néanmoins présent dans l'historique git et sur le remote `origin` (GitHub).
* Correctif : masquer 0x4C dans le CLI Python comme dans le Rust, remplacer la valeur du test par une donnée synthétique et décider d'une purge de l'historique.

### 3.11 Dépendances (S2-11)

`cargo audit` (base du 2026-10-01) :

| Crate | Avis | Chemin | Exploitable ? |
|---|---|---|---|
| quick-xml 0.30.0 | RUSTSEC-2026-0194/0195 (déni de service) | eframe → accesskit_unix → atspi → zbus-lockstep → zbus_xml | Non : XML d'introspection a11y local |
| quick-xml 0.39.2 | idem | wayland-scanner (proc-macro, compilation) | Non : compilation uniquement |
| webbrowser 1.2.0 | RUSTSEC-2026-0257 (injection d'arguments via `BROWSER`) | eframe → egui-winit | Non : l'application n'ouvre aucune URL |
| paste 1.0.15, ttf-parser 0.25.1 | non maintenus | egui | — |

Correctif : monter eframe/egui (≥ webbrowser 1.2.2), et ajouter `cargo audit` à la CI.

## 4. Preuves reproductibles

Fichiers nouveaux, sans modification du code existant :

```
export CARGO_TARGET_DIR=<scratch>/target-sec
cd apihub-app
cargo test -p apple-kb-monitord --test audit_sec2_poc -- --nocapture   # PoC-A/B/C, bus privé
cargo test -p akm-core --test audit_sec2_poc -- --nocapture --test-threads=1   # PoC-D/E
cargo build -p akm-helper && target/debug/akm-helper set fnmode 2; echo $?   # 64, rien écrit
cargo test -p akm-helper                                             # #152 : 11 passed
QT_QPA_PLATFORM=offscreen timeout 5 kdialog --title T --inputbox P --version   # S2-06
```

Sorties obtenues :

```
POC-B pkexec simultanes apres 1,5 s : 12/12
POC-B erreur rendue au client : "org.freedesktop.DBus.Error.Failed: helper failed (exit status: 127)"
POC-C SetAlias("--version") -> ACCEPTE
POC-C SetAlias("<img src=\"http://127.0.0.1:9/x\">") -> ACCEPTE
POC-D lectures HID declenchees en 60 s par Refresh() : 60 (attendu sans Refresh : 0, periode 4 h)
POC-E lock_path() sans XDG_RUNTIME_DIR = <TMPDIR>/apple-kb-monitor/hid.lock ; cible du lien creee : true
QML : HIT /alias-heading UA=Mozilla/5.0
```

## 5. Ordre de correction conseillé

1. S2-01 et S2-02 : les deux touchent le chemin root/polkit. Corriger ensemble : chemin absolu, argv `set-fnmode`, une seule demande à la fois, ou retrait des méthodes D-Bus d'écriture.
2. S2-04 et S2-06 : neutralisation des noms. Une modification d'une ligne par consommateur, plus un refus du `-` initial dans `alias::validate`.
3. S2-05 : plancher entre deux lectures forcées.
4. S2-03 : documentation immédiate, puis fermeture du nœud en session inactive.
5. S2-07, S2-08, S2-09, S2-10, S2-11.
