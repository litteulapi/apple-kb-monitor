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
