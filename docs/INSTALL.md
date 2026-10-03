# Installation guide

Package `apple-kb-monitor` **3.1.0-26**, Arch Linux / Manjaro, x86_64. Everything in this file was checked against `PKGBUILD`, `apple-kb-monitor.install` and `pacman -Ql apple-kb-monitor` (88 files) on 2026-10-03.

## Prerequisites

- Arch Linux or Manjaro (the PKGBUILD builds from the local tree; nothing is downloaded).
- A Bluetooth adapter supported by BlueZ, the keyboard already paired (`bluetoothctl` or the Plasma Bluetooth applet).
- KDE Plasma 6 for the widget, the System Settings module and the notification events. The daemon and `akmctl` work without Plasma.
- Build: `rust`, `gcc`, `gettext`, `cmake`, `extra-cmake-modules`, `git` (`makedepends`).

Runtime dependencies (`depends`): `bluez`, `polkit`, `dbus`, `systemd`, `libcap`, the Wayland / X11 / GL libraries of the egui window (`wayland`, `libxkbcommon`, `libxkbcommon-x11`, `libglvnd`, `libx11`, `libxcursor`, `libxi`, `libxrandr`), and the Qt / KDE Frameworks of the System Settings module (`qt6-base`, `qt6-declarative`, `kcmutils`, `ki18n`, `kcoreaddons`, `kirigami`).

Optional (`optdepends`): `bluez-utils` (`bluetoothctl`), `libnotify` (`notify-send` fallback), `keyd` (optional F3-F6 macros; see below).

## Build and install

```bash
git clone https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor.git
cd apple-kb-monitor
makepkg -si
```

**A package is one commit** ([#295](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/295)): `prepare()` stops the build when the tree is not a git work tree, or has a modified or untracked file (`git status --porcelain`; makepkg's own `src/` and `pkg/` aside). On 2026-10-03 two different packages were installed under the same 3.1.0-26, the second built from uncommitted changes, and no commit said what code was running. Commit first, and give every rebuild that is installed a new `pkgrel`. `AKM_ALLOW_DIRTY=1 makepkg …` lifts the guard on purpose (local test only; `scripts/ci-local.sh` sets it for the copy of the working tree it packages).

The build is locked (#14): `prepare()` runs `cargo fetch --locked` and `build()` runs `cargo build --locked`, so the crates are exactly those of `apihub-app/Cargo.lock`, each verified by cargo against the checksum recorded there; a `Cargo.lock` that no longer matches a `Cargo.toml` fails the build instead of being rewritten. The only `source=()` entry, the `.desktop` file, has its real `sha256sums` (after editing it: `updpkgsums && makepkg --printsrcinfo > .SRCINFO`). `tests/check-pkgbuild.sh` checks all of this, that `.SRCINFO` is what `makepkg --printsrcinfo` gives, and the clean-tree guard on throw-away repositories.

Not done, and why: the `PKGBUILD` lives inside the source tree and builds it from `$startdir`; there is no published source archive (the forge is internal and serves no release tarball or signed tag). So there is no `source=("git+…#tag=…")` with a checksum, no build confined to `$srcdir`, and therefore no clean-chroot build (`extra-x86_64-build` does not carry `$startdir`). These three come together the day a source repository is published.

`makepkg` compiles the Rust workspace (`apihub-app`, `apple-kb-monitord`, `akmctl`, `akm-helper`, `akm-keymap-helper`, `akm-hid-control`, `akm-hid-inspect`, `akm-doctor-fix`), the C helper `rssi-helper` and the C++/QML System Settings module, then installs (the exact list of the 88 files is `scripts/package-expected.txt`, checked both ways by the CI):

| Path | Content |
|---|---|
| `/usr/bin/` | `akmctl`, `apihub-app`, `apple-kb-monitord` |
| `/usr/lib/apple-kb-monitor/` | `rssi-helper`, `akm-helper`, `akm-keymap-helper`, `akm-hid-control`, `akm-hid-inspect`, `akm-doctor-fix` |
| `/usr/lib/systemd/user/` | `apple-kb-monitord.service`, `apple-kb-monitor-shutdown.service`, `apple-kb-monitor-selfcheck.{service,timer}` (+ `default.target.wants`, `timers.target.wants` symlinks) |
| `/usr/lib/systemd/system/` | `apple-kb-monitor-suspend.service`, `apple-kb-monitor-resume.service` (+ `sleep.target.wants`, `suspend/hibernate/hybrid-sleep/suspend-then-hibernate.target.wants`) |
| `/etc/apple-kb-monitor/hid-suspend.conf`, `/etc/modprobe.d/hid_apple.conf` | `enabled = false` (SUSPEND / EXIT_SUSPEND off by default); `fnmode=1` (both in `backup=`: your edits survive upgrades) |
| `/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules` | `uaccess` tag on the hidraw node of the 17 Bluetooth product ids of Apple keyboards (see [udev/README.md](../udev/README.md): a file of the same name in `/etc/udev/rules.d` replaces it) |
| `/usr/lib/sysusers.d/apple-kb-monitor.conf` | group `akm` |
| `/usr/share/polkit-1/actions/com.agenceapi.AppleKbMonitor.policy` | actions `set-fnmode`, `install-keymap`, `hid-control`, `hid-inspect`, `doctor-fix` |
| `/usr/share/dbus-1/services/` | `com.agenceapi.AppleKbMonitor1.service` (daemon), `com.agenceapi.AppleKbMonitor.service` (window), `com.agenceapi.AppleKbMonitor.Runner.service` (KRunner runner, `apihub-app --krunner`) |
| `/usr/share/krunner/dbusplugins/plasma-runner-applekeyboard.desktop` | KRunner plugin ("clavier", "keyboard") |
| `/usr/share/kglobalaccel/com.agenceapi.AppleKbMonitor.shortcuts.desktop` | global shortcuts component "Apple Keyboard" (no key bound by default) |
| `/usr/share/applications/` | `com.agenceapi.AppleKbMonitor.desktop`, `kcm_applekeyboard.desktop` |
| `/usr/lib/qt6/plugins/plasma/kcms/systemsettings/kcm_applekeyboard.so` | System Settings module |
| `/usr/share/plasma/plasmoids/com.agenceapi.devicehub/` | Plasma widget "ApiHub" (`metadata.json` + 10 files in `contents/ui/`) |
| `/usr/share/knotifications6/apple-kb-monitor.notifyrc` | notification events (18) |
| `/usr/share/icons/hicolor/scalable/{apps,status}/` | application icon, 25 tray icons by battery level |
| `/usr/share/{bash-completion,zsh,fish}/…`, `/usr/share/man/man1/akmctl.1.gz` | completions and manual, generated by `akmctl` itself at build time |
| `/usr/share/locale/fr/LC_MESSAGES/` | French catalogues of the widget and of the module |
| `/usr/share/doc/apple-kb-monitor/` | `KCM.md`, `KEYD.md`, `QA-AUTOMATIQUE.md`, `TOUCHES.md`, `VEILLE-HID.md`, `udev-README.md`, `examples/keyd/apple-keyboard.conf` |
| `/usr/share/licenses/apple-kb-monitor/OFL.txt` | licence of the VT323 font embedded in the window (the program itself: GPL-2.0-or-later, a common licence on Arch) |

### What `post_install` does

1. `udevadm control --reload-rules` and `udevadm trigger --subsystem-match=hidraw`: the `uaccess` ACL is applied to a keyboard that is already connected.
2. On a first install only, writes `1` to `/sys/module/hid_apple/parameters/fnmode` (the packaged default). Upgrades never touch the mode you chose with `akmctl set fnmode`.
3. Creates the group `akm` (`systemd-sysusers`), sets `rssi-helper` to `root:akm 0750`, then `setcap cap_net_admin+ep` on it. A warning is printed if either step fails, with the command to run by hand. It adds **nobody** to the group (see below).
4. Enables **nothing** by itself (#260): the units are enabled by the `*.target.wants/` links the package ships under `/usr/lib/systemd/{system,user}` (sleep/wake units for the system; daemon, shutdown notice and self-check timer for every user), which leave with the package. It removes the duplicate links an older scriptlet (up to 3.1.0-16) wrote under `/etc/systemd` and the orphaned `mqtt-bridge.py` leftovers of `/usr/lib/apple-kb-monitor` (only if no package owns them); `pre_remove` drops any `/etc/systemd` link to the units before they go. Opt out of any unit with `systemctl [--user] mask <unit>`.
5. keyd is **never** reloaded nor restarted (`keyd reload` crashes keyd 2.6.0, [#246](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/246)).
6. Reads `/proc` (nothing else) and prints, per user that has a running systemd user manager, what is still to do for the group `akm`, with the exact command; warns when `/etc/udev/rules.d/70-apple-kb-hidraw.rules` exists and differs from the packaged rule ([#296](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/296)). It restarts nothing in a user's session.

## After the install

As each user of the keyboard (no sudo), in the session already open:

```bash
systemctl --user start apple-kb-monitord.service apple-kb-monitor-selfcheck.timer   # now, not at the next login
systemctl --user restart plasma-plasmashell.service   # Plasma learns of the ApiHub widget
systemctl --user restart plasma-krunner.service       # KRunner learns of the "clavier" runner
akmctl doctor                    # adapter, pairing, link, hidraw, adapter power management,
                                 # BlueZ / UPower configuration, journal, daemon
akmctl status
```

pacman's own hook already reloaded the user managers (`systemctl reload 'user@*.service'`), so no `daemon-reload` is needed. Without the start, the daemon starts at the next login (`default.target`), or on demand through D-Bus activation when a client (`apihub-app`, the widget, the System Settings module) calls `com.agenceapi.AppleKbMonitor1`.

Then:

- **Widget as the notification-area icon**: notification area → Configure → Entries → ApiHub (Plasma does not enable it by itself). Until then the icon is the daemon's own. See [INTEGRATION-KDE.md](INTEGRATION-KDE.md) §2.
- **Global shortcuts** ("Apple Keyboard" in System Settings → Shortcuts): kglobalaccel reads `/usr/share/kglobalaccel` when the Plasma session starts; log out and back in once.

### Signal strength (RSSI): the group `akm`

`rssi-helper` carries `cap_net_admin` and only members of `akm` may run it (#209): it can read the RSSI / presence of every connected Bluetooth device. The daemon runs it as you, so **the daemon** must have the group. The package adds nobody: which accounts may use such a helper is the administrator's decision, not "whoever is logged in while pacman runs". The scriptlet prints, per user, the exact command; by hand:

```bash
sudo usermod -aG akm "$USER"
```

When it takes effect: the daemon is a service of your systemd user manager (`user@UID.service`) and gets **its** groups, fixed when that manager started.

- Without linger (`loginctl show-user "$USER" -p Linger` → `Linger=no`): close every session (Plasma, SSH, tmux…) and log in again, or reboot.
- With `Linger=yes`, logging out is **not** enough: the manager keeps running. `systemctl --user daemon-reexec` does not help either (the manager re-executes itself in the same process, with the same groups). Reboot, or: log out of Plasma, log in on a text console (Ctrl+Alt+F3), run `sudo systemctl restart user@$(id -u).service`, log out of the console and log in to Plasma again. Measured on 2026-10-03: user in `akm` since the morning, manager started on 10-01 (`Linger=yes`, already re-executed), `Groups:` of the manager and of the daemon without the GID of `akm`.

Check what the daemon really has: `grep Groups /proc/$(pgrep -u "$USER" -x apple-kb-monitord)/status` must contain the GID of `getent group akm`. (`id -nG` in a terminal is not enough: a new terminal can have the group while the daemon does not.)

### Permissions, summarised

| Need | Mechanism | Check |
|---|---|---|
| Read the keyboard (hidraw) | udev `uaccess` → logind ACL for the user of the **active seat** (an SSH session has no ACL) | `getfacl /dev/hidraw* \| grep "user:$USER"` or `akmctl doctor` (line `hidraw`) |
| RSSI / TX power | `rssi-helper`, `cap_net_admin+ep`, `root:akm 0750` → membership of `akm`, effective in the daemon | `Groups:` of the daemon (above); `getcap /usr/lib/apple-kb-monitor/rssi-helper` |
| Fn mode, `hid_apple` parameters, `/etc/modprobe.d` | `pkexec akm-helper`, polkit `auth_admin` | `akmctl set fnmode N` opens the dialog |
| Key mapping (udev hwdb) | `pkexec akm-keymap-helper` | `akmctl keymap apply` |
| Corrections of the diagnostic: BlueZ `main.conf` (`FastConnectable`, `[Policy] Reconnect*`), `UPower.conf` (`NoPollBatteries`), udev rule keeping the adapter `8087:0026` out of USB autosuspend | `pkexec akm-doctor-fix`, polkit `doctor-fix`, `auth_admin`; closed list, previous file kept as `<name>.akm-bak` | `akmctl doctor --fix` (`--dry-run` shows the changes without a password; `--restart-services` also restarts bluetooth / upower; `--optional` adds the UPower one); `pkaction --action-id com.agenceapi.AppleKbMonitor.doctor-fix` must answer (up to 3.1.0-26 the policy file did not parse and this action did not exist, [#292](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/292)) |
| Sleep / wake HID_CONTROL byte | system units as root; `pkexec akm-hid-control` for the manual test | `akmctl hid-control suspend --dry-run` |

### Recommended BlueZ / UPower settings (administrator, optional)

`akmctl doctor` reports them; the repository ships a tool that prints or applies them:

```bash
python3 bluetooth/akm-conf.py bluez            # shows the [General] FastConnectable / [Policy] Reconnect* changes
sudo python3 bluetooth/akm-conf.py bluez --apply
sudo python3 bluetooth/akm-conf.py upower --apply   # NoPollBatteries = true: UPower stops polling the keyboard every 30 s
```

For an Intel `8087:0026` adapter, `udev/61-akm-bt-adapter-no-autosuspend.rules` keeps the adapter out of USB autosuspend; it is **not** installed by the package, copy it to `/etc/udev/rules.d/` yourself. Rationale and measurements: [RECONNEXION-PAIRAGE.md](RECONNEXION-PAIRAGE.md) §5.1.

### keyd (optional)

The kernel (`hid_apple fnmode=1`) already sends brightness, Exposé, media and volume keys. keyd is only needed for the F3-F6 macros of the example config:

```bash
sudo install -Dm644 /usr/share/doc/apple-kb-monitor/examples/keyd/apple-keyboard.conf /etc/keyd/
sudo systemctl restart keyd        # never `keyd reload`
```

Without keyd, remap keys with `akmctl keymap` (udev hwdb, [TOUCHES.md](TOUCHES.md) §5). Why keyd became optional: [KEYD.md](KEYD.md).

## KDE pieces

- **System Settings → Input Devices → Keyboard → Apple Keyboard** appears as soon as the package is installed (`X-KDE-System-Settings-Parent-Category: keyboard`); restart `systemsettings` if it was open. [KCM.md](KCM.md)
- **Plasma widget** "ApiHub" (`com.agenceapi.devicehub`): it is a notification-area entry (`X-Plasma-NotificationArea`) and, once enabled there (notification area → Configure → Entries), it **is** the keyboard's icon: left click = panel anchored to the icon, opened and closed like the sound one; right click = the daemon's full menu. The daemon then withdraws its own icon: one icon on screen ([INTEGRATION-KDE.md](INTEGRATION-KDE.md) §2). It can also be placed on a panel or the desktop from the widget browser. To install from the tree instead of the package: `kpackagetool6 -t Plasma/Applet -i plasma/com.agenceapi.devicehub` (`-u` to update).
- **Notifications**: System Settings → Notifications → Application Settings → Apple Keyboard Monitor. [NOTIFICATIONS.md](NOTIFICATIONS.md)
- **Window**: `apihub-app`, or "Apple Keyboard Monitor" ("Moniteur de clavier Apple") in the application menu, the tray icon's menu, the widget or a notification button; single instance, no autostart.
- **Global shortcuts and KRunner**: read when the Plasma session starts (shortcuts) or when krunner starts (runner); see "After the install".

## Upgrade

Commit, raise `pkgrel`, then `makepkg -si` again. `post_upgrade` reloads udev, re-applies the group and the capability on `rssi-helper`, drops `/etc/systemd` links and orphans like `post_install`, and enables nothing (the package's `.wants` links do). At **every** upgrade it then prints, per user, only what applies:

- what still runs the previous version, read in `/proc` (pacman replaced the files, the processes keep the old ones): the daemon (`/usr/bin/apple-kb-monitord (deleted)`) → `systemctl --user restart apple-kb-monitord.service`; `apihub-app` (window or KRunner runner, `(deleted)`) → `pkill -x apihub-app`, then reopen the window; a running plasmashell, which keeps the widget it loaded at its start → `systemctl --user restart plasma-plasmashell.service`; plus the reminder that global shortcuts and the KRunner plugin are read at session start ([#294](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/294));
- users not yet in `akm`, or in it but whose daemon does not have it yet (see above, linger included);
- a `/etc/udev/rules.d/70-apple-kb-hidraw.rules` that hides the packaged rule.

Nothing is restarted by the scriptlet: a user's session is not root's to restart. Notices printed once, when crossing these versions:

| From before | Notice |
|---|---|
| 3.1.0-6 | the former `apple-kb-monitor` command and `apple-kb-monitor.service` are gone: `akmctl` replaces them; `systemctl --user disable apple-kb-monitor.service` if you had enabled it; `akmctl history import` keeps its volatile history |
| 3.1.0-10 | keyd optional: `/etc/keyd/apple-keyboard.conf` is no longer installed (a `.pacsave` is kept if you edited it); the example moved to `/usr/share/doc` |
| 3.1.0-11 | KDE hint for F4 / Eject (`akmctl keymap kde-apply`) |
| 3.1.0-20 | sleep / wake HID_CONTROL bytes ([#244](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/244)) off by default ([#264](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/264)); warning if your kept `/etc/apple-kb-monitor/hid-suspend.conf` still says `enabled = true` |

The group `akm` of `rssi-helper` (3.1.0-7, [#209](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/209)) is no longer a one-time notice: it is checked at every upgrade, as above.

The self-check's `versions` line compares the daemon's version with `pacman -Q` **without** the `pkgrel` (3.1.0 = 3.1.0): it does not see a daemon or a window left on the previous release of the same version. Rely on the upgrade notice, or `ls -l /proc/$(pgrep -x apple-kb-monitord)/exe` (`(deleted)` = old binary).

Equivalents of the removed command: `--once/--status/--json` → `akmctl status [--json]`, `--watch` → `akmctl watch`, `--history` → `akmctl history`, `--export-csv` → `akmctl history export --csv`, `--graph` → `akmctl graph`, `--waybar` → `akmctl waybar`, `--metrics` → `akmctl metrics`, `--led caps on` → `akmctl led caps on`, `--dump` → `akmctl dump` (3 safe reports only). The history lives in one file, `$XDG_STATE_HOME/apple-kb-monitor/history.jsonl`, written by the daemon only.

## Uninstall

```bash
sudo pacman -R apple-kb-monitor
```

`pre_remove` drops the `/etc/systemd` links to the units. `post_remove` reloads udev and prints what it leaves behind: a key mapping installed with `akmctl keymap apply` (`sudo rm /etc/udev/hwdb.d/90-apple-kb-monitor.hwdb* && sudo systemd-hwdb update`, then switch the keyboard off and on); links a user made with `systemctl --user enable` under `~/.config/systemd/user/*.wants/`, now dangling, listed per user with the exact `rm` ([#298](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/298)); the per-user state, kept (`~/.config/apple-kb-monitor/`, `~/.local/state/apple-kb-monitor/`). A daemon still running keeps running until its user manager stops: at logout, **never** with `Linger=yes`. Stop it now, as the user: `systemctl --user stop apple-kb-monitord.service`.

## Troubleshooting

See [TROUBLESHOOTING.md](TROUBLESHOOTING.md). The install-time checks: `akmctl doctor`, `getcap /usr/lib/apple-kb-monitor/rssi-helper`, `pkaction | grep AppleKbMonitor` (five actions), `systemctl --user list-unit-files 'apple-kb-monitor*'` (user units `enabled` or `static`). The system units `apple-kb-monitor-{suspend,resume}.service` are enabled by the package's links under `/usr/lib/systemd/system/*.target.wants/`, which `systemctl is-enabled` does not count: it answers `disabled` although they run at sleep / wake; check with `systemctl list-dependencies sleep.target`. On a machine whose sleep targets are masked (`systemctl show -p LoadState sleep.target` → `masked`) they never run.
