# KDE Plasma 6 integration: fixes from the 2026-10-01 audit

Follow-up of a review of the KDE Plasma 6 integration. Each
section says what the daemon does, how to check it and what is deliberately
left undone.

## 1. "Forget" from Plasma

A keyboard removed from outside (Settings › Bluetooth › Forget, Bluetooth
applet, `bluetoothctl remove`) disappears from the daemon **immediately**:

* the daemon listens to BlueZ's `org.freedesktop.DBus.ObjectManager.InterfacesRemoved` /
  `InterfacesAdded` (`repair.rs`, `add_rules`);
* on an `InterfacesRemoved` carrying `org.bluez.Device1`: the entry, its
  scheduled reconnections and its `Link.Status()` status disappear, acquisition
  is reconciled, and **one** `KeyboardRemoved` notification is sent:
  "Keyboard removed from this computer: turn it off and on again to pair it again",
  with the **Repair…** (`akmctl repair` in a terminal) and **Open** buttons;
* BlueZ publishes `Paired=false` just before removing the object: that `Paired=false` is
  only believed after `BOND_GRACE` (1.5 s) without removal. A real pairing loss
  (the object stays) still raises "re-pairing needed"; a removal never
  raises it;
* an `InterfacesAdded` of a `Device1` (re-pairing) triggers a re-enumeration.

Test: `apple-kb-monitord/tests/bluez_forget.rs` (private bus, fake `org.bluez`,
fake notification server) and the `repair::tests::forgetting_from_plasma…` tests.

## 2. A single icon: the widget in the system tray

Intended behaviour since 3.1.0-26 (commit cde256b, the author's request): the
`com.agenceapi.devicehub` widget ("ApiHub") **is** the keyboard icon in
the system tray.

* left click: panel anchored to the icon, opening and closing like the
  sound or network ones;
* right click: the daemon's action entries (Refresh, Copy, Bluetooth,
  Rename, Repair, Reconnect, Disconnect, Forget, and the two Fn mode
  choices, radio items of one group). The widget builds the menu from
  `Tray.MenuItems()` (JSON, the ten action entries only) on every state change, and each entry goes through
  `Tray.ActivateMenuItem(id)`, executed like a click; any other id, or an
  entry the current state disables, is refused;
* the daemon has no icon of its own (§8): the widget is the only one.

The widget is a system tray item
(`X-Plasma-NotificationArea: "true"` and `KPlugin.EnabledByDefault: true` in
`metadata.json`, category `Hardware`): Plasma shows it on installation. If the
user removed it, System Tray → Configure → Entries → ApiHub brings it back.
The tray loads it only while `com.agenceapi.AppleKbMonitor1` is on the
session bus (`X-Plasma-DBusActivationService`): with the daemon stopped there
is no keyboard icon at all. The "NO DAEMON" view only shows in a copy of the
widget placed on a panel or the desktop.

After an update, plasmashell keeps the widget it loaded at startup:
`systemctl --user restart plasma-plasmashell.service` (the package scriptlet
reminds of it on every update).

History: from 3.1.0-22 to 3.1.0-25 the widget only offered a small popup,
with no right-click menu; cde256b gave it the daemon's full menu.

Test: `menu_json` and `MenuItems` / `ActivateMenuItem` tests on a private bus,
`plasma/tests/run-widget-tests.sh` (fake daemon).

## 3. Battery alerts and PowerDevil

PowerDevil follows the keyboard through UPower and alerts at `PeripheralBatteryLowLevel`
(`powerdevilrc`, group `BatteryManagement`, **10 %** by default) through the
`lowperipheralbattery` event of `powerdevil.notifyrc`. Our 30 / 15 / 5 % steps were
a second voice on the same device.

`[notifications] defer_to_powerdevil = true` (default): the daemon reads those two
files (`~/.config`, then `/etc/xdg`; **read-only**, equivalent of
`kreadconfig6 --file powerdevilrc --group BatteryManagement --key
PeripheralBatteryLowLevel`), checks that `org.kde.Solid.PowerManagement` is on
the bus, that the event still shows a popup and that the keyboard appears in
`/sys/class/power_supply/hid-*-battery` (what UPower sees). If all of this holds
(check redone at most every 60 s):

* our **percentage** alerts become **a single distinct reminder** "Estimate
  from your batteries" (event `BatteryEstimate`), sent at the first step crossed
  and only if the alert relies on the chemistry-based estimate (information
  PowerDevil does not have); reset when the batteries are changed;
* if the alert relies on the keyboard's raw indication (no estimate), that is the
  figure PowerDevil sees: nothing is sent;
* the keyboard's own alerts (`0x30`, `KeyboardAlert`) do not change.

`defer_to_powerdevil = false` restores the full 30 / 15 / 5 % steps.
PowerDevil missing, popup disabled in Settings › Notifications, or keyboard not
followed by UPower: previous behaviour.

## 4. Two names for one keyboard

The daemon exposes `Name` = the BlueZ **alias** (the one the user picks). But:

| Where | Name shown | Source |
|---|---|---|
| Bluetooth applet, Battery applet, widget | alias | BlueZ `Device1.Alias` |
| KWin, Settings › Keyboard, Input & Output | kernel name | `HID_NAME` of the input device |

The kernel name is set when the input device is created: it only changes after a
**reconnection** of the keyboard (turning it off and on again). KWin stores per-device settings
under that name (`kcminputrc [Libinput][vendor][product][name]`):
renaming inside the keyboard would lose such settings. `akmctl status` adds a
`Kernel:` line when the alias and the kernel name differ (JSON key `kernel_name`).

## 5. F4 and Eject keys

On package installation and update (up to 3.1.0-10), a message suggested
`akmctl keymap kde-apply --dry-run` then `akmctl keymap kde-apply` (F4 → application
launcher, Eject → session menu; `--undo` removes exactly those keys).
**The package changes no KDE shortcut.** Details: `docs/KEYS.md`.

## 6. Languages

19 languages are shipped in the three gettext domains (daemon and `akmctl`, KDE module,
widget). Language: `LANGUAGE` (a list, honoured by gettext), then the first non-empty
variable among `LC_ALL`, `LC_MESSAGES`, `LANG`; a language without a catalog gives English
(default).

| Surface | Mechanism | Test |
|---|---|---|
| Plasma widget | `i18n()` + `plasma/po/<lang>.po` compiled to `.mo` (`/usr/share/locale/<lang>/LC_MESSAGES/plasma_applet_com.agenceapi.devicehub.mo`) | `plasma/tests/check_i18n.py` (in `run-widget-tests.sh`) |
| Notifications, tooltip and icon menu | `notify.rs`, `tray/view.rs` | existing tests (FR ≠ EN for each text) |
| Reasons of the sleep and shutdown inhibitors | `sleep.rs`, `inhibitor_reason` | `sleep::tests::inhibitor_reasons_…` |
| Global shortcuts | `data/com.agenceapi.AppleKbMonitor.shortcuts.desktop`: `Name[<lang>]` | `tests/check-data-files.sh` |

Adding a text: write it in English in the code (`tr("…")` / `i18n("…")`), then
add the entry to the `.po`; otherwise the test fails.

## 7. No window

There is no separate window any more: the `apihub-app` egui window, its
`com.agenceapi.AppleKbMonitor` D-Bus service and its launcher were removed
(commit `aed523a`). The Plasma widget
`com.agenceapi.devicehub` is the only interface, with System Settings › Apple
Keyboard for the settings; the daemon `apple-kb-monitord` (user unit,
`default.target`) carries the alerts and the notifications. Anything that used
to open the window (notification "Open", global shortcut) now calls
`…1.Tray.OpenPanel`, and the daemon emits `PanelRequested`, on which the widget
expands its popup.

## 8. The daemon has no icon of its own

The daemon no longer has a StatusNotifierItem: the widget is the only icon
in the notification area (§2). The daemon keeps `…1.Tray.MenuItems` (action
entries only) and `ActivateMenuItem` for the widget's right-click menu.

## 9. Global shortcuts

`data/com.agenceapi.AppleKbMonitor.shortcuts.desktop`, installed in
`/usr/share/kglobalaccel/`, declares to `kglobalacceld` the "Apple Keyboard" component
with two entries, visible in System Settings › Shortcuts:

| Entry | Command | Default key |
|---|---|---|
| Apple Keyboard (open the widget popup) | `gdbus call … --method com.agenceapi.AppleKbMonitor1.Tray.OpenPanel` | none |
| Toggle the function keys (media keys / F1–F12) | `gdbus call … --method com.agenceapi.AppleKbMonitor1.Tray.ToggleFnMode` | none |

`X-KDE-Shortcuts=` is empty in both groups: **nothing is bound until
the user assigns a key**. The package changes no existing
shortcut.

`Tray.ToggleFnMode` reads `hid_apple.fnmode` in the daemon and switches F1–F12
first (2) to media keys first (1), anything else to F1–F12 first
(`tray::next_fn_mode`); the write follows the same path as the widget menu and
the settings module (`akm-helper` behind the polkit action
`com.agenceapi.AppleKbMonitor.set-fnmode`).

Not tested here: actually binding a key in a Plasma session (requires
`kglobalacceld`) and the Plasma OSD.

## 10. KRunner

Not implemented: no runner is shipped (the one of the former window went with it).

## 11. Widget: 7-day sparkline and Fn mode button

The popup of the `com.agenceapi.devicehub` widget shows, under the gauge:

* a **sparkline of the last 7 days** (percentage, 0–100 % scale), drawn by
  `LineChart.qml` (a `Canvas`, no chart library) from the daemon's
  `History(since)`. `History.js` drops invalid points (outside 0–100 %,
  unmeasured voltage, future date) and reduces each series to 120 points for the
  sparkline (240 for the Data tab, §12) while keeping the extremes;
* a **Function keys** line: the mode read on the keyboard object
  (`Device.FnMode`) and a "Switch to F1–F12 first" / "Switch to media keys
  first" button that calls `Device.SetFnMode`. The daemon checks the caller
  and opens the polkit authentication; the widget writes nothing itself. The line is
  hidden when `hid_apple` is not loaded (unknown mode).

These reads only happen when the popup opens (`onExpandedChanged`): when closed,
the widget only follows `StateChanged`, as before. Daemon missing: the tray
removes the icon (§2); a copy on a panel or the desktop says NO DAEMON and how
to start it.

Tests (`plasma/tests/run-widget-tests.sh`): `tst_history.qml` (259,200 points over
90 days bounded to 240, extremes kept, invalid points dropped, Fn toggle,
object path refused for an invalid address) and `tst_link.qml` against the fake
daemon (`History`, `FnMode` read, `SetFnMode(2)` called exactly once, read back). Not
tested here: rendering inside plasmashell (`plasmoidviewer` would open a window on
the desktop).

## 12. Widget: popup tabs

The popup has five tabs (`FullRepresentation.qml`): **State**, **Radio**, **Keys**, **Data**,
**Diag**. Everything comes from D-Bus methods the daemon exposes; the widget starts no
process and reads no file.

| Tab | Contents | D-Bus source |
|---|---|---|
| State (`TabStat.qml`) | batteries, autonomy, estimate, keyboard name (rename), firmware, Fn mode, Reconnect | `GetState`, `SetAlias`, `Device.FnMode` / `SetFnMode`, `Link.Reconnect()` |
| Radio (`TabRadio.qml`) | signal and link quality, disconnections, last wake-up, monitor's link (attempts, failures, last error), paired host (masked), Reconnect | `GetState`, `Link.Status()`, `Link.Reconnect()` |
| Keys (`TabKeys.qml`) | Fn mode (function keys or media keys first), Caps Lock / Num Lock; the key table and mapping editor stay in System Settings | `Device.FnMode` / `SetFnMode`, `PropertiesChanged` of the keyboard object |
| Data (`TabData.qml`) | batteries (0–100 %) and voltage over 24 h, 7, 30 or 90 days, device details (masked address, chip, driver, installed on) | `History(t since)`, `GetState` |
| Diag (`TabDiag.qml`) | monitor running and version, last error, the full check run by the daemon (passed / problems, log) | `DaemonVersion` property, `Diagnose()`, `Refresh()` |

The charts never draw more than 240 points per series (`History.js`, `POINTS_MAX`),
whatever the period. The daemon caps `History(since)` at 2000 points;
`HistoryMax(since, max)` returns fewer, but the widget does not use it yet.

When the daemon leaves the bus, `clear()` empties every value of its session, errors and
check results included; only the keyboard's identity (name, model, address, firmware) stays. Every call is gated on the daemon owning its name, so the widget
never makes D-Bus start it.

Tests: `tst_pages.qml` instantiates the full popup and its five tabs off screen, on a
stand-in applet, and fails on any binding to a missing name; `tst_link.qml` and
`tst_fnmode.qml` check the calls above against `fake_services.py` on a private bus that
cannot start any service. Not tested here: rendering inside plasmashell.

## 13. Autonomy: one estimate, formatted by each client

The daemon publishes one autonomy estimate: the least-squares forecast of the battery
percentage over the last 30 days (`forecast` in `GetState` / `Json`: `empty_at`,
`rate_pct_per_day`, `fitted_pct`, `span_s`, `buckets`; `null` when there is not enough
history). The same value backs the `RemainingSeconds` and `EmptyAt` properties of the
keyboard object and `.Device`. The state carries no preformatted text for it: the former
`remaining_display` field (an English string from a second, voltage-based estimate) is gone.

Each client turns `empty_at` into words in its own catalog: "≈ N days" from 1.5 days on,
else "≈ N h". The widget (tooltip and State tab), the KDE module (State page, "Remaining:"),
the icon menu tooltip ("Estimated runtime:") and `akmctl` (Waybar tooltip, "Autonomy:")
all show the same number.
