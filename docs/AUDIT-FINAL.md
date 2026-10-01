# Audit final adversarial — code ajouté le 2026-10-01 (617c3db..8cb044e)

Auditeur indépendant, avec outils, branche `audit/final` (worktree, aucun code de
production modifié ; seuls des fichiers NOUVEAUX : ce document et trois fichiers de
tests de preuve). Paquet constaté sur le poste : **3.1.0-16** (le brief disait -15).

Périmètre : `akm-hid-control` (+ unités système), `akm-keymap-helper` (+ hwdb),
`WriteDoor`, API D-Bus du démon, `akmctl repair/forget/rename`, module KCM,
notifications KDE, scriptlet `.install` / fichiers du paquet.

Méthode : lecture du diff, 752 tests relancés dans un dossier de build propre
(`CARGO_TARGET_DIR` dédié, supprimé en fin d'audit), tests de preuve nouveaux sur
répertoires temporaires / fonctions pures (aucune écriture HID, aucun bus réel,
aucune unité touchée), `cargo clippy -W clippy::pedantic`, `cargo audit`,
`cargo geiger`, fuzz déterministe des parseurs nouveaux, `cargo mutants` sur
`hidctl.rs`, `breaker_state.rs`, `lib.rs` (helper), `keymap.rs`, `devname.rs`,
`hidraw.rs` (akm-core), casse volontaire sur copie de l'arbre pour éprouver
`scripts/ci-local.sh`. Rien n'a été poussé.

Fichiers de preuve (tous verts = défaut reproduit) :

- `apihub-app/crates/akm-helper/tests/audit_final_breaker_spoof.rs` (D1, D2, D3)
- `apihub-app/apple-kb-monitord/tests/audit_final_notify_markup.rs` (D4)
- `apihub-app/crates/akm-helper/tests/audit_final_parsers_fuzz.rs` (fuzz, R1)
- Toml.js : vérifié hors arbre avec `node` (reproduction donnée dans l'issue D5)

## 1. Défauts confirmés

### D1 (#256) — `breaker.state` : un `written_unix` futur n'est jamais périmé → blocage permanent (sécurité, moyen)

`breaker_state::BreakerState::age()` fait `now.saturating_sub(written_unix)` : un
état daté du futur a un âge de 0 pour toujours, donc `is_stale()` est toujours faux.
Le parseur accepte 12 chiffres (`999999999999` = an 33658). Conséquences,
prouvées par `audit_final_breaker_spoof.rs` :

- `open=1` + `written_unix` futur ⇒ `Verdict::Refuse(Open)` **même si aucun démon
  ne tourne** (`verdict()` ne consulte `daemon_alive` que pour un état périmé).
  `akm-hid-control` (root) n'envoie plus jamais SUSPEND/EXIT_SUSPEND au clavier,
  et `WriteDoor` (`akmctl repair`/`rename --write-device-name`) refuse toute
  écriture, jusqu'à suppression manuelle du fichier. Un démon qui plante avec une
  horloge en avance (NTP non encore synchronisé au boot, puis correction) laisse ce
  fichier derrière lui.
- `hidctl::published_breakers()` parcourt **tous** les `/run/user/<uid>` et
  `combine()` retient le premier refus : un autre compte local, propriétaire de son
  `/run/user/<uid>/apple-kb-monitor/breaker.state` (tous les contrôles de
  `read_user_file` sont satisfaits : propriétaire = uid du chemin, 0644, 1 lien),
  bloque l'émission vers le clavier d'un **autre** utilisateur. La MAC visée est
  lisible dans `/sys/bus/hid/devices/*/uevent` (`HID_UNIQ`, monde-lisible).
- `daemon_alive_in()` ne vérifie que `comm == "apple-kb-monito"` et l'uid :
  `prctl(PR_SET_NAME)` est libre, n'importe quel processus de l'utilisateur
  « est » le démon (test D3). Cela rend aussi `Refuse::Unreadable`/`Stale`
  déclenchables à volonté (fichier malformé + processus renommé).

Correctif suggéré : refuser (ou traiter comme périmé) tout `written_unix > now + 5 s`
dans `verdict()` ; borner la lecture root au seul uid de la session active
(`loginctl`/`sd_seat_get_active`) ou au propriétaire du périphérique (`HID_UNIQ`
↔ uid de l'ACL `uaccess` du nœud hidraw) ; vérifier la vie du démon par
`/proc/<pid>/exe` = `/usr/bin/apple-kb-monitord` (ou par le nom D-Bus) plutôt que
par `comm`.

### D4 (#257) — Notifications : le nom du clavier n'est pas échappé (sécurité, faible)

`notify::removed_notification(name)` (et les textes de `repair::notice_text` qui
incluent le nom) formatent le nom BlueZ dans le corps de la notification sans
`escape_markup` (le tray, lui, échappe : `tray/sni.rs`). Le corps d'une
notification Plasma est interprété comme balisage (liens, `<img>`). Le nom vient de
`Device1.Alias`/`Name` : **toute application de la session** peut le fixer par
`SetAlias` (`alias::validate` n'interdit que les caractères de contrôle/invisibles,
64 caractères suffisent) ; un appareil distant peut aussi annoncer un nom forgé.
Preuve : `audit_final_notify_markup.rs`.

Correctif : passer le nom par `escape_markup` dans `notify.rs` (ou un helper commun
`akm_core::text::escape`), et un test de non-régression sur chaque constructeur de
notification qui embarque un nom.

### D5 (#258) — KCM `Toml.js` : écriture produisant un `config.toml` invalide (bug, faible-moyen)

`Toml.set()` ne retrouve pas une clé existante et en **ajoute une seconde** dans ces
cas (vérifié avec `node`, sorties rejetées par `tomllib`) :

- fichier à fins de ligne CRLF : la regex `(.*)$` ne traverse pas `\r` ⇒
  `low = 15\r\nlow = 5` (clé dupliquée) ;
- clé entre guillemets `"low" = 15` ⇒ dupliquée ;
- table inline `alerts = { low = 15 }` ⇒ `[alerts]` déclarée deux fois ;
- `[[alerts]]` ⇒ idem.

En plus, le remplacement d'un tableau multi-ligne perd le commentaire de fin
(`] # c`). Le parseur maison du démon (`config.rs`) est tolérant, mais tout autre
lecteur TOML (python, `toml` Rust, éditeurs) refuse alors le fichier, et un
futur passage du démon à un parseur strict casserait silencieusement la config
(retour aux valeurs par défaut avec un simple avertissement).

Correctif : normaliser `\r\n` → `\n` dans `set()`/`parse()` (ou autoriser `\r?$`),
détecter clé citée / table inline / `[[…]]` et refuser l'enregistrement avec un
message (« éditez le fichier à la main »), test QML (`qmltestrunner`) des cas.

### D6 (#259) — `scripts/ci-local.sh` ne linte pas le QML du KCM (tests, faible)

L'étape `qml` ne parcourt que `plasma/…/*.qml` ; `kcm/lint/qmllint.sh` (qui, lui,
détecte une erreur de syntaxe injectée dans `kcm/ui/StatePage.qml`) n'est pas dans
la chaîne. Preuve : copie de l'arbre + `property int broken:` ajouté à
`StatePage.qml` ⇒ `ci-local.sh --only qml,shell,security,units` = **RESULT: OK**.
Le C++ du KCM n'est compilé que par l'étape `package` (makepkg), absente de
`--fast`. Correctif : ajouter `kcm/lint/qmllint.sh` à l'étape `qml` (ou une étape
`kcm`), et un `cmake --build` du KCM en étape `fast` si cmake/ECM sont présents.

### D7 (#260) — `.install` : double activation des unités et liens pendants après désinstallation (packaging, faible)

Le paquet livre déjà les liens `*.target.wants/` sous `/usr/lib/systemd` (PKGBUILD,
commentaire « enabled through the .wants symlinks of the package »), **et**
`_akm_enable` exécute `systemctl enable` / `systemctl --global enable`, qui crée
les mêmes liens sous `/etc/systemd/{system,user}` (constaté sur le poste :
`/etc/systemd/system/sleep.target.wants/apple-kb-monitor-suspend.service` etc.).
`post_remove` ne désactive rien : après `pacman -R`, les liens de `/etc` pointent
vers des unités disparues. Correctif : supprimer `_akm_enable` (les liens du paquet
suffisent et sont retirés avec lui), ou ajouter `systemctl disable` /
`--global disable` dans `pre_remove`.

Constat annexe (hors paquet) : `/usr/lib/apple-kb-monitor/mqtt-bridge.py` et
`mqtt-bridge.py.bak-20260821-100929` (exécutables root, 21/08) n'appartiennent à
aucun paquet : restes d'une version antérieure à nettoyer (`.bak` en répertoire
d'exécutables, cf. règle « jamais de .bak »).

## 2. Remarques sans issue (défaut non confirmé ou choix de conception à documenter)

- **R1 — hwdb : cibles `power`, `sleep`, `wakeup`, `sysrq`, `macro` acceptées**
  (`audit_final_parsers_fuzz::hwdb_whitelist_accepts_power_and_sysrq_as_targets`).
  Le fichier est système (tous les comptes, écran de connexion compris) ; une
  remap `Eject → power` déclenche `HandlePowerKey=poweroff` de logind. Protégé par
  `auth_admin` : pas une élévation, mais la boîte polkit ne montre pas le mapping
  demandé, et `Keymap.SetKey` (D-Bus, sans contrôle d'appelant) permet à une appli
  de la session de préparer le mapping avant que l'utilisateur clique « Appliquer ».
  Suggestion : retirer de la liste blanche des cibles les codes système
  (`KEY_POWER*`, `KEY_SLEEP`, `KEY_WAKEUP`, `KEY_SYSRQ`, `KEY_MACRO*`, `KEY_FN*`).
- **R2 — D-Bus sans contrôle d'appelant** : `NotifyShutdown` (une écriture HID
  `0x40`, une fois par run, si `[apple] will_shutdown` — vrai par défaut — et
  clavier connecté), `ExpectDisconnect`, `Refresh` (borné par
  `FORCE_REFRESH_FLOOR`), `Keymap.*`, `Tray.ClaimTrayFor` (TTL, par expéditeur),
  `SetAlias`. Même uid = même domaine de confiance sous Linux ; seul `SetFnMode`
  vérifie l'uid (#203). À documenter dans l'API ; `NotifyShutdown` pourrait exiger
  que l'appelant soit `akmctl` lancé par l'unité (`PrepareForShutdown` suffit).
- **R3 — `udevadm trigger --subsystem-match=input --action=change`** à chaque
  install/remove/rollback de hwdb : événement `change` sur **tous** les
  périphériques d'entrée (ré-application des règles, hwdb, ACL `uaccess`). Sans
  effet observé sur les autres claviers (le hwdb ne matche que `b0005v05AC`), mais
  `--sysname-match=event*` + `--attr-match` sur le modalias Apple serait plus
  ciblé.
- **R4 — Unités système** : durcissement cohérent (`CapabilityBoundingSet`
  minimal, `NoNewPrivileges`, `ProtectSystem=strict`, filtre seccomp avec
  `pidfd_getfd` ajouté). `TimeoutStartSec=2s`/`3s` et `ExecStart=-` bornent le
  retard de veille à 2 s ; `DRAIN_WAIT` 1 s. Avec `kernel.yama.ptrace_scope=3`
  la fonction est muette sans message explicite (`pidfd_getfd` ⇒ EPERM : « no
  candidate »). `RestrictAddressFamilies=AF_UNIX` n'empêche pas `send(2)` sur le
  socket emprunté (filtre sur `socket(2)` seulement) : conforme au commentaire.
- **R5 — `cargo audit`** : 4 vulnérabilités, toutes via `eframe 0.29` (déjà #224)
  + `rand 0.8.5` RUSTSEC-2026-0097 (ignoré dans `deny.toml` avec motif). Aucune ne
  touche les trois programmes root (`akm-helper` : `libc` seul, `cargo geiger`
  = 0 `unsafe` dans les dépendances hors `libc`).

## 3. Éprouvé sans défaut

- `akm-hid-control` : octet sur le fil = `HidControl::byte()` uniquement (constante),
  `check_byte` balaye 256 valeurs ; `select_control` exige exactement un socket
  L2CAP SEQPACKET connecté, PSM pair 0x0011, local 0x0011/0 ; `pidfd_open` +
  `exe` + `Uid` + `pidfd_send_signal(0)` après le scan (un `bluetoothd` redémarré
  ⇒ `pid exited during the scan`, rien d'écrit ; un pid réutilisé par un processus
  non root ⇒ `WrongUid`). Aucun `setsockopt`/`fcntl` sur le socket emprunté.
  `--mac` strictement validée et exigée dans la table des modèles connectés.
  `hid-suspend.conf` : O_NOFOLLOW, root, non inscriptible, ≤ 4 KiB, parseur
  strict (fuzz 200 000 entrées sans panique), défaut OFF si invalide.
- `akm-keymap-helper` : source à chemin fixe `/run/user/$PKEXEC_UID/…`
  (`PKEXEC_UID` chiffres seuls), `read_user_file` (O_NOFOLLOW, régulier,
  propriétaire, 1 lien, non inscriptible groupe/monde, ≤ 16 KiB), `parse_hwdb`
  ASCII seul, en-tête obligatoire, lignes `evdev:input:b0005v05ACpPPPP*` des 9 PID
  ALU seulement, `KEYBOARD_KEY_<sc>=<nom>` avec scancode de la table et nom du
  noyau, ≤ 512 lignes, **re-rendu canonique** (fuzz : aucune ligne hors liste
  blanche ne survit, relecture identique). Destination : symlink refusé, temp
  `O_EXCL|O_NOFOLLOW` 0600 → 0644 root:root, fsync, rename ; sauvegarde
  `.akm-bak` (ignorée par systemd-hwdb). `remove` écrit d'abord les valeurs par
  défaut (EVIOCSKEYCODE survit au fichier).
- `WriteDoor` / `hid_write_feature` : deux ioctl à taille encodée (1 et 65 octets),
  `check_write_op` (registre) + longueur exacte + `WriteSession` une fois ; aucun
  autre `ioctl` d'écriture dans l'arbre (test `registry.rs` compte les
  occurrences) ; `hid_read_input` limité à `0x30`. Verrou inter-processus +
  espacement 1 s + disjoncteur local + disjoncteur publié du démon.
- `akmctl repair/forget/rename` : confirmation tapée exacte (`OUBLIER`/`FORGET`,
  nom pour `rename`), `stdin`/`stdout` terminal exigés (le KCM lance `akmctl` avec
  stdin `/dev/null` : refus garanti), sauvegarde avant toute écriture, porte
  ouverte seulement après confirmation ; `--write-device-name` n'est atteignable
  que par la CLI (le KCM ne fait que copier la commande, `shellQuote` correct).
- KCM : programmes limités à `akmctl` et `systemctl --user {start,try-restart,is-active}
  apple-kb-monitord.service`, sans shell, arguments en liste ; `QSaveFile`
  (écriture atomique), bornes 256 KiB / 1 MiB ; appels D-Bus asynchrones avec
  délai borné [100 ms, 180 s].
- Notifications : `ActionInvoked` n'est accepté que de l'owner de
  `org.freedesktop.Notifications` (`from_server`) ; la commande « Réparer » est une
  liste fixe (`konsole --hold -e akmctl repair`), sans argument externe.
- Paquet installé : `/usr/lib/apple-kb-monitor/*` 0755 root:root sauf
  `rssi-helper` 0750 root:akm `cap_net_admin=ep` ; `hid-suspend.conf` 0644 root ;
  pas de hwdb installé ; polkit `auth_admin` sur les trois actions.
- `scripts/ci-local.sh` attrape une casse de la liste blanche (`0x15` accepté
  dans `HidControl::from_byte` ⇒ étape `test` FAIL).

## 4. Mutants (cargo mutants) — tests faibles

### akm-helper (`hidctl.rs`, `lib.rs`, `breaker_state.rs` par chemin)

293 mutants, **99 survivants** (34 %), 170 tués, 24 non viables, 4 min. Le module
`breaker_state.rs` inclus par `#[path]` n'a produit qu'un mutant : sa couverture
est mesurée côté akm-core (§4.2). Survivants qui comptent (sécurité) :

- `hidctl.rs:1008` `read_config` : **toutes** les mutations des contrôles
  « propriétaire root / non inscriptible groupe-monde / ≤ 4 KiB » survivent : la
  promesse « fail closed » de `hid-suspend.conf` n'était vérifiée par aucun test.
  Tests ajoutés : `audit_final_weak_tests.rs` (4 cas + symlink/absent).
- `hidctl.rs:792` `run_action` : `u == env.required_uid` → `true` survit : le
  refus `WrongUid` (bluetoothd qui ne tourne pas en root, ou pid réutilisé par un
  processus non root) n'est jamais exercé par `hid_control_fake_bluetoothd.rs`.
- `hidctl.rs:932/943` : la boucle d'attente de vidage (`DRAIN_WAIT`) n'est pas
  observée (le faux `drained()` répond toujours `Some(true)`).
- `lib.rs:101` `read_user_file` : la borne de taille (`> max`) survit ;
  `lib.rs:276` `parse_uid` : bornes de longueur ; `lib.rs:132` `atomic_write` :
  le test `fchown` (non exécutable hors root, attendu).
- `hidctl.rs:201-248` : `getsockopt_int`, `sockaddr_l2`, `inspect` réel : non
  testables sans socket Bluetooth (le test d'intégration passe par le trait) —
  attendu, à documenter.

### 4.2 akm-core (`breaker_state.rs`, `hidraw.rs`, hwdb de `keymap.rs`, trames de `devname.rs`)

Exécuté `--in-place` sur une copie de l'arbre (les `include_str!("../../../tests/…")`
de `decode.rs`/`led.rs` sortent de la racine du workspace : la copie par défaut
de cargo-mutants ne compile pas — à noter pour la QA), tests `--lib` seulement.
188 mutants, **43 survivants** (23 %), 128 tués, 16 non viables, 1 délai, 78 min.

- `hidraw.rs` (`WriteDoor`, `hid_write_feature`, portes 1/65 octets) : **aucun
  survivant** — la zone la plus sensible est bien verrouillée par les tests.
- `keymap.rs::parse_hwdb` : survivants qui comptent —
  `1016` l'en-tête obligatoire (`l == HWDB_HEADER` → `true` : un fichier sans
  en-tête serait accepté), `1005`/`1038` bornes 16 KiB et 512 lignes, `1056` le
  test de PID (`&&` → `||`), `1071` « property outside a record » (`||` → `&&`).
  Les tests d'intégration de `akm-helper` (`install_tests`) couvrent une partie
  de ces cas (« not ours »), mais pas le module lui-même ni les bornes.
  `read_installed` : non testé (4 survivants).
- `breaker_state.rs` : `parse` borne 12 chiffres (`105`), `read_own` bornes de
  taille et flags d'ouverture (`450-466`), `now_unix`, `Verdict::allows` → `false`
  survit (aucun test n'appelle `allows()` sur un `Allow`).
- `devname.rs::long_name_frame` : 10 survivants, tous sur les métadonnées `Field`
  (bornes `start/end` des annotations de preuve), pas sur les octets du rapport
  (`report` est comparé octet à octet à la fixture Lion).

Issue « tests faibles » : #261 (D8).
