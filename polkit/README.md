# polkit — bascule du mode Fn (F07 #88)

`com.agenceapi.AppleKbMonitor.set-fnmode` autorise `pkexec /usr/lib/apple-kb-monitor/akm-helper set-fnmode <0-3> [--persist]`.

- Défaut `allow_active=auth_admin` (sans `_keep`, #203) : le paramètre `hid_apple.fnmode` est **global à tous les claviers Apple** de la machine et `--persist` écrit dans `/etc/modprobe.d` ; une session locale active doit donc s'authentifier en administrateur à chaque changement. Pas de rétention : le démon (processus long, sujet vu par polkit) transmettrait sinon l'autorisation à toute application de la session via D-Bus. Sessions distantes/inactives refusées.
- Côté démon (`SetFnMode`) : `/usr/bin/pkexec` et le helper sont des constantes de compilation (#202), argv `set-fnmode <n>`, uid de l'appelant D-Bus = uid du démon, **un seul dialogue à la fois** (puis 5 s de pause), chaque demande journalisée. `SetSwapOptCmd`/`SetIsoLayout` n'existent plus (aucune action polkit).
- `49-apple-kb-monitor.rules.example` : règle optionnelle (groupe `wheel` sans mot de passe), non installée.
- Le helper ne prend aucune dépendance, n'accepte que `set-fnmode` + une valeur 0..=3 + `--persist`, n'ouvre que `/sys/module/hid_apple/parameters/fnmode` et `/etc/modprobe.d/hid_apple.conf` (seul le jeton `fnmode=` est réécrit, rename atomique).

## Securite du helper (#152)

`pkexec` ne modifie pas l'umask : le helper l'herite d'un appelant non privilegie. `akm-helper` force donc `umask 077` des son entree, cree le fichier temporaire en `O_EXCL|O_NOFOLLOW` mode 0600, puis impose explicitement `0644 root:root` (chmod/chown, independants de l'umask) avant le `rename` atomique ; un lien symbolique en destination est refuse. `/etc/modprobe.d/hid_apple.conf` ne peut ainsi jamais devenir inscriptible par un tiers (ce qui permettrait `install hid_apple <script>` execute en root). Tests : `cargo test -p akm-helper` (reproduit l'ancien defaut sous `umask 000`).

`akmctl` appelle `/usr/bin/pkexec` en chemin absolu et le helper au chemin compile `/usr/lib/apple-kb-monitor/akm-helper` : aucune variable d'environnement (l'ancienne `AKM_HELPER` n'existe plus hors build de test) ne choisit le programme lance via pkexec.

`--persist` ecrit la configuration meme si `hid_apple` n'est pas charge (message « applique au prochain chargement du module », code 0).

## Mapping des touches (#247)

`com.agenceapi.AppleKbMonitor.install-keymap` autorise `pkexec /usr/lib/apple-kb-monitor/akm-keymap-helper install|remove|rollback` (un programme distinct : pkexec choisit l'action d'après le chemin du programme). Aucun chemin ni aucune valeur en argument : le helper lit le seul fichier `/run/user/$PKEXEC_UID/apple-kb-monitor/keymap.hwdb` (régulier, `O_NOFOLLOW`, propriétaire = l'appelant, un seul lien, non inscriptible par le groupe/les autres, 16 Kio max), le valide ligne par ligne (en-tête apple-kb-monitor, lignes `evdev:input:b0005v05ACpPPPP*` des seuls PID aluminium sans fil, ` KEYBOARD_KEY_<usage>=<nom>` avec usage en liste blanche — page clavier 0x07, Éjection 0xc00b8, Fn 0xff0003 — et nom `KEY_*` du noyau ; tout le reste est refusé), le **réécrit sous forme canonique**, sauvegarde l'ancien en `90-apple-kb-monitor.hwdb.akm-bak`, écrit `/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb` en 0644 root:root (temporaire `O_EXCL|O_NOFOLLOW`, rename atomique), puis lance `/usr/bin/systemd-hwdb update` et `/usr/bin/udevadm trigger --settle --subsystem-match=input --action=change` (environnement vidé). `remove`/`rollback` réécrivent d'abord les codes par défaut des touches remappées (`EVIOCSKEYCODE` survit à la suppression du fichier jusqu'à la reconnexion du clavier). Ni saisie exclusive (grab) ni uinput.

`set-fnmode` couvre aussi `akm-helper set-params NAME=VALUE... [--persist]` : liste blanche des paramètres que `modinfo hid_apple` liste sur 7.x (`fnmode` 0-4, `iso_layout` -1..1, `swap_opt_cmd` 0-2, `swap_ctrl_cmd` 0-1, `swap_fn_leftctrl` 0-1 ; `rightalt_as_rightctrl` et `ejectcd_as_delete` n'existent pas dans ce pilote).

Les deux helpers restent sans dépendance (libc) : les listes blanches sont les fichiers source d'`akm-core` (`keymap.rs`, `keycodes.rs`) compilés par chemin.

## Correctifs du diagnostic (#106)

`com.agenceapi.AppleKbMonitor.doctor-fix` autorise `pkexec /usr/lib/apple-kb-monitor/akm-doctor-fix <bluez-conf|upower-conf|adapter-autosuspend>... [--restart] [--dry-run]`, lancé par `akmctl doctor --fix`. Un programme distinct, donc une action distincte ; `auth_admin` sans `_keep`, refusé hors session locale active.

Jusqu'à 3.1.0-26 inclus, cette action **n'existait pas** pour polkitd : un double tiret dans un commentaire XML du fichier le rendait invalide (`xmllint` : « Double hyphen within comment »), et polkitd ignore alors toute la suite du fichier ; `pkaction` ne listait que 4 actions et `pkexec akm-doctor-fix` retombait sur l'action générique `org.freedesktop.policykit.exec` (#292). Garde-fou : `tests/check-data-files.sh` (xmllint + DTD de polkit, analyse expat, actions analysées = actions déclarées) et le contrôle du paquet construit ; vérification sur un poste : `pkaction --action-id com.agenceapi.AppleKbMonitor.doctor-fix`. Aucun double tiret dans un commentaire de ce fichier.

- Liste fermée : trois identifiants et deux commutateurs, dans un ordre imposé. Aucun chemin, aucune clé, aucune valeur en argument ; ni variable d'environnement, ni entrée standard, ni shell.
- `bluez-conf` ne peut écrire que `FastConnectable = true` dans `[General]` et `ReconnectUUIDs` / `ReconnectAttempts` / `ReconnectIntervals` dans `[Policy]` de `/etc/bluetooth/main.conf` (retirées des autres sections, où bluetoothd les ignore). `upower-conf` : `NoPollBatteries = true` dans `/etc/UPower/UPower.conf`. `adapter-autosuspend` : `/etc/udev/rules.d/61-akm-bt-adapter-no-autosuspend.rules`, dont le contenu est compilé dans le programme, puis `udevadm control --reload` et `udevadm trigger` sur l'adaptateur `8087:0026`.
- Les autres lignes, les commentaires et l'ordre du fichier sont conservés. Réécriture atomique (`0644 root:root`, lien symbolique refusé), contenu précédent gardé dans `<nom>.akm-bak`, et seulement si le contenu change : lancer deux fois un correctif ne change rien la seconde fois, la sauvegarde garde l'original.
- `--restart` (`akmctl doctor --fix --restart-services`) lance `/usr/bin/systemctl restart bluetooth.service` ou `upower.service`, arguments fixes, environnement vide. Jamais fait sans ce commutateur : redémarrer bluetooth coupe tous les liens Bluetooth quelques secondes.
- `--dry-run` n'écrit rien et ne lance rien ; `akmctl` l'exécute sans `pkexec`, donc sans mot de passe (les fichiers lus sont lisibles par tous).
- Ce que l'assistant ne fait pas reste un conseil affiché : appairage, confiance BlueZ, espace disque, services de l'utilisateur.
