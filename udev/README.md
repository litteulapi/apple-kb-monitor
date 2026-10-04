# udev - hidraw access to Apple keyboards

`70-apple-kb-hidraw.rules` sets the `uaccess` tag (logind rw ACL for the user of the active session) on the `/dev/hidraw*` of Apple **Bluetooth keyboards** only: the list of product ids is that of `APPLE_MODELS` (`apihub-app/akm-core/src/model.rs`). Mice, trackpads (Magic Mouse/Trackpad) and wired USB devices are no longer affected.

## Where the rule is, and what masks it

The package installs it in `/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules`. udev reads rules by **file name**: a file of the same name in `/etc/udev/rules.d/` (or `/run/udev/rules.d/`) **fully replaces** the package's one, without warning. Found on 2026-10-03: an `/etc/udev/rules.d/70-apple-kb-hidraw.rules` from an old installation (no owning package) gave the ACL to **every** Apple HID device (`KERNELS=="0005:05AC:*|0005:004C:*|0003:05AC:*"`: mice, trackpads, wired), and any fix of the packaged rule had no effect; neither `pacman -Qkk` nor the CI saw it.

* The package scriptlet reports it at installation and at each update when this file exists and differs from the packaged file; it does not delete it (it can be an intended override, for example the "dedicated group" variant below).
* Check which rule applies: `udevadm cat 70-apple-kb-hidraw.rules` (first line = path of the file used; checked on 2026-10-03: `# /usr/lib/udev/rules.d/70-apple-kb-hidraw.rules` once the orphan was removed); `pacman -Qo /etc/udev/rules.d/70-apple-kb-hidraw.rules` ("no package" = leftover of a manual installation).
* Remove an unintended override: `sudo rm /etc/udev/rules.d/70-apple-kb-hidraw.rules && sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=hidraw`.
* Override on purpose: copy the package's file into `/etc/udev/rules.d/` under the same name and adapt it; the scriptlet message will remind you at each update (the copy no longer follows the package).

## Accepted trade-off: keylogger risk

A `read()` on hidraw returns the keyboard's raw input reports, hence the keystrokes. With `uaccess`, **any process running as the user of the active session** can read the keystrokes of this keyboard, including outside the Wayland isolation model. The ACL also opens writing (output/feature reports: LEDs, etc.).

Reason for the choice: the `apple-kb-monitord` daemon is a **user** service (it reads the battery with GET_REPORT and drives the LEDs); it therefore needs access within the session without a group or privilege. The `input` group would give the same capability over a wider surface.

## Multi-user machine: an open descriptor that survives a session switch

logind recomputes the `uaccess` ACL when the active session changes, but **does not revoke already open descriptors** (`HIDIOCREVOKE` only applies to devices taken by `TakeDevice`). An account A that opened `/dev/hidrawN` (`cat /dev/hidrawN`) and then hands over to an account B (fast user switching) **keeps receiving B's input reports**, hence B's keystrokes and passwords. Level of evidence: analysis of the code and binaries, no live demonstration (neither a second account nor hardware during the audit).

Mitigations:

* **No udev-side mitigation is safe**: a udev rule can neither close another process's descriptor nor prevent the `open()` while A is active. `uaccess` therefore remains a valid trade-off for a **single-user** machine (the target case).
* Shared machine: prefer the "dedicated group" variant below, or disconnect the keyboard / close A's session (`loginctl terminate-session`, or `KillUserProcesses=yes` in `logind.conf`) before handing over. Fast user switching with a shared Apple Bluetooth keyboard is not supported.
* Daemon side: it keeps the node open for passive listening only while the keyboard is connected and never reads the content of the keystrokes (timestamps only). Closing the node when `login1.Session.Active` is lost is a lead, not a fix for A's process.

Hardened alternative (not provided, up to the administrator): replace `TAG+="uaccess"` with `GROUP="akm", MODE="0660"` and run the daemon under an account of the `akm` group (system systemd unit with `SupplementaryGroups=akm`). The user daemon (`apple-kb-monitord`) then loses hidraw access.

Check: `udevadm verify udev/70-apple-kb-hidraw.rules`; `getfacl /dev/hidrawN` on a connected keyboard (ACL present) and on an Apple mouse (ACL absent).

## Key mapping hwdb

No hwdb file is shipped by the package (default = kernel mapping). `akmctl keymap apply`
has `/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb` written by `akm-helper install-keymap` (entries
`evdev:input:b0005v05ACpPPPP*` + `KEYBOARD_KEY_<HID usage>=<name>`, allowlist), then
`systemd-hwdb update` and `udevadm trigger --subsystem-match=input --action=change`.
`akmctl keymap reset` removes it. Details: docs/KEYS.md.
