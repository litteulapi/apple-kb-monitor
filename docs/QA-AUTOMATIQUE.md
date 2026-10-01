# QA automatique — détecter les défauts avant l'utilisateur

Incident déclencheur (01/10/2026) : la fenêtre egui est passée « Ne répond plus » chez
l'utilisateur alors que 376 tests passaient. Personne n'avait lancé la fenêtre avec des données
réelles ni mesuré sa réactivité. Ce document décrit les cinq couches qui, depuis, cherchent
ces défauts sans intervention : chacune **détecte, consigne et signale**.

| Couche | Commande / unité | Quand | Ce qu'elle attrape | Où est le résultat |
|---|---|---|---|---|
| 1. Pipeline local | `scripts/ci-local.sh` | à la demande, avant une livraison | build, lint, tests (dont un test qui gèle), paquet, secrets, affirmations contraires au registre, binaires Apple | `scripts/out/<horodatage>/report.{txt,json}` |
| 2. Bout en bout | `tests/e2e/run.sh` (étape `e2e` du pipeline) | pipeline complet | fenêtre qui gèle, ne s'ouvre pas, ne se ferme pas, fuit des processus, rouvre, plante, écrit au clavier, change d'apparence | `…/e2e/summary.json` + captures |
| 3. Surveillance continue | `apple-kb-monitor-selfcheck.timer` → `akmctl selftest` | toutes les 15 min chez l'utilisateur | démon mort/bloqué/relancé en boucle, état périmé, liaison, erreurs du journal, core dumps, fenêtre figée, disque, historique, versions | `~/.local/state/apple-kb-monitor/selfcheck.json`, notification, issue Gitea (option) |
| 4. Garde-fou git | `.githooks/pre-push` | à chaque `git push` | sous-ensemble rapide de la couche 1 : refuse le push | `scripts/out/latest/` |
| 5. CI Gitea | `.gitea/workflows/ci.yml` | à chaque push (quand un runner existera) | idem couches 1-2 sur une machine propre | onglet Actions |

Aucune couche ne lit ni n'écrit le clavier, aucune n'a besoin de root.

## 1. Pipeline local — `scripts/ci-local.sh`

```bash
scripts/ci-local.sh             # complet (≈ 10 min avec le paquet et les tests de bout en bout)
scripts/ci-local.sh --no-e2e    # sans Xvfb
scripts/ci-local.sh --fast      # sous-ensemble du pre-push (≈ 1 min avec un cache chaud)
scripts/ci-local.sh --only test,secrets
scripts/ci-local.sh --list
```

Code de retour : `0` toutes les étapes requises passent, `1` au moins une échoue, `64` usage.
Chaque passage écrit `scripts/out/<horodatage UTC>/report.txt`, `report.json` (étapes, statut,
durée, résumé, chemin du journal) et `logs/<étape>.log` ; `scripts/out/latest` pointe sur le
dernier. `scripts/out/` n'est jamais commité. `CARGO_TARGET_DIR` est respecté.

| Étape | Requise | Contrôle |
|---|---|---|
| `versions` | oui | Cargo = PKGBUILD = .SRCINFO = CHANGELOG (+ pkgrel), et chaque fichier que le PKGBUILD installe existe |
| `secrets` | oui | `scripts/qa_checks.py secrets` : clés privées, jetons (GitHub/GitLab/Slack/AWS, jeton Gitea de `~/.config/gitea/token` recherché **littéralement**), clés de liaison BlueZ (`Key=`, IRK, LTK, `link_keys` debugfs), rapport `0x4C` publié au-delà de l'adresse hôte (les 12 octets suivants doivent être nuls ou caviardés, #133) ; `gitleaks dir` en plus s'il est installé. Arbre suivi + fichiers non suivis non ignorés |
| `fmt` | oui | `rustfmt` : un fichier `.rs` **ajouté** doit être propre ; un fichier **modifié** ne doit pas avoir plus de blocs non formatés que sur `main` (l'arbre historique en a 229, compté pour information) |
| `clippy` | oui | `cargo clippy --workspace --all-targets -D warnings` |
| `test` | oui | `cargo test --workspace` sous `timeout --kill-after` (900 s, `AKM_CI_TEST_TIMEOUT`) : **un test qui gèle est un échec `TIMEOUT`** qui nomme le test (« has been running for over 60 seconds ») ; sortie complète dans `logs/test-full.log`, chaque échec avec 12 lignes de contexte |
| `claims` | oui | `scripts/qa_checks.py claims` : les documents et chaînes visibles ne contredisent pas le registre `docs/CONTRE-AUDIT.md` (« 0xF5 ADC », « ARM7TDMI », « firmware signé », RSSI en « dBm », « 0x4C identity key ») sauf si la ligne réfute (« pas », « ❌ », « ≠ », « inconnu »…) |
| `redaction` | oui | aucun binaire Apple ou tiers (Mach-O, xar/pkg, dmg, ELF, PE, extensions firmware), aucun code décompilé collé (≥ 3 lignes d'artefacts Ghidra/IDA : `undefined4 x`, `unaff_`, `extraout_`, `in_stack_`, `CONCAT44(`…) |
| `deny` | oui | `cargo deny check advisories bans sources` (politique `apihub-app/deny.toml`) |
| `audit` | non | `cargo audit` (base locale si hors ligne) |
| `udev` | oui | `udevadm verify` sur `udev/*.rules` |
| `qml` | oui | `qmllint` (erreurs de syntaxe ; les imports Plasma ne sont pas résolubles hors session) + `kcm/lint/qmllint.sh` sur les pages du KCM (tout avertissement échoue, #259) |
| `kcm` | oui (`--fast` aussi) | `cmake` + build du C++ du KCM + `ctest`, puis `kcm/tests/toml_js_test.py` (sorties de `Toml.js` relues par `tomllib`, #258) ; ignorée si cmake/ECM absents |
| `shell` | oui | `bash -n` / `sh -n` sur chaque script (+ `.install`, hooks) ; `shellcheck -S warning` s'il est installé |
| `c` | oui | `gcc -Wall -Wextra -Werror rssi-helper.c` |
| `security` | oui | `tests/check-security-files.sh` + texte brut du widget |
| `units` | oui | `systemd-analyze --user verify` des unités de `systemd/` |
| `plasma` | non | `plasma/tests/run-widget-tests.sh` si `qmltestrunner` existe |
| `package` | oui | `makepkg -f --nodeps` dans une **copie** de l'arbre (`scripts/out/…/pkgbuild`), puis `qa_checks.py package` : fichiers attendus (`scripts/package-expected.txt`), aucun fichier mondialement inscriptible, aucun setuid, `.INSTALL` applique `setcap cap_net_admin`, version du paquet = Cargo, aucune dépendance Python |
| `e2e` | oui | couche 2 (sauf `--no-e2e`/`--fast`) |

Une étape qui renvoie `77` (outil absent) est `skip`, jamais un succès silencieux ; une étape
non requise qui échoue apparaît en `warn`.

**Listes connues.** `scripts/{secrets,claims,redaction,package}-allow.tsv` : `chemin<TAB>regex<TAB>raison`.
Une entrée sans raison fait échouer le contrôle ; un défaut connu porte son numéro d'issue
(aujourd'hui : 3 affirmations de docs contraires au registre, #228). Une constatation listée
reste affichée (`KNOWN (issue #228)`), elle ne fait simplement plus échouer.

## 2. Tests de bout en bout — `tests/e2e/`

```bash
tests/e2e/run.sh                         # tout (≈ 6 min) ; compile apihub-app + démon en release
tests/e2e/run.sh --only responsive,spy --duration 20
tests/e2e/run.sh --bin-dir DIR           # éprouver d'autres binaires (ex. une version publiée)
tests/e2e/run.sh --update-refs           # régénérer les captures de référence (relire le diff !)
```

**Isolation** (`bwrap`, espace de noms utilisateur, sans root) : `/sys/class/hidraw`,
`/sys/bus/hid/devices`, `/sys/class/power_supply` remplacés par des arbres construits depuis
`tests/fixtures/a1314_iso` (MAC anonymisée `04:DB:56:00:E2:E0`) ; `/dev` minimal où
`/dev/hidraw7` (le clavier) est un **fichier vide** — toute écriture le ferait grossir — et
`/dev/hidraw3` une fausse souris qui ne doit jamais être ouverte ; `/usr/lib/apple-kb-monitor`
(rssi-helper, akm-helper) masqué, `pkexec` = `false` ; `/proc` et PID privés (le harnais refuse
de tourner hors de ce bac à sable) ; serveur X propre (`Xvfb -displayfd`, 1920×1200) ; deux bus
D-Bus privés : session et « système » avec un faux BlueZ/UPower (`tests/e2e/fakes.py`) qui
**enregistre chaque appel**. Le lecteur de sysfs `power_supply` d'un clavier HID peut provoquer
un GET_REPORT réel dans le noyau : il est donc lui aussi remplacé.

**Mesure de réactivité.** `tests/e2e/xprobe.py` envoie `_NET_WM_PING` à la fenêtre toutes les
100 ms, comme un gestionnaire de fenêtres qui décide « Ne répond plus ». winit y répond depuis
sa boucle d'événements, c'est-à-dire le fil qui exécute `update()` d'egui : une trame longue ou
un appel bloquant retarde la réponse d'autant. Critères : aucune réponse au-delà de 100 ms
(`--max-latency-ms`) après 3 s de démarrage, aucune absence de réponse en 5 s. Des survols
souris (xdotool) provoquent des trames pendant la mesure. Si l'application publie la pulsation
du §3.2, `frame_max_ms` est joint au résultat.

| Scénario | Données / conditions | Échoue si |
|---|---|---|
| `responsive` | faux démon, `StateChanged` chaque seconde, historique réel anonymisé (`fixtures/history-seed.jsonl`, horodatages relatifs), 60 s | pause > 100 ms ou pas de réponse |
| `open_close` | 10 × ouvrir, ping, `WM_DELETE_WINDOW` | ne se ferme pas en 10 s, code ≠ 0, processus `apihub-app` résiduel, fenêtre rouverte |
| `daemon_absent` | aucun démon : repli local sur le faux hidraw | gel, panique |
| `daemon_slow` | chaque appel du démon répond en 3 s | gel |
| `daemon_dies` | le démon meurt à t = 8 s | gel, mort de la fenêtre |
| `daemon_garbage` | JSON invalide | gel, panique |
| `desktop` | faux portail de réglages + `StatusNotifierWatcher` réactifs | gel |
| `desktop_slow` | portail et hôte du tray qui répondent en 40 s (dbus-broker n'a pas de délai de réponse) | pas de fenêtre en 90 s, gel, **ou la fenêtre prend le démon pour absent et ouvre elle-même le clavier** (#199) |
| `history_50k` | 50 000 points (forme réelle, 5 min d'écart) par `History` et par fichier | gel |
| `history_corrupt` | octets non UTF-8, NUL, lignes tronquées, nombres hors bornes, ligne de 1 Mio (#222, #223) | gel, panique |
| `resize` | 1×1, 120×90, 260×900, 1900×140, 1920×1200 | pas de réponse, pause > 300 ms, mort |
| `spy` | **vrai** `apple-kb-monitord` sur le faux clavier + fenêtre, tous deux sous `strace -y -P /dev/hidraw7 -P /dev/hidraw3` | écriture sur le clavier, `HIDIOCS*` (SET report), ioctl inconnu, accès à la souris, fenêtre qui ouvre le clavier alors que le démon le possède, fichier-clavier qui grossit, appel BlueZ hors lecture (Connect, Pair, RemoveDevice, Set…) |
| `screens` | données figées, 900×700 et 420×700 | fenêtre uniforme (vide/noire), > 3 % des pixels diffèrent de `tests/e2e/refs/*.png` (image de différence jointe) |

**Échecs connus.** `tests/e2e/known-failures.tsv` (`scénario<TAB>#issue<TAB>raison`) : le
scénario tourne toujours et s'affiche `KNOWN-FAIL #233`, sans faire échouer le pipeline ; s'il
repasse, le rapport demande de retirer la ligne (dans le commit du correctif). Exemple réel :
`desktop_slow` échouait sur `main` 8f73964 (aucune fenêtre en 90 s quand le portail et l'hôte
du tray répondent en 40 s, #232/#233) ; il passe sur `main` b570c1d (paquet 3.1.0-8, « D-Bus
borné ») et la ligne a été retirée. Liste vide au 01/10/2026.

Chaque échec donne un message en clair et le chemin d'une capture prise **au moment du gel**
(`stall-t13s.png`), ou de l'écran entier si la fenêtre n'apparaît pas.

Limites connues : le gel sous Wayland d'une fenêtre réduite (#231, `eglSwapBuffers` bloqué) ne
se reproduit pas sous Xvfb ; les identifiants de rapport ne sont pas visibles dans `strace`
(on voit `HIDIOCGFEATURE(len)` ; la liste blanche des identifiants est vérifiée par les tests
unitaires de `akm-core/src/read_policy.rs`) ; la comparaison de captures détecte un changement
(chevauchement, légende tronquée, élément manquant), pas sa cause : après une évolution voulue
de l'interface, `--update-refs` puis relecture des images.

## 3. Surveillance continue — `akmctl selftest`

`systemd/apple-kb-monitor-selfcheck.{service,timer}` : installés et **activés pour tous les
utilisateurs** par le paquet (lien `timers.target.wants`), premier passage 5 min après
l'ouverture de session puis toutes les 15 min. Désactiver : `systemctl --user mask
apple-kb-monitor-selfcheck.timer`. Lancer à la main : `akmctl selftest [--json]`.

Le selftest n'interroge que le démon (D-Bus, délai maximal 5 s), systemd, le journal, les
enregistrements de core dumps, `/proc` et `statvfs` ; `akmctl doctor` y est exécuté avec un
délai maximal de 20 s (son étape hidraw se limite à un `open()` de contrôle d'accès, sans
échange de rapport).

| Contrôle | `bad` (grave) | `warn` |
|---|---|---|
| `daemon` | pas sur le bus, ou **pas de réponse à `GetState` en 5 s** (démon bloqué) | réponse > 1 s |
| `daemon-restarts` | `NRestarts` a augmenté depuis le passage précédent (boucle de plantage) | — |
| `freshness` | clavier connecté, dernière acquisition > 24 h | > 6 h |
| `versions` | — | démon ≠ akmctl ≠ paquet (démon non relancé après mise à jour) |
| `link` | verdict `akmctl doctor` mauvais, ou doctor bloqué > 20 s | verdict à surveiller |
| `journal-daemon` | `panicked at` dans le journal du démon depuis le dernier passage | lignes d'avertissement/erreur |
| `panic` | panique d'un de nos programmes dans le journal utilisateur | — |
| `journal-bluetooth` | — | erreurs de `bluetoothd` (journal système lisible : groupe `systemd-journal`) |
| `coredumps` | core dump de `apihub-app`, `apple-kb-monitord`, `rssi-helper`, `akm-helper`, `akmctl` **installés** (`/usr/bin`, `/usr/lib/apple-kb-monitor` ; `AKM_SELFTEST_EXE_DIRS` pour d'autres répertoires) avec `si_code ≠ SI_USER` (un `kill -SEGV` manuel est ignoré, un `abort()`/panique est compté) | — |
| `ui-heartbeat` | fenêtre ouverte dont la pulsation a plus de 10 s : **fenêtre figée** | trame > 250 ms |
| `ui-cpu` | — | fenêtre à > 90 % d'un cœur en moyenne entre deux passages (boucle active) |
| `disk` | < 100 Mio ou < 1 % libres (`/var/lib/bluetooth`, `~/.local/state/apple-kb-monitor`) | < 1 Gio ou < 5 % |
| `history` | — | lignes illisibles parmi les 1000 dernières, fichier > 50 Mio |

**Résultat** : `$XDG_STATE_HOME/apple-kb-monitor/selfcheck.json` (écrit atomiquement) :
`verdict`, `checks[]` (`id`, `level`, `key`, `text`), `new_grave[]`, `reported[]`, `since`,
`finished`, `state` (compteurs pour le passage suivant). Code de retour `1` si un contrôle est
`bad` : l'unité apparaît alors dans `systemctl --user --failed` tant que le problème dure.

**Signalement dédoublonné.** Chaque problème a une clé stable (`daemon:unreachable`,
`coredumps:apihub-app:SIGABRT:<pid>`…). Seule une clé `bad` absente du passage précédent est
« nouvelle » : une notification de bureau (urgence critique) par nouveauté, jamais répétée
toutes les 15 min. Option désactivée par défaut, `AKM_SELFCHECK_ARGS=--gitea-issue` dans
`~/.config/apple-kb-monitor/selfcheck.env` : une issue Gitea par clé, titre préfixé
`[selfcheck:<clé>]`, recherchée parmi les issues ouvertes avant création (jamais deux fois) ;
jeton lu dans `~/.config/gitea/token` et passé à `curl` par un fichier d'en-têtes `0600`
supprimé aussitôt (jamais en argument de processus).

### 3.2 Contrat de pulsation de la fenêtre (à implémenter dans `apihub-app`)

Un gestionnaire de fenêtres Wayland ne permet pas à un tiers de « pinger » une fenêtre. La
fenêtre publie donc elle-même une pulsation ; le selftest et les tests de bout en bout la
lisent déjà (absente : `info` « freeze not measurable »).

- Fichier : `$XDG_RUNTIME_DIR/apple-kb-monitor/ui-heartbeat.json` (répertoire `0700`).
- Écrit par le **fil de l'interface, à la fin d'une trame** (dans `update()`), au plus une fois
  par seconde, par écriture d'un fichier temporaire puis `rename()` (jamais un fichier tronqué).
- Pour qu'une fenêtre au repos pulse quand même : `ctx.request_repaint_after(1 s)`.
  Fenêtre réduite/masquée : continuer à pulser (c'est justement le cas du gel #231).
- Contenu (une ligne JSON) :
  `{"pid":1234,"ts_ms":1790860000123,"frames":5321,"frame_max_ms":18.4,"frame_last_ms":6.1,"version":"3.1.0"}`
  — `ts_ms` horloge Unix en ms, `frame_max_ms` maximum de la durée de `update()` sur la
  dernière seconde, `frames` compteur depuis le lancement.
- Le fichier est supprimé à la fermeture normale.
- Interprétation : processus `apihub-app` vivant + `ts_ms` plus vieux que 10 s = **figé** (`bad`) ;
  `frame_max_ms` > 250 = à-coup visible (`warn`) ; `pid` différent = autre instance (`info`).

Écrire depuis `update()` (et non depuis un fil à part) est le point essentiel : un fil séparé
continuerait à pulser pendant que l'interface est bloquée.

## 4. Garde-fou git — `.githooks/pre-push`

Activé dans ce dépôt (`git config core.hooksPath .githooks`, à refaire sur un nouveau clone).
Au `git push` : refuse si des fichiers suivis ont des modifications non commitées (les contrôles
ne testeraient pas ce qui part), puis lance `scripts/ci-local.sh --fast` (versions, secrets,
rustfmt des fichiers modifiés, clippy `-D warnings`, tous les tests avec délai de gel 600 s,
syntaxe des scripts) et **refuse le push** au premier échec, avec le chemin du rapport.
Une suppression de branche seule n'est pas contrôlée. Contournement d'urgence :
`git push --no-verify` (à justifier dans le message de commit suivant).

## 5. CI Gitea

État constaté le 01/10/2026 par l'API :

- dépôt `adminapi/apple-kb-monitor` : `has_actions = false` ;
- runners visibles : 0 (`/repos/adminapi/apple-kb-monitor/actions/runners`, `/user/actions/runners`,
  `/orgs/agenceapi/actions/runners`) ; la liste des runners d'instance
  (`/admin/actions/runners`) répond 403 au jeton utilisé (non administrateur) ;
- dernière tâche Actions visible sur l'instance : `agenceapi/apinetwork-fr`, 11/07/2026, en
  échec ; aucune activité depuis sur les 20 dépôts dont les Actions sont activées.

Faute de runner, les Actions **n'ont pas été activées** (des tâches resteraient « en attente »
indéfiniment) et `scripts/ci-local.sh` reste la référence. `ci.yml` contient déjà les jobs
`rust`, `shell-c-udev`, `reverse-tools`, et deux nouveaux : `qa-static` (étapes statiques du
pipeline) et `e2e` (Xvfb ; sortie 77 = conteneur sans espaces de noms utilisateur, signalé en
avertissement).

Enregistrer un runner (administrateur de l'instance) :

1. **Où** : pas sur kub-gra (nœud de production unique ; des builds Rust y ont saturé
   `/var/lib/docker` et fait tomber Nextcloud le 25/05/2026). Machine conseillée : node7 HOME,
   ou un hôte de build dédié avec ≥ 30 Go libres.
2. Jeton : Administration du site → Actions → Runners → « Créer un runner » (ou
   `gitea actions generate-runner-token` sur le serveur Gitea).
3. `act_runner register --no-interactive --instance https://gitea.pika.agenceapi.fr --token <jeton>
   --name build-node7 --labels ubuntu-latest:docker://catthehacker/ubuntu:act-latest` ;
   dans `config.yaml` : `capacity: 1`, et si le runner est hors du LAN, une entrée
   `extra_hosts`/`/etc/hosts` pour `gitea.pika.agenceapi.fr` (le DNS public ne la résout pas).
4. Pour le job `e2e`, le conteneur doit pouvoir créer des espaces de noms utilisateur :
   `container.options: --security-opt seccomp=unconfined --security-opt apparmor=unconfined`
   (déjà dans `ci.yml`) et `kernel.unprivileged_userns_clone=1` sur l'hôte.
5. Activer : `PATCH /api/v1/repos/adminapi/apple-kb-monitor {"has_actions": true}` puis pousser.
6. Purge : `docker system prune --filter until=24h` quotidien sur l'hôte du runner.

## 6. Preuves : défauts volontaires attrapés (01/10/2026)

| Couche | Défaut introduit | Résultat |
|---|---|---|
| 1 | test `loop { sleep(1 s) }` ajouté à `akm-core/tests` | `test` **KO** : « test qa_demo_hangs_forever has been running for over 60 seconds », « TIMEOUT: cargo test did not finish in 150s », pipeline code 1 |
| 1 | `PKGBUILD` installe `docs/QA-AUTOMATIQUE.md` avant que le fichier n'existe | `versions` **KO** « PKGBUILD installs a missing file », `package` **KO** dans `package()` |
| 1 | 3 lignes de docs (« `0xF5` ADC », « RSSI < -80 dBm », « RSSI −48 dBm ») | `claims` **KO** → issue #228, mises en liste connue |
| 1/4 | faux jeton `ghp_…` dans un fichier non suivi | `secrets` **KO**, `pre-push` **REFUSED** (voir §6.1) |
| 2 | binaire de `main` 8f73964 (avant correctif) avec un portail et un hôte de tray lents | `desktop_slow` **KO** : aucune fenêtre en 90 s (appels D-Bus synchrones avant la première trame, #232/#233) ; **passe** sur `main` b570c1d (3.1.0-8) : le test distingue l'avant et l'après correctif |
| 2 | enveloppe qui laisse un processus `apihub-app` après fermeture | `open_close` **KO** « cycle 1/10: process leak after close: apihub-app pid(s) [18] still alive » |
| 2 | enveloppe qui fige la fenêtre 7 s (`SIGSTOP`) | `responsive` **KO** « window NOT RESPONDING (1 ping without answer within 5 s) », capture `stall-t13s.png` |
| 3 | faux démon qui répond en 10 s (bus privé) | `daemon` **bad** « no answer within 5 s (daemon stuck?) », code 1 |
| 3 | pulsation vieille de 45 s pour un processus `apihub-app` vivant | `ui-heartbeat` **bad** « window frozen: no frame for 45 s » |
| 3 | `abort()` d'un programme nommé `apihub-app` | ignoré hors répertoires installés ; avec `AKM_SELFTEST_EXE_DIRS` : `coredumps` **bad** « apihub-app crashed (SIGABRT…) » |
| 3 | même problème sur 3 passages, faux serveur de notifications | 1 notification par clé (2 au total), `new_grave` vide aux passages 2 et 3 |
| 3 | `--gitea-issue` lancé deux fois | issues #237/#238 créées puis « already open » ; fermées comme tests |

### 6.1 Exemple : push refusé

```text
$ git push            # avec un fichier non suivi contenant demo_token = "ghp_QQQ…"
pre-push: scripts/ci-local.sh --fast (fmt, clippy, tests, secrets, versions)...
==> versions         pass (0.0s)
==> secrets          fail (1.1s) -> scripts/out/20261001T135240Z/logs/secrets.log
      leak-demo.txt:1: [github-token] looks like a secret: ghp_QQQQQQQQ…
      -- secrets: 1 finding(s), 0 known/allow-listed
==> fmt              pass (0.4s)
==> clippy           pass (1.4s)
==> test             pass (46.0s)
==> shell            pass (0.0s)
RESULT: FAILED: secrets
pre-push: REFUSED - fix the failing step (report: scripts/out/latest/report.txt)
```
