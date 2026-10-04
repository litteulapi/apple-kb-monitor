# keyd 2.6.0: the 2026-10-01 crash and keyd's place in the package

Branch `fix/keyd-plantage`

## Summary

| Question | Proven answer |
|---|---|
| What crashed? | keyd 2.6.0 (Arch `keyd 2.6.0-5`, build-id `fe1f6dcf6909da903bcd8450db266132d1442418`), PID 1145, SIGSEGV `SEGV_MAPERR`, 2026-10-01 12:35:13 |
| Trigger | `keyd reload`, run by the `post_upgrade` of apple-kb-monitor 3.1.0-4 → 3.1.0-5 (`pacman.log` 12:35:10, same PID: the journal shows `CONFIG: parsing` then `DEVICE: match`, not a restart) |
| Root cause | **upstream** keyd defect: `reload()` frees every `struct keyboard` (`free_configs()`) without resetting the static pointer `active_kbd` to `NULL`; the next click of a mouse that keyd does not manage calls `process_keypress(active_kbd, KEYD_EXTERNAL_MOUSE_BUTTON)` on the freed memory |
| Is our configuration at fault? | **No.** Any configuration that grabs a keyboard reproduces the defect, including `[ids]` + `05ac:0256` alone or `[ids]` + `*` (bisection below). `keyd check`: no error, no warning |
| Upstream fix | one line: `active_kbd = NULL;` in `reload()` (or `free_configs()`) — already proposed upstream: [rvaiya/keyd#1319](https://github.com/rvaiya/keyd/pull/1319), issue [#1320](https://github.com/rvaiya/keyd/issues/1320), both open as of 2026-10-01, not released in any version |
| Our fix | the package never runs `keyd reload` nor restarts keyd any more; keyd moves to `optdepends`; the configuration becomes an example in `/usr/share/doc` |

## 1. Evidence

### Journal and pacman

```
12:35:10 [ALPM] upgraded apple-kb-monitor (3.1.0-4 -> 3.1.0-5)      # post_upgrade -> _akm_keyd_reload -> `keyd reload`
12:35:10 keyd[1145]: CONFIG: parsing /etc/keyd/apple-keyboard.conf   # IPC_RELOAD in the already running daemon
12:35:10 keyd[1145]: DEVICE: ignoring 1d57:fa60:3c7f9e03 (2.4G Wireless Device)   # mouse NOT managed
12:35:10 keyd[1145]: DEVICE: match 05ac:0256:09409bbc /etc/keyd/apple-keyboard.conf (Clavier de alice #1)
12:35:13 systemd-coredump: Process 1145 (keyd) dumped core (SEGV)
12:35:14 keyd.service: Failed with result 'core-dump'.               # no Restart=: stays failed
```

The configuration loaded at 12:35:10 is byte-for-byte identical to `keyd/apple-keyboard.conf` of commit `e023c11`.

### Symbolized stack

Symbols: `https://debuginfod.archlinux.org/buildid/fe1f6dcf…/debuginfo` (same build-id as `/usr/bin/keyd` and as the coredump image).

| Frame | Address | Function | Source (keyd 2.6.0) |
|---|---|---|---|
| #0 | keyd+0xbecf | `handle_chord` (inlined) in `process_event` | `src/keyboard.c:941` (`kbd->config.chord_hold_timeout`) and `:1190` |
| #1 | keyd+0x13c56 | `kbd_process_events` (inlined) | `src/keyboard.c:1267` |
| #2 | keyd+0x8495 | `process_keypress` ← `event_handler` | `src/daemon.c:487` ← `src/daemon.c:571` |
| #3 | keyd+0x2c80 | `evloop` ← `run_daemon` ← `main` | `src/evloop.c:114`, `src/daemon.c:618`, `src/keyd.c:265` |

Arguments of `process_event` in the core: `kbd=0x7fcf1174a010, code=196, pressed=1`. `196` = `KEYD_EXTERNAL_MOUSE_BUTTON` (`src/keys.h:278`): only `daemon.c:571` emits it, for a click/scroll of a mouse **not managed** by keyd.

Faulting instruction: `mov 0xb44a8(%rdi),%r14` with `rdi = kbd = 0x7fcf1174a010` → address `0x7fcf117fe4b8`, absent from every mapping of the process.

### The dangling pointer, read in the coredump

```
(gdb) p active_kbd       -> (struct keyboard *) 0x7fcf1174a010   # mmap block freed by free_configs(), no longer mapped
(gdb) p configs->kbd     -> (struct keyboard *) 0x55d63c22cfb0   # the new keyboard allocated by the reload
(gdb) p configs->next    -> 0x0                                  # a single file: /etc/keyd/apple-keyboard.conf
(gdb) p sizeof(struct keyboard) -> 741648                        # > glibc mmap threshold: free() = munmap() => immediate SEGV
```

### Code involved (`src/daemon.c`, keyd 2.6.0 and `master` f564288 as of 2026-10-01)

```c
static struct keyboard *active_kbd = NULL;          /* l.17 */

static void reload(void)                            /* l.295 */
{
	free_configs();      /* free(ent->kbd) for each config: active_kbd dangles */
	load_configs();
	for (i = 0; i < device_table_sz; i++)
		manage_device(&device_table[i]);   /* resets dev->data, NOT active_kbd */
	clear_vkbd();
}
...
	} else if (!ev->dev->is_virtual && ev->dev->capabilities & CAP_MOUSE) {   /* l.569 */
		if (active_kbd && (ev->devev->type == DEV_KEY || ev->devev->type == DEV_MOUSE_SCROLL))
			timeout = process_keypress(active_kbd, KEYD_EXTERNAL_MOUSE_BUTTON, ev->timestamp);
```

Readers of `active_kbd` that do not revalidate after a reload: `EV_TIMEOUT` (l.509), scroll of a managed device (l.551-563, only after reassignment so safe), unmanaged mouse (l.571, **our case**), `add_listener()` (l.102, `keyd listen`). The pointer is only reassigned at the next key of a managed keyboard (l.514): between the reload and that key, a mouse click, a pending timeout or a `keyd listen` client is enough.

Conditions met on the test PC: a managed keyboard was used before the reload (`active_kbd` non-null), an unmanaged 2.4G mouse, a click in the 3 s following `pacman -U` — before any keystroke on the Apple keyboard.

## 2. Risk-free reproduction

`tests/keyd/run-harness.sh` builds `tests/keyd/reload-harness.c` against the official keyd 2.6.0 sources (pinned archive `sha256 6970896…a90744e26`, the one of the Arch 2.6.0-5 package) with `-fsanitize=address,undefined`. The harness includes `src/daemon.c` to call `reload()` and `event_handler()` as they are and replaces everything that touches the system: `vkbd/stdout.c` backend (no `/dev/uinput`), fake device table and no-op `device_grab()` (no `/dev/input`, no `EVIOCGRAB`), no IPC socket, no event loop. No root privileges, nothing is grabbed, nothing is written to a keyboard.

`mouse` scenario (the one of the test PC): load the configuration → plug `05ac:0256:09409bbc` (keyboard) and `1d57:fa60:3c7f9e03` (mouse) → type `a` on the keyboard → `reload()` (what `IPC_RELOAD` does) → click on the mouse.

```
ok    vanilla  parse    pass
ok    vanilla  rekey    pass       # reload then keystroke on the keyboard before the mouse: active_kbd reassigned
ok    vanilla  mouse    asan       # test PC
ok    vanilla  timeout  asan       # same defect via EV_TIMEOUT (path of rvaiya/keyd#1320)
ok    patched  parse    pass
ok    patched  rekey    pass
ok    patched  mouse    pass
ok    patched  timeout  pass
```

ASAN report of the `mouse` scenario (stack identical to the coredump's):

```
ERROR: AddressSanitizer: heap-use-after-free ... READ of size 8
    #0 handle_chord        src/keyboard.c:940
    #1 process_event       src/keyboard.c:1190
    #2 kbd_process_events  src/keyboard.c:1267
    #3 process_keypress    src/daemon.c:484
    #4 event_handler       src/daemon.c:571
... located 738464 bytes inside of 741648-byte region     # a struct keyboard freed by free_configs()
```

Duration: ~4 s (build included). The archive is cached in `${XDG_CACHE_HOME:-~/.cache}/apple-kb-monitor/`; offline without a cache the test is skipped (code 77); `--src DIR` accepts an already extracted tree.

## 3. Bisection of our configuration

Vanilla harness binary, `mouse` and `timeout` scenarios (`UAF` = heap-use-after-free detected by ASAN):

| Configuration tested | parse | mouse | timeout |
|---|---|---|---|
| full `keyd/apple-keyboard.conf` (17 models, 25 ids, 12 remappings) | ok | UAF | UAF |
| the same without comments or blank lines | ok | UAF | UAF |
| `[ids]` section alone (25 ids, no `[main]`) | ok | UAF | UAF |
| `[ids]` + `05ac:0256` alone | ok | UAF | UAF |
| `[ids]` + `05ac:0256` + `f3 = macro(M-z)` | ok | UAF | UAF |
| `[ids]` + `*` alone (generic configuration) | ok | UAF | UAF |
| `[ids]` + `004c:0267` (does not grab 05ac:0256) | ok | ok | ok |

Conclusion: no line is at fault — neither the ids with the double vendor 05ac/004c, nor the comments, nor the macros, nor the names `scale`/`dashboard`/`micmute`/`sleep`. The only factor is "keyd grabs a keyboard and receives a reload". `keyd check keyd/apple-keyboard.conf` (keyd 2.6.0): `No errors found`, no warning. There is therefore **no** configuration fix: the fix is about how the package touches keyd.

## 4. Fix applied in the repository

| File | Change |
|---|---|
| `apple-kb-monitor.install` | `_akm_keyd_reload` removed: no more `keyd reload` or `systemctl … keyd`; `post_upgrade` from < 3.1.0-10 shows a notice (keyd optional, how to keep it, "restart, never reload"); install message updated |
| `PKGBUILD`, `.SRCINFO` | `keyd` removed from `depends`, added to `optdepends`; `etc/keyd/apple-keyboard.conf` removed from `backup=`; the configuration is installed as `/usr/share/doc/apple-kb-monitor/examples/keyd/apple-keyboard.conf`, this document as `/usr/share/doc/apple-kb-monitor/KEYD.md` |
| `keyd/apple-keyboard.conf` | content unchanged (the `led.rs` and `test_model_inputs.py` tests still read it); "optional example" header, activation procedure, `keyd reload` warning |
| `scripts/package-expected.txt` | `etc/keyd/apple-keyboard.conf` replaced by the two files in `/usr/share/doc` |
| `tests/keyd/check-config.sh` | `keyd check` on the shipped file + package safeguards (no executable `keyd reload`/`systemctl … keyd` in the `.install`, keyd absent from `depends`, nothing installed in `/etc/keyd`); `--harness` also runs the ASAN harness. Fails on `main` (5 findings), passes on the branch |
| `tests/keyd/run-harness.sh`, `reload-harness.c`, `keyd-2.6.0-reload-active_kbd.patch` | ASAN harness of § 2 and upstream fix applied to the `patched` variant |

Effect on upgrade: an unmodified `/etc/keyd/apple-keyboard.conf` is removed by pacman (modified: kept as `.pacsave`). An already running keyd keeps the old table in memory until its next start; the scriptlet does not touch it.

**Merge order**: this branch removes the F3-F6 remapping active by default. It must be merged **with or after** the hwdb branch of `akmctl keymap` (`feat/touches-keymap`), otherwise F3-F6 fall back to the `hid-apple` media functions between the two versions. The hwdb/polkit/akmctl files are not touched here. The `apihub-app` diagnostic (`src/main.rs:662`) still shows "/etc/keyd/apple-keyboard.conf …": to be reworded when the hwdb is integrated (keyd absent = normal state).

### Line to add to `scripts/ci-local.sh` (not modified here)

Next to `step udev`:

```bash
s_keyd() { bash tests/keyd/check-config.sh --harness; }
step keyd       1 ""   -- s_keyd
```

(`--harness` without network or cache is skipped cleanly; for a quick pre-push: `bash tests/keyd/check-config.sh` without `--harness`, < 1 s.)

## 5. Recommendation: keyd optional

1. **Two stacked remappings contradict each other.** The `akmctl keymap` udev hwdb changes the codes at kernel level (scancode → keycode); keyd reads these already remapped codes and remaps them a second time. Leaving `/etc/keyd/apple-keyboard.conf` active by default guarantees a double remapping as soon as the hwdb arrives.
2. **keyd is a single point of failure for the keyboard.** It grabs the keyboard exclusively (`EVIOCGRAB`): if it crashes, the keyboard becomes raw again; if it freezes, nothing gets through. The hwdb has no process.
3. **A package must not drive a third-party system daemon.** The only call to keyd (`keyd reload`) is precisely what triggered the crash; the defect is fixed in no released version of keyd.
4. **LEDs do not depend on it**: `akm-core/src/led.rs` reads `/etc/keyd` to know whether keyd grabs the keyboard; without a configuration it writes to the keyboard's own evdev (`LedTarget::AppleDirect`), a path already tested. Edge case: after the upgrade, a keyd still running with the old table still grabs the keyboard while `/etc/keyd` no longer lists it; `led.rs` then targets the grabbed evdev until the next stop/restart of keyd — hence the scriptlet notice.

The file is still provided as an example for anyone who wants keyd macros (layers, per-application macros, F27) instead of the hwdb.

## 6. Upstream bug report (ready, NOT submitted)

The defect is already tracked upstream ([#1320](https://github.com/rvaiya/keyd/issues/1320), fix [#1319](https://github.com/rvaiya/keyd/pull/1319)). The text below is therefore written as a **comment** for rvaiya/keyd#1320; it brings a non-chord path, with no pending timeout, triggered by an unmanaged mouse, and a device-free ASAN harness.

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
> **Device-free reproduction:** a harness that `#include`s `src/daemon.c`, stubs `device_grab()`/`evloop()`, uses `vkbd/stdout.c` and feeds `event_handler()` with fake `EV_DEV_ADD`/`EV_DEV_EVENT` events, built with `-fsanitize=address,undefined` against v2.6.0: `heap-use-after-free` in `handle_chord` (keyboard.c:940) ← `process_keypress` (daemon.c:484) ← `event_handler` (daemon.c:571), "738464 bytes inside of 741648-byte region" freed by `free_configs()`. The `EV_TIMEOUT` path (daemon.c:509) fails the same way. With `active_kbd = NULL` at the start of `reload()` — equivalent to rvaiya/keyd#1319 — all scenarios pass under ASAN. Harness (~120 lines of C) available on request.
>
> **Impact:** any package that runs `keyd reload` from an install hook (ours did) can take keyd down on a desktop where a mouse is clicked within seconds; `keyd.service` has no `Restart=`, so the keyboard stays unmapped until a manual restart.
>
> **Suggested fix:** merge rvaiya/keyd#1319 (clear `active_kbd` in `free_configs()`), and possibly `Restart=on-failure` in `keyd.service.in`.

## 7. What the author must do

- **Recommended, as soon as the `akmctl keymap` hwdb is installed**: leave keyd stopped and disable it so it does not come back at the next boot (it is currently `enabled` + `failed`): `sudo systemctl disable keyd`. The keyboard then goes through the hwdb alone.
- **Before the hwdb** (current package 3.1.0-9), to get F3-F6 back right away: `sudo systemctl restart keyd` — risk-free: the defect only occurs after a `keyd reload`, never at startup. Never run `keyd reload` (nor a package ≤ 3.1.0-9 that does it) while keyd 2.6.0 is installed.
- keyd can be uninstalled (`sudo pacman -Rs keyd`) once the package built from this branch is installed, if nothing else on the computer uses it.
