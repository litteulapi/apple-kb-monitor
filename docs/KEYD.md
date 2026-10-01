# keyd 2.6.0 : plantage du 2026-10-01 et place de keyd dans le paquet

Issue : [#246](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/246) · branche `fix/keyd-plantage`

## Résumé

| Question | Réponse prouvée |
|---|---|
| Qu'est-ce qui a planté ? | keyd 2.6.0 (Arch `keyd 2.6.0-5`, build-id `fe1f6dcf6909da903bcd8450db266132d1442418`), PID 1145, SIGSEGV `SEGV_MAPERR`, 2026-10-01 12:35:13 |
| Déclencheur | `keyd reload`, lancé par `post_upgrade` de apple-kb-monitor 3.1.0-4 → 3.1.0-5 (`pacman.log` 12:35:10, même PID : le journal montre `CONFIG: parsing` puis `DEVICE: match`, pas un redémarrage) |
| Cause racine | défaut **amont** de keyd : `reload()` libère toutes les `struct keyboard` (`free_configs()`) sans remettre à `NULL` le pointeur statique `active_kbd` ; le clic suivant d'une souris que keyd ne gère pas appelle `process_keypress(active_kbd, KEYD_EXTERNAL_MOUSE_BUTTON)` sur la mémoire libérée |
| Notre configuration est-elle fautive ? | **Non.** Toute configuration qui saisit un clavier reproduit le défaut, y compris `[ids]` + `05ac:0256` seul ou `[ids]` + `*` (bissection ci-dessous). `keyd check` : aucune erreur, aucun avertissement |
| Correctif amont | une ligne : `active_kbd = NULL;` dans `reload()` (ou `free_configs()`) — déjà proposé en amont : [rvaiya/keyd#1319](https://github.com/rvaiya/keyd/pull/1319), issue [#1320](https://github.com/rvaiya/keyd/issues/1320), toutes deux ouvertes au 2026-10-01, non publiées dans une version |
| Correctif chez nous | le paquet ne lance plus jamais `keyd reload` ni ne redémarre keyd ; keyd passe en `optdepends` ; la configuration devient un exemple dans `/usr/share/doc` |

## 1. Preuves

### Journal et pacman

```
12:35:10 [ALPM] upgraded apple-kb-monitor (3.1.0-4 -> 3.1.0-5)      # post_upgrade -> _akm_keyd_reload -> `keyd reload`
12:35:10 keyd[1145]: CONFIG: parsing /etc/keyd/apple-keyboard.conf   # IPC_RELOAD dans le démon déjà lancé
12:35:10 keyd[1145]: DEVICE: ignoring 1d57:fa60:3c7f9e03 (2.4G Wireless Device)   # souris NON gérée
12:35:10 keyd[1145]: DEVICE: match 05ac:0256:09409bbc /etc/keyd/apple-keyboard.conf (Clavier de maria #1)
12:35:13 systemd-coredump: Process 1145 (keyd) dumped core (SEGV)
12:35:14 keyd.service: Failed with result 'core-dump'.               # Restart= absent : reste failed
```

La configuration chargée à 12:35:10 est identique octet pour octet à `keyd/apple-keyboard.conf` du commit `e023c11` (#126).

### Pile symbolisée

Symboles : `https://debuginfod.archlinux.org/buildid/fe1f6dcf…/debuginfo` (même build-id que `/usr/bin/keyd` et que l'image du coredump).

| Cadre | Adresse | Fonction | Source (keyd 2.6.0) |
|---|---|---|---|
| #0 | keyd+0xbecf | `handle_chord` (inlinée) dans `process_event` | `src/keyboard.c:941` (`kbd->config.chord_hold_timeout`) et `:1190` |
| #1 | keyd+0x13c56 | `kbd_process_events` (inlinée) | `src/keyboard.c:1267` |
| #2 | keyd+0x8495 | `process_keypress` ← `event_handler` | `src/daemon.c:487` ← `src/daemon.c:571` |
| #3 | keyd+0x2c80 | `evloop` ← `run_daemon` ← `main` | `src/evloop.c:114`, `src/daemon.c:618`, `src/keyd.c:265` |

Arguments de `process_event` dans le core : `kbd=0x7fcf1174a010, code=196, pressed=1`. `196` = `KEYD_EXTERNAL_MOUSE_BUTTON` (`src/keys.h:278`) : seul `daemon.c:571` l'émet, pour un clic/défilement d'une souris **non gérée** par keyd.

Instruction fautive : `mov 0xb44a8(%rdi),%r14` avec `rdi = kbd = 0x7fcf1174a010` → adresse `0x7fcf117fe4b8`, absente de toutes les projections du processus.

### Le pointeur pendant, lu dans le coredump

```
(gdb) p active_kbd       -> (struct keyboard *) 0x7fcf1174a010   # bloc mmap libéré par free_configs(), plus projeté
(gdb) p configs->kbd     -> (struct keyboard *) 0x55d63c22cfb0   # le nouveau clavier alloué par le reload
(gdb) p configs->next    -> 0x0                                  # un seul fichier : /etc/keyd/apple-keyboard.conf
(gdb) p sizeof(struct keyboard) -> 741648                        # > seuil mmap de glibc : free() = munmap() => SEGV immédiat
```

### Code en cause (`src/daemon.c`, keyd 2.6.0 et `master` f564288 au 2026-10-01)

```c
static struct keyboard *active_kbd = NULL;          /* l.17 */

static void reload(void)                            /* l.295 */
{
	free_configs();      /* free(ent->kbd) pour chaque config : active_kbd pend */
	load_configs();
	for (i = 0; i < device_table_sz; i++)
		manage_device(&device_table[i]);   /* remet dev->data, PAS active_kbd */
	clear_vkbd();
}
...
	} else if (!ev->dev->is_virtual && ev->dev->capabilities & CAP_MOUSE) {   /* l.569 */
		if (active_kbd && (ev->devev->type == DEV_KEY || ev->devev->type == DEV_MOUSE_SCROLL))
			timeout = process_keypress(active_kbd, KEYD_EXTERNAL_MOUSE_BUTTON, ev->timestamp);
```

Lecteurs de `active_kbd` qui ne revalident pas après un reload : `EV_TIMEOUT` (l.509), défilement d'un périphérique géré (l.551-563, seulement après réaffectation donc sûr), souris non gérée (l.571, **notre cas**), `add_listener()` (l.102, `keyd listen`). Le pointeur n'est réaffecté qu'à la prochaine touche d'un clavier géré (l.514) : entre le reload et cette touche, un clic de souris, un délai en attente ou un client `keyd listen` suffit.

Conditions réunies sur PC01 : un clavier géré a servi avant le reload (`active_kbd` non nul), une souris 2.4G non gérée, un clic dans les 3 s qui ont suivi `pacman -U` — avant toute frappe sur le clavier Apple.

## 2. Reproduction sans risque

`tests/keyd/run-harness.sh` compile `tests/keyd/reload-harness.c` contre les sources officielles keyd 2.6.0 (archive épinglée `sha256 6970896…a90744e26`, celle du paquet Arch 2.6.0-5) avec `-fsanitize=address,undefined`. Le harnais inclut `src/daemon.c` pour appeler `reload()` et `event_handler()` tels quels et remplace tout ce qui touche le système : backend `vkbd/stdout.c` (pas de `/dev/uinput`), table de périphériques factice et `device_grab()` neutre (pas de `/dev/input`, pas d'`EVIOCGRAB`), pas de socket IPC, pas de boucle d'événements. Aucun droit root, rien n'est saisi, rien n'est écrit sur un clavier.

Scénario `mouse` (celui de PC01) : charger la configuration → brancher `05ac:0256:09409bbc` (clavier) et `1d57:fa60:3c7f9e03` (souris) → taper `a` sur le clavier → `reload()` (ce que fait `IPC_RELOAD`) → clic sur la souris.

```
ok    vanilla  parse    pass
ok    vanilla  rekey    pass       # reload puis frappe au clavier avant la souris : active_kbd réaffecté
ok    vanilla  mouse    asan       # PC01
ok    vanilla  timeout  asan       # même défaut par EV_TIMEOUT (chemin de rvaiya/keyd#1320)
ok    patched  parse    pass
ok    patched  rekey    pass
ok    patched  mouse    pass
ok    patched  timeout  pass
```

Rapport ASAN du scénario `mouse` (pile identique à celle du coredump) :

```
ERROR: AddressSanitizer: heap-use-after-free ... READ of size 8
    #0 handle_chord        src/keyboard.c:940
    #1 process_event       src/keyboard.c:1190
    #2 kbd_process_events  src/keyboard.c:1267
    #3 process_keypress    src/daemon.c:484
    #4 event_handler       src/daemon.c:571
... located 738464 bytes inside of 741648-byte region     # une struct keyboard libérée par free_configs()
```

Durée : ~4 s (compilation comprise). L'archive est mise en cache dans `${XDG_CACHE_HOME:-~/.cache}/apple-kb-monitor/` ; hors ligne sans cache le test est sauté (code 77) ; `--src DIR` accepte un arbre déjà extrait.

## 3. Bissection de notre configuration

Binaire vanilla du harnais, scénarios `mouse` et `timeout` (`UAF` = heap-use-after-free détecté par ASAN) :

| Configuration testée | parse | mouse | timeout |
|---|---|---|---|
| `keyd/apple-keyboard.conf` complète (17 modèles, 25 ids, 12 remappages) | ok | UAF | UAF |
| la même sans commentaires ni lignes vides | ok | UAF | UAF |
| section `[ids]` seule (25 ids, aucun `[main]`) | ok | UAF | UAF |
| `[ids]` + `05ac:0256` seul | ok | UAF | UAF |
| `[ids]` + `05ac:0256` + `f3 = macro(M-z)` | ok | UAF | UAF |
| `[ids]` + `*` seul (configuration générique) | ok | UAF | UAF |
| `[ids]` + `004c:0267` (ne saisit pas 05ac:0256) | ok | ok | ok |

Conclusion : aucune ligne n'est en cause — ni les ids en double vendeur 05ac/004c, ni les commentaires, ni les macros, ni les noms `scale`/`dashboard`/`micmute`/`sleep`. Le seul facteur est « keyd saisit un clavier et reçoit un reload ». `keyd check keyd/apple-keyboard.conf` (keyd 2.6.0) : `No errors found`, aucun avertissement. Il n'existe donc **pas** de correctif de configuration : le correctif porte sur la façon dont le paquet touche keyd.

## 4. Correctif appliqué dans le dépôt

| Fichier | Changement |
|---|---|
| `apple-kb-monitor.install` | `_akm_keyd_reload` supprimée : plus aucun `keyd reload` ni `systemctl … keyd` ; `post_upgrade` depuis < 3.1.0-10 affiche une notice (keyd optionnel, comment le garder, « restart, jamais reload ») ; message d'installation mis à jour |
| `PKGBUILD`, `.SRCINFO` | `keyd` retiré de `depends`, ajouté à `optdepends` ; `etc/keyd/apple-keyboard.conf` retiré de `backup=` ; la configuration est installée en `/usr/share/doc/apple-kb-monitor/examples/keyd/apple-keyboard.conf`, ce document en `/usr/share/doc/apple-kb-monitor/KEYD.md` |
| `keyd/apple-keyboard.conf` | contenu inchangé (les tests `led.rs` et `test_entrees_modeles.py` le lisent toujours) ; en-tête « exemple optionnel », procédure d'activation, avertissement `keyd reload` |
| `scripts/package-expected.txt` | `etc/keyd/apple-keyboard.conf` remplacé par les deux fichiers de `/usr/share/doc` |
| `tests/keyd/check-config.sh` | `keyd check` sur le fichier livré + garde-fous paquet (pas de `keyd reload`/`systemctl … keyd` exécutable dans le `.install`, keyd absent de `depends`, rien installé dans `/etc/keyd`) ; `--harness` lance aussi le harnais ASAN. Échoue sur `main` (5 constats), passe sur la branche |
| `tests/keyd/run-harness.sh`, `reload-harness.c`, `keyd-2.6.0-reload-active_kbd.patch` | harnais ASAN du § 2 et correctif amont appliqué à la variante `patched` |

Effet à la mise à jour : `/etc/keyd/apple-keyboard.conf` non modifié est supprimé par pacman (modifié : conservé en `.pacsave`). Un keyd déjà lancé garde l'ancienne table en mémoire jusqu'à son prochain démarrage ; le scriptlet ne le touche pas.

**Ordre de fusion** : cette branche retire le remappage F3-F6 actif par défaut. Elle doit être fusionnée **avec ou après** la branche hwdb de `akmctl keymap` (`feat/touches-keymap`), sinon F3-F6 retombent sur les fonctions média de `hid-apple` entre les deux versions. Les fichiers hwdb/polkit/akmctl ne sont pas touchés ici. Le diagnostic d'`apihub-app` (`src/main.rs:662`) affiche encore « /etc/keyd/apple-keyboard.conf … » : à reformuler par l'agent qui intègre le hwdb (keyd absent = état normal).

### Ligne à ajouter à `scripts/ci-local.sh` (non modifié ici)

À côté de `step udev` :

```bash
s_keyd() { bash tests/keyd/check-config.sh --harness; }
step keyd       1 ""   -- s_keyd
```

(`--harness` sans réseau ni cache est sauté proprement ; pour un pré-push rapide : `bash tests/keyd/check-config.sh` sans `--harness`, < 1 s.)

## 5. Recommandation : keyd optionnel

1. **Deux remappages empilés se contredisent.** Le hwdb udev de `akmctl keymap` change les codes au niveau noyau (scancode → keycode) ; keyd relit ces codes déjà remappés et les remappe une seconde fois. Laisser `/etc/keyd/apple-keyboard.conf` actif par défaut garantit un double remappage dès que le hwdb arrive.
2. **keyd est un point de défaillance unique du clavier.** Il saisit le clavier en exclusif (`EVIOCGRAB`) : s'il plante, le clavier redevient brut ; s'il gèle, plus rien ne passe. Le hwdb n'a aucun processus.
3. **Un paquet ne doit pas piloter un démon système tiers.** Le seul appel à keyd (`keyd reload`) a précisément déclenché le plantage ; le défaut n'est corrigé dans aucune version publiée de keyd.
4. **Les LED n'en dépendent pas** : `akm-core/src/led.rs` lit `/etc/keyd` pour savoir si keyd saisit le clavier (#125) ; sans configuration il écrit sur l'evdev du clavier lui-même (`LedTarget::AppleDirect`), chemin déjà testé. Cas limite : après la mise à jour, un keyd encore lancé avec l'ancienne table saisit toujours le clavier alors que `/etc/keyd` ne le liste plus ; `led.rs` vise alors l'evdev saisi jusqu'au prochain arrêt/redémarrage de keyd — d'où la notice du scriptlet.

Le fichier reste fourni comme exemple pour qui veut les macros keyd (couches, macros par application, F27) à la place du hwdb.

## 6. Rapport de bogue amont (prêt, NON soumis)

Le défaut est déjà suivi en amont ([#1320](https://github.com/rvaiya/keyd/issues/1320), correctif [#1319](https://github.com/rvaiya/keyd/pull/1319)). Le texte ci-dessous est donc rédigé comme **commentaire** pour #1320 ; il apporte un chemin non chord, sans délai en attente, déclenché par une souris non gérée, et un harnais ASAN sans périphérique.

> **Another trigger: unmanaged mouse click right after `keyd reload` (keyd 2.6.0, Arch `keyd 2.6.0-5`)**
>
> Same dangling `active_kbd`, reached through `daemon.c:571` (`KEYD_EXTERNAL_MOUSE_BUTTON`), with no chord and no pending timeout.
>
> **Setup:** one config with an `[ids]` section matching a single Apple Bluetooth keyboard (`05ac:0256`); a USB wireless mouse (`1d57:fa60`) that keyd ignores. Reproduces with a two-line config:
> ```
> [ids]
> 05ac:0256
> ```
> (`[ids]` + `*` crashes too.)
>
> **Steps:** type any key on the managed keyboard → `keyd reload` → click the unmanaged mouse before touching the keyboard again → SIGSEGV.
>
> **Core dump (build-id fe1f6dcf6909da903bcd8450db266132d1442418, symbols from debuginfod.archlinux.org):**
> ```
> #0 handle_chord (inlined) / process_event  src/keyboard.c:941 / :1190   kbd=0x7fcf1174a010 code=196 pressed=1
> #1 kbd_process_events                      src/keyboard.c:1267
> #2 process_keypress ← event_handler         src/daemon.c:487 ← :571
> #3 evloop ← run_daemon ← main               src/evloop.c:114, daemon.c:618, keyd.c:265
> (gdb) p active_kbd   -> 0x7fcf1174a010   (unmapped: struct keyboard is 741648 bytes, so free() munmap()s it)
> (gdb) p configs->kbd -> 0x55d63c22cfb0   (the keyboard allocated by the reload)
> ```
> `code=196` is `KEYD_EXTERNAL_MOUSE_BUTTON`, only emitted at `daemon.c:571` for a device with `CAP_MOUSE` and no config.
>
> **Device-free reproduction:** a harness that `#include`s `src/daemon.c`, stubs `device_grab()`/`evloop()`, uses `vkbd/stdout.c` and feeds `event_handler()` with fake `EV_DEV_ADD`/`EV_DEV_EVENT` events, built with `-fsanitize=address,undefined` against v2.6.0: `heap-use-after-free` in `handle_chord` (keyboard.c:940) ← `process_keypress` (daemon.c:484) ← `event_handler` (daemon.c:571), "738464 bytes inside of 741648-byte region" freed by `free_configs()`. The `EV_TIMEOUT` path (daemon.c:509) fails the same way. With `active_kbd = NULL` at the start of `reload()` — equivalent to #1319 — all scenarios pass under ASAN. Harness (~120 lines of C) available on request.
>
> **Impact:** any package that runs `keyd reload` from an install hook (ours did) can take keyd down on a desktop where a mouse is clicked within seconds; `keyd.service` has no `Restart=`, so the keyboard stays unmapped until a manual restart.
>
> **Suggested fix:** merge #1319 (clear `active_kbd` in `free_configs()`), and possibly `Restart=on-failure` in `keyd.service.in`.

## 7. Ce que le gérant doit faire

- **Recommandé, dès que le hwdb de `akmctl keymap` est installé** : laisser keyd arrêté et le désactiver pour qu'il ne revienne pas au prochain démarrage (il est aujourd'hui `enabled` + `failed`) : `sudo systemctl disable keyd`. Le clavier passe alors par le hwdb seul.
- **Avant le hwdb** (paquet actuel 3.1.0-9), pour retrouver F3-F6 tout de suite : `sudo systemctl restart keyd` — sans risque : le défaut ne survient qu'après un `keyd reload`, jamais au démarrage. Ne jamais lancer `keyd reload` (ni un paquet ≤ 3.1.0-9 qui le fait) tant que keyd 2.6.0 est installé.
- keyd pourra être désinstallé (`sudo pacman -Rs keyd`) une fois le paquet issu de cette branche installé, s'il ne sert à rien d'autre sur le poste.
