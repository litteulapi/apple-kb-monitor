# Installation guide

Package `apple-kb-monitor` (version: `PKGBUILD`), Arch Linux / Manjaro, x86_64. The
step-by-step tutorial is in the [README](../README.md#install-step-by-step);
this page is the reference behind it.

## Prerequisites

- Arch Linux or Manjaro, a Bluetooth adapter supported by BlueZ, the keyboard paired (Plasma Bluetooth settings or `bluetoothctl`).
- KDE Plasma 6 for the widget, the System Settings module and the notification events. The daemon and `akmctl` work without Plasma.
- Runtime dependencies are pulled by pacman (`depends` of the [PKGBUILD](../PKGBUILD)); building needs `rust gcc gettext cmake extra-cmake-modules git`.

## Three ways to install

| Way | Command | What you get |
|---|---|---|
| Release package | `curl -fsSL https://raw.githubusercontent.com/litteulapi/apple-kb-monitor/main/install.sh \| bash` | the `.pkg.tar.zst` of the latest [GitHub release](https://github.com/litteulapi/apple-kb-monitor/releases), sha256 checked, `pacman -U` ([install.sh](../install.sh)) |
| From the source tree | `git clone https://github.com/litteulapi/apple-kb-monitor.git && cd apple-kb-monitor && makepkg -si` | the same package, built from the commit you checked out |
| AUR, `-git` | [packaging/aur-git/](../packaging/aur-git/) (prepared, **not published** on the AUR) | builds the latest commit of the public repository; `provides`/`conflicts` `apple-kb-monitor` |

**A package is one commit**: `prepare()` stops when the tree is not a git work tree or has a modified or untracked file. Commit first and give every installed rebuild a new `pkgrel`; `AKM_ALLOW_DIRTY=1 makepkg …` lifts the guard for a local test. The build is locked: `cargo fetch --locked` / `cargo build --frozen`, the crates of `apihub-app/Cargo.lock` and nothing else. The installed files are listed in [scripts/package-expected.txt](../scripts/package-expected.txt), checked both ways by the CI.

### How releases are made

A tag `v<pkgver>` or `v<pkgver>-<pkgrel>` matching the PKGBUILD, pushed to GitHub, runs [.github/workflows/package.yml](../.github/workflows/package.yml): a fresh `archlinux:latest` container runs [scripts/release-build.sh](../scripts/release-build.sh) (dependencies of the PKGBUILD, clean clone built by an unprivileged user, `check-pkgbuild`, trademark, security-file, scriptlet and secret checks, `makepkg`, package content checked against `package-expected.txt`), then the package and its `.sha256` are attached to the release of that tag. Pushes to `main` and pull requests run the same build without publishing. The same script runs locally:

```bash
git clone . /path/to/clean-clone
podman run --rm -v /path/to/clean-clone:/src:ro -v "$PWD/dist":/out archlinux:latest \
    bash /src/scripts/release-build.sh /out
```

The AUR files are regenerated from the PKGBUILD by `scripts/gen-aur-git.py` (`--check` is part of `tests/check-pkgbuild.sh`).

## What the install does

1. `udevadm control --reload-rules` and `udevadm trigger --subsystem-match=hidraw`: the `uaccess` ACL is applied to a keyboard that is already connected.
2. Never writes sysfs: the packaged Fn mode (`fnmode=1`, `/etc/modprobe.d/hid_apple.conf`) applies when `hid_apple` loads next; now: `akmctl set fnmode 1`.
3. Starts, restarts and enables **no** unit (Arch packaging practice). No group, no `setcap`: `rssi-helper` comes with its capability inside the package.
4. The system sleep/wake units are enabled by the `*.target.wants/` links the package ships under `/usr/lib/systemd/system`. The user units (daemon, shutdown notice, self-check timer) are listed in the preset `/usr/lib/systemd/user-preset/90-apple-kb-monitor.preset`: each user enables them (`systemctl --user enable --now …`, printed by the scriptlet, or `systemctl --user preset …`), and `systemctl --user disable` opts out. It removes the duplicate system links an older scriptlet (up to 3.1.0-16) wrote under `/etc/systemd` and the orphaned `mqtt-bridge.py` leftovers of `/usr/lib/apple-kb-monitor` (only if no package owns them); `pre_remove` drops any `/etc/systemd` link to the units before they go.
5. keyd is **never** reloaded nor restarted (`keyd reload` crashes keyd 2.6.0).
6. Warns when `/etc/udev/rules.d/70-apple-kb-hidraw.rules` exists and differs from the packaged rule. It restarts nothing in a user's session.

## After the install

Enable the daemon once, as your user (no sudo):

```bash
systemctl --user enable --now apple-kb-monitord.service apple-kb-monitor-shutdown.service apple-kb-monitor-selfcheck.timer
```

Plasma reads the widget, the global shortcuts ("Apple Keyboard" in System Settings → Shortcuts) when the session starts: they appear at the next login (or now: `systemctl --user restart plasma-plasmashell.service`).

```bash
akmctl doctor                    # adapter, pairing, link, hidraw, adapter power management,
                                 # BlueZ / UPower configuration, journal, daemon
akmctl status
```

- **Widget in the notification area**: if Plasma does not show it, notification area → Configure → Entries → ApiHub. See [INTEGRATION-KDE.md](INTEGRATION-KDE.md) §2.
- **Signal strength (RSSI)**: no group. `rssi-helper` carries `cap_net_admin`, is runnable by anyone and answers **only** for a connected Apple keyboard (its address must be the `HID_UNIQ` of a `/sys/bus/hid/devices/0005:05AC|004C:<keyboard>` entry, same list as the udev rule): other devices (phones, headsets) are refused before the MGMT socket is opened. The group `akm` of 3.1.0-7 to 3.1.0-29 is no longer used: `sudo groupdel akm` if you wish.

### Permissions, summarised

| Need | Mechanism | Check |
|---|---|---|
| Read the keyboard (hidraw) | udev `uaccess` → logind ACL for the user of the **active seat** (an SSH session has no ACL) | `getfacl /dev/hidraw* \| grep "user:$USER"` or `akmctl doctor` (line `hidraw`) |
| RSSI / TX power | `rssi-helper`, `cap_net_admin+ep`, `root:root 0755`, Apple keyboards only | `Groups:` of the daemon (above); `getcap /usr/lib/apple-kb-monitor/rssi-helper` |
| Fn mode, `hid_apple` parameters, `/etc/modprobe.d` | `pkexec akm-helper`, polkit `auth_admin` | `akmctl set fnmode N` opens the dialog |
| Key mapping (udev hwdb) | `pkexec akm-helper install-keymap` | `akmctl keymap apply` |
| Corrections of the diagnostic: BlueZ `main.conf` (`FastConnectable`, `[Policy] Reconnect*`), `UPower.conf` (`NoPollBatteries`), udev rule keeping the adapter `8087:0026` out of USB autosuspend | `pkexec akm-helper doctor-fix`, polkit `doctor-fix`, `auth_admin`; closed list, previous file kept as `<name>.akm-bak` | `akmctl doctor --fix` (`--dry-run` shows the changes without a password; `--restart-services` also restarts bluetooth / upower; `--optional` adds the UPower one); `pkaction --action-id com.agenceapi.AppleKbMonitor.doctor-fix` must answer (up to 3.1.0-26 the policy file did not parse and this action did not exist) |
| Sleep / wake HID_CONTROL byte | system units as root; `pkexec akm-helper hid-control` for the manual test | `akmctl hid-control suspend --dry-run` |

### keyd (optional)

The kernel (`hid_apple fnmode=1`) already sends brightness, Exposé, media and volume. keyd is only for the F3-F6 macros of the example:

```bash
sudo install -Dm644 /usr/share/doc/apple-kb-monitor/examples/keyd/apple-keyboard.conf /etc/keyd/
sudo systemctl restart keyd        # never `keyd reload`
```

Without keyd, remap with `akmctl keymap` ([KEYS.md](KEYS.md) §5, [KEYD.md](KEYD.md)).

## Upgrade

Install the new package (release, or commit + new `pkgrel` + `makepkg -si`). `post_upgrade` does what `post_install` does, enables and restarts nothing (Arch practice: `systemctl --user try-restart apple-kb-monitord.service` loads the new binary), prints the enable command once when upgrading from a version older than 3.3.0 (whose package enabled the user units by itself), and prints per user only what applies: processes still running the replaced binaries (`systemctl --user restart apple-kb-monitord.service`, `systemctl --user restart plasma-plasmashell.service`), the group `akm`, a local udev rule hiding the packaged one, and one-time notices when crossing 3.1.0-6, -10, -11 and -20 (old `apple-kb-monitor` command replaced by `akmctl`, keyd made optional, F4 / Eject hint, sleep / wake bytes off by default). `akmctl selftest` and `akmctl doctor` compare the running version with `pacman -Q`, release included.

## Uninstall

```bash
sudo pacman -R apple-kb-monitor
systemctl --user stop apple-kb-monitord.service    # a running daemon outlives the package until logout
```

`post_remove` reloads udev and lists what it leaves: a key mapping installed by `akmctl keymap apply` (`sudo rm /etc/udev/hwdb.d/90-apple-kb-monitor.hwdb* && sudo systemd-hwdb update`), user `systemctl --user enable` links now dangling, and your data (`~/.config/apple-kb-monitor/`, `~/.local/state/apple-kb-monitor/`), kept.

## Troubleshooting

[TROUBLESHOOTING.md](TROUBLESHOOTING.md). Install-time checks: `akmctl doctor`, `getcap /usr/lib/apple-kb-monitor/rssi-helper`, `pkaction | grep AppleKbMonitor` (five actions), `systemctl --user list-unit-files 'apple-kb-monitor*'`. The system sleep / wake units are enabled by links under `/usr/lib/systemd/system/*.target.wants/`, which `systemctl is-enabled` reports as `disabled`: check with `systemctl list-dependencies sleep.target`.
