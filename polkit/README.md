# polkit — a single privileged program

A single root executable, `/usr/lib/apple-kb-monitor/akm-helper`, started by `pkexec`. Its first argument
is the command AND selects the polkit action (annotation `org.freedesktop.policykit.exec.argv1`, polkit 127):

| Command (`argv[1]`) | Action | Default for the active session |
|---|---|---|
| `set-fnmode` | `…set-fnmode` | `auth_admin` |
| `install-keymap` | `…install-keymap` | `auth_admin` |
| `hid-control` | `…hid-control` | `auth_admin` |
| `hid-inspect` | `…hid-inspect` | `yes` (read-only, connected Apple keyboards only) |
| `doctor-fix` | `…doctor-fix` | `auth_admin` |

Every action carries an `argv1`: `pkexec` picks exactly one per command; any other `argv1` falls back
to `org.freedesktop.policykit.exec` (`auth_admin`) and the helper refuses it (code 64). Descriptions and messages
are in English with translations in 19 languages (`xml:lang`). Proof on a private polkitd: `pkaction --verbose` shows `exec.path` +
`exec.argv1`, `pkcheck` returns 0 for `hid-inspect` and 2 (authentication) for the four others.

## Fn mode

`com.agenceapi.AppleKbMonitor.set-fnmode` allows `pkexec /usr/lib/apple-kb-monitor/akm-helper set-fnmode <0-4>|NAME=VALUE... [--persist]` (whitelist below).

- Default `allow_active=auth_admin` (no `_keep`): the `hid_apple.fnmode` parameter is **global to all Apple keyboards** of the machine and `--persist` writes into `/etc/modprobe.d`; an active local session must therefore authenticate as administrator at each change. No retention: otherwise the daemon (a long-lived process, the subject seen by polkit) would pass the authorisation on to any application of the session through D-Bus. Remote/inactive sessions are refused.
- Daemon side (`SetFnMode`): `/usr/bin/pkexec` and the helper are compile-time constants, argv `set-fnmode <n>`, uid of the D-Bus caller = uid of the daemon, **one dialog at a time** (then a 5 s pause), every request logged. `SetSwapOptCmd`/`SetIsoLayout` no longer exist (no polkit action).
- `49-apple-kb-monitor.rules.example`: optional rule (group `wheel` without password), not installed.
- For this command the helper accepts only a value 0..=4 or up to five whitelisted `NAME=VALUE` + `--persist`, writes only the sysfs files of those parameters under `/sys/module/hid_apple/parameters/` and `/etc/modprobe.d/hid_apple.conf` (only the given tokens are rewritten, atomic rename).

## Helper security

`pkexec` does not change the umask: the helper inherits it from an unprivileged caller. `akm-helper` therefore forces `umask 077` as soon as it starts, creates the temporary file with `O_EXCL|O_NOFOLLOW` mode 0600, then explicitly sets `0644 root:root` (chmod/chown, independent of the umask) before the atomic `rename`; a symbolic link as destination is refused. `/etc/modprobe.d/hid_apple.conf` can thus never become writable by someone else (which would allow `install hid_apple <script>` run as root). Tests: `cargo test -p akm-helper` (reproduces the old defect under `umask 000`).

`akmctl` calls `/usr/bin/pkexec` by absolute path and the helper at the compiled path `/usr/lib/apple-kb-monitor/akm-helper`: no environment variable (the former `AKM_HELPER` no longer exists) selects the program run through pkexec.

`--persist` writes the configuration even if `hid_apple` is not loaded (message "applied at the next module load", code 0).

## Key mapping

`com.agenceapi.AppleKbMonitor.install-keymap` allows `pkexec /usr/lib/apple-kb-monitor/akm-helper install-keymap install|remove|rollback` (former rationale: one program per action, pkexec choosing the action from the program path). No path and no value as argument: the helper reads the single file `/run/user/$PKEXEC_UID/apple-kb-monitor/keymap.hwdb` (regular, `O_NOFOLLOW`, owner = the caller, a single link, not writable by group/others, 16 KiB max), validates it line by line (apple-kb-monitor header, `evdev:input:b0005v05ACpPPPP*` lines for the wireless aluminium PIDs only, ` KEYBOARD_KEY_<usage>=<name>` with a whitelisted usage — keyboard page 0x07, Eject 0xc00b8, Fn 0xff0003 — and a kernel `KEY_*` name; everything else is refused), **rewrites it in canonical form**, backs up the previous one as `90-apple-kb-monitor.hwdb.akm-bak`, writes `/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb` as 0644 root:root (temporary `O_EXCL|O_NOFOLLOW`, atomic rename), then runs `/usr/bin/systemd-hwdb update` and `/usr/bin/udevadm trigger --settle --subsystem-match=input --action=change` (empty environment). `remove`/`rollback` first rewrite the default codes of the remapped keys (`EVIOCSKEYCODE` survives the removal of the file until the keyboard reconnects). No exclusive grab and no uinput.

`set-fnmode` also covers `akm-helper set-fnmode NAME=VALUE... [--persist]`: whitelist of the parameters that `modinfo hid_apple` lists on 7.x (`fnmode` 0-4, `iso_layout` -1..1, `swap_opt_cmd` 0-2, `swap_ctrl_cmd` 0-1, `swap_fn_leftctrl` 0-1; `rightalt_as_rightctrl` and `ejectcd_as_delete` do not exist in this driver).

The helper stays dependency-free (libc): the whitelists are source files of `akm-core` (`keymap.rs`, `keycodes.rs`) compiled by path.

## Diagnostic fixes

`com.agenceapi.AppleKbMonitor.doctor-fix` allows `pkexec /usr/lib/apple-kb-monitor/akm-helper doctor-fix <bluez-conf|upower-conf|adapter-autosuspend>... [--restart] [--dry-run]`, started by `akmctl doctor --fix`. A separate program, hence a separate action; `auth_admin` without `_keep`, refused outside an active local session.

Up to and including 3.1.0-26, this action **did not exist** for polkitd: a double hyphen in an XML comment of the file made it invalid (`xmllint`: "Double hyphen within comment"), and polkitd then ignores the rest of the file; `pkaction` listed only 4 actions and `pkexec akm-doctor-fix` fell back to the generic action `org.freedesktop.policykit.exec`. Safeguard: `tests/check-data-files.sh` (xmllint + polkit DTD, expat parsing, parsed actions = declared actions) and the check of the built package; check on a computer: `pkaction --action-id com.agenceapi.AppleKbMonitor.doctor-fix`. No double hyphen in any comment of this file.

- Closed list: three identifiers and two switches, in a fixed order. No path, no key, no value as argument; no environment variable, no standard input, no shell.
- `bluez-conf` can only write `FastConnectable = true` in `[General]` and `ReconnectUUIDs` / `ReconnectAttempts` / `ReconnectIntervals` in `[Policy]` of `/etc/bluetooth/main.conf` (removed from the other sections, where bluetoothd ignores them). `upower-conf`: `NoPollBatteries = true` in `/etc/UPower/UPower.conf`. `adapter-autosuspend`: `/etc/udev/rules.d/61-akm-bt-adapter-no-autosuspend.rules`, whose content is compiled into the program, then `udevadm control --reload` and `udevadm trigger` on the adapter `8087:0026`.
- The other lines, the comments and the order of the file are kept. Atomic rewrite (`0644 root:root`, symbolic link refused), previous content kept in `<name>.akm-bak`, and only if the content changes: running a fix twice changes nothing the second time, the backup keeps the original.
- `--restart` (`akmctl doctor --fix --restart-services`) runs `/usr/bin/systemctl restart bluetooth.service` or `upower.service`, fixed arguments, empty environment. Never done without this switch: restarting bluetooth cuts every Bluetooth link for a few seconds.
- `--dry-run` writes nothing and runs nothing; `akmctl` runs it without `pkexec`, hence without password (the files read are world-readable).
- What the assistant does not do stays a displayed advice: pairing, BlueZ trust, disk space, user services.
