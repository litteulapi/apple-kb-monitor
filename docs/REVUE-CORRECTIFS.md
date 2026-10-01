# Revue adversariale des correctifs clavier

- **Périmètre** : commits `617c3db..33142db` de la branche `audit/debug-complet` qui touchent le clavier (keyboard, bluez, rssi, history, poll/tray dans `main.rs`, scripts Python). DDC, écran et MQTT sont hors périmètre.
- **Méthode** : relecture des diffs avec `git show`. Reproduction dans un `git worktree` détaché, hors du dépôt principal.
- **Tests à `33142db`** : `cargo test` passe 67 tests sur 67, `pytest tests` en passe 92 sur 92. Les défauts ci-dessous passent tous entre les mailles des tests existants.
- **Date** : 2026-10-01.

## Verdict par commit

| Commit | Objet | Verdict | Défauts |
|---|---|---|---|
| `53fc950` | history : points invalides ignorés (#39) | **OK** (code) / écart de message | Le diff supprime aussi 3 unités systemd obsolètes (`apple-brightness`, `apple-kb-monitor-system`, `mqtt-bridge`). Cela relève de #4 (déjà fermée), pas de #39, et le message de commit n'en dit rien. `docs/TROUBLESHOOTING.md` mentionne encore ces unités. |
| `e4e6d94` | keyboard : fd HID, wake-monitor, calibration/PID/nom/LED (#52, #53) | **À corriger** | #79 : le wake-monitor n'est pas lancé si le clavier est absent au démarrage, et `last_wake` n'est jamais lu. #80 : la validation de calibration n'est pas portée en Python. La fermeture du fd, `O_CLOEXEC`, la sortie sur POLLHUP/POLLERR, le PID exact et le nom coupé au NUL sont corrects. Ni la fuite de fd ni la boucle POLLHUP n'ont de test dédié. |
| `af0a4a4` | rssi : appariement MGMT, 127, MAC strict (#56) | **À corriger** | #76 : un RSSI valide est rejeté si la puissance TX vaut 127. #77 : le repli MGMT Python n'est pas corrigé (127 dBm enregistré, reproduit). L'appariement opcode/événement est correct, car `CMD_COMPLETE` n'est livré qu'au socket émetteur. |
| `50efdb0` | rssi via helper `cap_net_admin`, timeout, cache (#69) | **À corriger** (hérité) | Reprend la règle « rejet si TX = 127 » (#76) dans `parse_helper_output` et `rssi-helper.c:116`. Le timeout, la lecture de sortie bornée et le cache sont corrects. |
| `dd7d376` | poll : RSSI conservé, alertes réarmées, plus de 100 %/0 V (#31, #32) | **À corriger** | #75 : le RSSI recopié n'a ni âge ni contrôle de MAC, et reste figé indéfiniment quand `read_rssi` échoue. Le réarmement à 20 %, le filtre `pct_opt` et l'effacement de « Remaining » sont corrects. |
| `bda9be1` | tray : relance SNI, scroll saturant (#38) | **À corriger** | #74 : seulement 12 essais en 60 s, et aucun ré-enregistrement quand plasmashell démarre tard ou redémarre. La boucle de relance de `run()` sur erreur est correcte. |
| `df54d15` | UI/tray (#28 Quit, tooltip n/a, etc.) | **OK** pour la partie clavier/tray | Quit : `ViewportCommand::Close` est testé à chaque `update()`, avec un repaint toutes les 2 s, donc la fermeture prend au plus environ 2 s. Le tooltip affiche « n/a » sans source. *Risque non confirmé* : fenêtre minimisée sous Wayland, où les frame callbacks sont suspendus et le Quit peut être retardé. |
| `9069a06` | bluez : PropertiesChanged, ré-enregistrement, MAC validée (#57) | **À corriger** | #72 (bloquant, avec `b753bdc` et `518c389`) : le provider ne démarre jamais en utilisateur, et l'échec est silencieux, sans nouvel essai. #81 : un échec d'enregistrement est journalisé toutes les 5 s sans fin. La logique PropertiesChanged et la validation de MAC sont correctes. |
| `6b69619` | durcissement des scripts Python (#40 à #51) | **À corriger** | #77 (repli MGMT 127), #78 (0 % exporté vers BlueZ si la lecture HID échoue, reproduit), #80 (calibration). Corrects : SDP borné, `read_history`, `ensure_state_dir`, `positive_int`/`percent_int`, réinitialisation du provider sur adaptateur retiré, debounce et valeurs `null` d'apihub-settings. Remarque : `closeEvent` peut bloquer l'UI jusqu'à 15 s (`wait(15000)`). |
| `b753bdc` | policy D-Bus restreinte (#6) | **À corriger** | #72 : supprimer `allow own` pour le contexte `default` casse `bluez.rs:216`, qui demande toujours le nom. |
| `518c389` | udev 70- uaccess (#63) | **À corriger** | #72 : #63 est déclarée corrigée alors que le nom D-Bus est toujours demandé. #73 : les claviers Apple en USB (bus 0003) perdent l'accès hidraw. |

## Défauts ouverts

| Issue | Gravité | Résumé |
|---|---|---|
| #72 | Haute | Provider BlueZ Rust mort : la policy n'autorise plus le nom et l'échec est silencieux. |
| #73 | Moyenne | udev 70- : les claviers Apple branchés en USB n'ont plus accès au hidraw. |
| #74 | Moyenne | Tray : pas de ré-enregistrement après 60 s ni après un redémarrage de plasmashell. |
| #78 | Moyenne | Python : 0 % exporté vers BlueZ quand la lecture HID échoue. |
| #75 | Faible-moyenne | RSSI figé indéfiniment (reprise sans âge ni MAC). |
| #76 | Faible | RSSI valide jeté quand la puissance TX vaut 127. |
| #77 | Faible | Python : le repli MGMT accepte 127 dBm, sans appariement. |
| #79 | Faible | Wake-monitor non lancé si le clavier est absent au démarrage, et horodatage jamais lu. |
| #80 | Faible | Python : calibration 0x5A non validée. |
| #81 | Faible | bluez : échec d'enregistrement journalisé toutes les 5 s. |

## Reproductions

- **#78** : `_setup_provider` avec `find_devices` mocké (un clavier) et `read_all_reports` qui renvoie `{}`. Résultat : `{'AA:BB:CC:DD:EE:FF': 0}`.
- **#77** : `get_conn_info` avec un socket mocké qui renvoie `CMD_COMPLETE` 0x0031 et rssi = 127. Résultat : `{'rssi_dbm': 127, ...}`.
- **#72** : `/usr/share/dbus-1/system.conf` contient `<deny own="*"/>`. Depuis `b753bdc`, seul `<policy user="root">` autorise le nom, alors que zbus `Builder::name()` + `build()` le demande au démarrage du provider.
