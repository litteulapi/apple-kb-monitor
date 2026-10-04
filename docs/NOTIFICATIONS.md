# Notifications and KDE integration

The `apple-kb-monitord` daemon speaks the `KNotification` dialect to
`org.freedesktop.Notifications`: Plasma treats it as a full application
(System Settings → Notifications → Apple Keyboard Monitor).

## What links the daemon to Plasma

| Item | Value | Role |
|---|---|---|
| `data/apple-kb-monitor.notifyrc` | installed in `/usr/share/knotifications6/` | declares the application and its events (names in English and 19 languages) |
| hint `x-kde-appname` | `apple-kb-monitor` | name of the notifyrc file |
| hint `x-kde-eventId` | `BatteryLow`, ... | `[Event/<id>]` section: Plasma applies the user's popup, sound, history and "Do Not Disturb" settings |
| `app_name` | `Apple Keyboard Monitor` | displayed name (= `[Global] Name=` of the notifyrc) |

The three hints are the ones `libKF6Notifications` emits (checked in the
installed binary). The user's settings are written by Plasma to
`~/.config/apple-kb-monitor.notifyrc`.

## Events

| Id | When | Urgency | Buttons | Slot (replacement) |
|---|---|---|---|---|
| `BatteryLow` | a low threshold crossed (default 30 / 15 %) | normal, 12 s | Open, Remind me tomorrow | `battery` |
| `BatteryCritical` | critical level crossed (default 5 %) | critical, persistent | Open | `battery` |
| `KeyboardAlert` | keyboard alert (Input `0x30`) | normal or critical | Open | `battery` |
| `KeyboardDisconnected` | link lost | low, transient | Open | `link` |
| `KeyboardReconnected` | link restored | low, transient | Open | `link` |
| `KeyboardOff` | keyboard switched off (announced before the cut) | low, transient | Open | `link` |
| `KeyboardUnreachable` | paired keyboard that no longer answers | normal | Open | `link` |
| `RepairNeeded` | re-pairing needed | critical, persistent | Repair…, Open | `repair` |
| `KeyboardRemoved` | keyboard removed from the computer from outside (Plasma's Forget, `bluetoothctl remove`) | normal, 12 s | Repair…, Open | `repair` |
| `SettingsReapply` | the reconnecting keyboard has a remembered Fn mode different from the one in effect; states that the `hid_apple` setting is shared by all Apple keyboards on the computer | normal, 12 s | Apply (only button; a polkit authentication follows) | `settings` |
| `ForgetConfirm` | "Forget the keyboard?": confirmation requested by the tray's "Forget this keyboard…" entry or by `Link.RequestForget`; valid for 60 s | critical, 60 s | Forget (only button; a click on the body does nothing) | `forget` |
| `LinkUnstable` | "unstable link": more than 3 disconnections in one hour (excluding system sleep, requested disconnection, keyboard switched off), once per episode | normal, 12 s | Open | `link-quality` |
| `BatteryEstimate` | the single "estimate based on your batteries" reminder when PowerDevil already alerts | normal, 12 s | Open, Remind me tomorrow | `battery` |
| `FirmwareUpdate` | newer firmware known (embedded table): once per version, remembered | normal | Open, Remind me tomorrow | `firmware` |
| `BatteryReminder` | "replace the batteries": smoothed `0x49` voltage below the **keyboard's** Low then Critical threshold (`0x60` = `0x5A`, 2506 / 2404 mV on A1314), with the keyboard's % reading | Low: normal, 15 s; Critical: critical, persistent | Open, Remind me tomorrow, Ignore this reminder | `battery` |
| `BatteryReplaced` | new batteries detected | normal | Open | `battery` |
| `BatteryAdvice` | "batteries replaced too often": two consecutive battery sets lasting less than 30 days, said once, when the second is replaced; measured durations then advice (declared chemistry, another battery series or low self-discharge NiMH rechargeables, switch the keyboard off) | normal, 12 s | Open | `advice` |
| `Error` | operation failure | critical | none | `error` |

* Critical urgency: `expire_timeout = 0` (does not go away on its own). Low: 6 s,
  not kept in history (`transient`).
* A click on the notification body (`default`) does "Open".
* Replacement: the daemon keeps the id returned by the server for each slot
  and sends it back in `replaces_id`: battery alerts do not pile up, the
  reconnection replaces the disconnection. A notification closed by the user
  (`NotificationClosed`) is forgotten.
* Triggering of `BatteryReminder` and `FirmwareUpdate` (`akm-core::reminder`,
  called by the actor after each read): **once per crossing**.
  A battery level is re-armed only if the voltage rises 50 mV above its
  threshold (ADC noise ignored), or by new batteries; the firmware is
  announced again for a newer version, or after becoming up to date again.
  The state is kept in `$XDG_STATE_HOME/apple-kb-monitor/notices.json` (written
  only when the history is): a daemon restart does not repeat a reminder
  already seen. Without read thresholds (kernel read only), no voltage
  reminder. `[alerts] enabled = false` also turns off the battery reminder.

## Buttons

| Button | Effect |
|---|---|
| Open | opens the ApiHub widget popup: the daemon calls its own `com.agenceapi.AppleKbMonitor1.Tray.OpenPanel`, which emits `PanelRequested`; the notification's activation token is not passed on (the widget runs inside plasmashell and QML has no API to use it), so on Wayland KWin may open the popup without giving it focus |
| Repair… | `akmctl repair` in a terminal (`konsole --hold -e`, then the other known terminals) |
| Remind me tomorrow | the notification is set aside and re-sent identically 24 h later, including after a daemon restart (`deferred-notifications.json`); if the due time falls in a "do not disturb" window, it waits for the end of the window; new batteries, or "Ignore this reminder", cancel a pending battery reminder. Offered by `BatteryLow`, `BatteryEstimate`, `BatteryReminder` and `FirmwareUpdate`, never by what must be handled right away (`BatteryCritical`, `RepairNeeded`) |
| Apply | restores the Fn mode remembered for this keyboard, through the usual path (`akm-helper` behind polkit); only once per offer |
| Forget | confirms the forget requested from the tray: the daemon calls `org.bluez.Adapter1.RemoveDevice` for this keyboard, only once, if the request is less than 60 s old. Nothing is written to the keyboard. Without a notification server, the request opens `akmctl repair` (typed confirmation) instead |
| Ignore this reminder | turns off the Low threshold reminder until new batteries are detected; the Critical reminder is still sent |

The `ActionInvoked` signal is listened to by a dedicated thread (`kb-notify-actions`), on its
own connection: neither acquisition nor sending waits. Each action runs
in its own thread. Safeguards: the signal must come from the current owner of the
`org.freedesktop.Notifications` name (another client cannot trigger
"Repair"), and the button must be offered by that notification. At most 64
pending notifications.

## Language

Texts come from the daemon's gettext catalog (19 languages): `LANGUAGE` (a list), then
the first non-empty variable among `LC_ALL`, `LC_MESSAGES`, `LANG`; a language without a
catalog gives English. Event names are translated in the notifyrc (`Name[<lang>]`,
`Comment[<lang>]`). See [INTEGRATION-KDE.md §6](INTEGRATION-KDE.md#6-languages).

## Configuration

`[alerts] enabled`, `[notifications] connection` / `battery_replaced` and
`--no-notify` turn off sending on the daemon side. The rest (popup, sound, history,
do not disturb) is set in Plasma, per event.

### "Do not disturb" time windows

`[notifications] quiet_hours = "22:00-07:00"` (local time; several windows
separated by commas; a window may span midnight; `""` = none,
the default). Inside a window:

* a **non-critical** notification is not sent: it is logged
  (`notification held until 07:00 (quiet hours 22:00-07:00): [BatteryLow] …`)
  and kept, one per replacement slot (the latest), then sent at the
  end of the window (`quiet hours over: showing […]`);
* a **critical** notification (critical batteries, re-pairing needed,
  error) is always sent immediately.

What is waiting is kept in
`$XDG_STATE_HOME/apple-kb-monitor/deferred-notifications.json` (at most 32
entries): a daemon restart loses nothing. Plasma's "Do Not Disturb" mode
is still applied by Plasma itself (`ShowPopupsInDndMode` for the
critical events of the notifyrc): both add up. Logic:
`akm-core::quiet` (windows), `akm-core::deferred` (queue),
`apple-kb-monitord::notify_policy` (decision, injectable clock).

## Testing

```sh
# The notifyrc is installed and readable
ls /usr/share/knotifications6/apple-kb-monitor.notifyrc

# Automated tests (private bus + fake server, nothing reaches the desktop)
cargo test -p apple-kb-monitord --test notify_kde --test notify_mute --test notify_quiet_remind
```

On the desktop: System Settings → Notifications → "Apple Keyboard
Monitor" must list the events; unchecking "Show popups" for `BatteryLow`
removes the bubble without touching the daemon.
`dbus-monitor "interface=org.freedesktop.Notifications"` shows `x-kde-appname`,
`x-kde-eventId` and the actions of each `Notify`.
