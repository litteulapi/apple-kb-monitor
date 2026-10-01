# polkit — bascule du mode Fn (F07 #88)

`com.agenceapi.AppleKbMonitor.set-fnmode` autorise `pkexec /usr/lib/apple-kb-monitor/akm-helper set-fnmode <0-3> [--persist]`.

- Défaut `allow_active=auth_admin_keep` : le paramètre `hid_apple.fnmode` est **global à tous les claviers Apple** de la machine et `--persist` écrit dans `/etc/modprobe.d` ; une session locale active doit donc s'authentifier en administrateur (autorisation mémorisée quelques minutes). Sessions distantes/inactives refusées.
- `49-apple-kb-monitor.rules.example` : règle optionnelle (groupe `wheel` sans mot de passe), non installée.
- Le helper ne prend aucune dépendance, n'accepte que `set-fnmode` + une valeur 0..=3 + `--persist`, n'ouvre que `/sys/module/hid_apple/parameters/fnmode` et `/etc/modprobe.d/hid_apple.conf` (seul le jeton `fnmode=` est réécrit, rename atomique).
