# Sleep and wake: HID_CONTROL SUSPEND / EXIT_SUSPEND (#244)

Before the computer sleeps, each connected Apple Bluetooth keyboard receives one HID_CONTROL **SUSPEND** byte (`0x13`); when it wakes, one **EXIT_SUSPEND** byte (`0x14`). One byte, once per keyboard, on the HIDP control channel (L2CAP PSM `0x0011`), never retried.

## What Apple does

`RE-GHIDRA-IOBLUETOOTH.md` §3.3 and §5.3, macOS 26.5 `bluetoothd`:

* `FUN_1006a1ae0` (`HIDProfile::prepareForSleep`): for each connected Apple HID, HID_CONTROL SUSPEND once per handle, then a wait of at most 1000 ms for the mode change events;
* `FUN_1006a2108` (`prepareForWake`): EXIT_SUSPEND for each Apple HID.

`RE-GHIDRA-KEXT.md` §2.4 adds that on macOS the A1314 is driven by the kernel driver, whose `HIDExitSuspend` command puts nothing on the wire; the direct `0x14` path of `bluetoothd` serves the HID devices handled in user space. Under BlueZ the keyboard **is** a user-space HID device (`bluetoothd` + `uhid`, `UserspaceHID=true` by default in BlueZ 5.87): the direct path is the one that applies. Decision of the manager (#244): send both bytes.

## Why a root helper borrows the socket of bluetoothd

BlueZ 5.87 exposes no D-Bus method for the HIDP control channel and owns it already; a second L2CAP channel to the keyboard on PSM `0x0011` cannot be opened. BlueZ is not patched. `akm-hid-control` (root) duplicates the control socket that `bluetoothd` already holds (`pidfd_open` + `pidfd_getfd`, Linux ≥ 5.6) and writes one byte to it.

```
akm-hid-control suspend|exit-suspend [--mac XX:XX:XX:XX:XX:XX] [--dry-run]
```

## Safety rules (code: `apihub-app/crates/akm-helper/src/hidctl.rs`)

1. **Targets**: Apple keyboards of the model table (`akm-core/src/model.rs`) connected over Bluetooth as HID devices: `/sys/bus/hid/devices/*/uevent` with bus `0005`, (vendor, product) in the table, MAC from `HID_UNIQ`. Under BlueZ this list is exactly the input devices `bluetoothd` has connected (it creates the `uhid` device). The helper stays `libc`-only, without a D-Bus client in root. `--mac` must be one of them, else nothing is done. Any other device is never addressed.
2. **Process**: PID = `MainPID` of `bluetooth.service` (`/usr/bin/systemctl show`, empty environment), pinned with `pidfd_open`; `/proc/<pid>/exe` must be `/usr/lib/bluetooth/bluetoothd` (or the Debian/Fedora path; ` (deleted)` after an upgrade is accepted); uid 0; still alive after the scan.
3. **Socket**: each `socket:[…]` of `/proc/<pid>/fd` is duplicated and inspected with `getsockopt` / `getsockname` / `getpeername` only. Required: `SO_DOMAIN` = `AF_BLUETOOTH`, `SO_PROTOCOL` = `BTPROTO_L2CAP`, `SO_TYPE` = `SOCK_SEQPACKET`, peer = MAC, peer PSM `0x0011`, local PSM `0x0011` (or `0`: outgoing, unbound), `L2CAP_CONNINFO` answers (state `BT_CONNECTED`). The interrupt channel (`0x0013`) never qualifies. **Exactly one** candidate per keyboard, else nothing is written to that keyboard. Non-L2CAP duplicates are closed at once.
4. **Write**: one `send(2)` of one byte with `MSG_DONTWAIT | MSG_NOSIGNAL`. The byte comes from the `HidControl` enum (`0x13`, `0x14`) and is checked again by `check_byte` (a test sweeps the 256 values). No `setsockopt`, no `fcntl`: the socket and its file flags are shared with `bluetoothd` and are never changed. No retry. Then at most 1 s (`DRAIN_WAIT`, as Apple) for the byte to leave the socket queue (`TIOCOUTQ` = `SO_SNDBUF`), and every duplicate is closed.
5. **Journal**: every decision (keyboards, pid/exe, each L2CAP socket of the keyboard with PSM, CID and state, the byte sent or the reason for refusing) goes to journald: stderr under systemd, `syslog(3)` with the tag `akm-hid-control` under pkexec. `journalctl -t akm-hid-control` or `journalctl -u apple-kb-monitor-suspend -u apple-kb-monitor-resume`.
6. **Source scan**: `this_file_has_one_write_path` fails if a second `send`, or any `write` / `sendmsg` / `setsockopt` / `fcntl`, appears in the helper.
7. **Apple's breaker (R3, #251)**: before the byte, the circuit breaker of the user daemon is consulted (next section). Open: `NOT sent`, exit 0 (nothing to do, nothing to retry).

## The daemon's circuit breaker reaches the helper (R3, #251)

Apple's driver stops **every** emission after three consecutive silences, HID_CONTROL included (`docs/PARITE-APPLE.md` R3), until a new connection or a sleep. The breaker lives in `apple-kb-monitord` (user session); the helper runs as root from system units at sleep time, without a session bus, with `RestrictAddressFamilies=AF_UNIX` and `libc` only. The daemon therefore **publishes** its breaker as one small file, and the helper reads it:

```text
/run/user/<uid>/apple-kb-monitor/breaker.state      ($XDG_RUNTIME_DIR, next to hid.lock)
schema=2                     (schema 1 = without starttime, still read)
mac=AA:BB:CC:DD:EE:F1        (keyboard followed, or -)
open=1                       (R3 flag)
counter=3                    (consecutive silences)
written_unix=1790000000
pid=4242                     (the daemon)
starttime=123456             (/proc/<pid>/stat field 22 of the daemon: a recycled pid does not match)
```

* **Writer** (`akm-core/src/read_policy.rs::publish_breaker_state`, `breaker_state.rs`): after every pass of the actor loop, rewritten atomically (temp + rename, 0644 in the user's private 0700 directory) only on a change or every 20 s (heartbeat); removed when the daemon stops.
* **Reader** (`hidctl::breaker_verdict`, same `breaker_state.rs` compiled in by path, `libc` only): only the state of the **active user of the seat** (`ACTIVE_UID=` of logind's `/run/systemd/seats/seat0`, the owner of the keyboard through `uaccess`) is read, never an aggregate of every `/run/user/<uid>` (#256: another local account must not be able to block the keyboard of the user at the seat). No active user known = **refused** (fail closed). The file is read with the checks of the other root readers (`fsutil::read_user_file`: owner = `<uid>` of the path, regular file, `O_NOFOLLOW`, one hard link, ≤ 1 KiB). The unit already holds `CAP_DAC_READ_SEARCH`.
* **Liveness of the daemon** (#256, `breaker_state::daemon_alive_in`): `/proc/<pid>/exe` = `/usr/bin/apple-kb-monitord` (or the same with ` (deleted)` after an upgrade), real uid = the user, and, when the state carries it, the start time of the writer. `comm` is never trusted (any process sets it with `prctl(PR_SET_NAME)`).
* **Decision** (`breaker_state::verdict`, pure), per keyboard:

| Published state | Daemon | Verdict |
|---|---|---|
| dated more than 5 s in the future (#256) | running | **refused** (clock or file not trusted: fail closed) |
| dated more than 5 s in the future | gone | sent (a file left by a daemon whose clock ran ahead never blocks for ever) |
| `open=1` for this MAC, fresh (≤ 60 s) | running or just dead | **refused** (Apple's verdict for this connection, 65 s at most) |
| closed, fresh | — | sent |
| older than 60 s | running (exe, uid and start time of the writer) | **refused** (wedged daemon, breaker unknown: fail closed) |
| older than 60 s | gone | sent |
| unreadable / malformed | running (any `apple-kb-monitord` of that uid) | **refused** |
| unreadable / malformed | gone | sent |
| no file | — | sent (older daemon or none: the behaviour before #251) |
| another MAC, or `mac=-` | — | sent (not concerned) |

Why not D-Bus or a socket: no session bus at sleep time for a system unit, no `AF_BLUETOOTH`/`AF_INET` allowed, the helper stays without a D-Bus client in root; `/run/user/<uid>` is the directory the root helpers already read (`akm-keymap-helper`), with the same owner checks. The same verdict guards `akmctl` (`hidraw::WriteDoor`: `0x41` forget, `0x55` name): a short-lived process has a fresh breaker of its own, the daemon's is the one that counts.

Journal: `… breaker state Closed: emission allowed` / `… NOT sent, breaker open: the keyboard did not answer 3 requests in a row (Apple R3: nothing is sent until a new connection or a sleep)` / `… breaker state is N s old (> 60 s) while the daemon runs: not trusted, nothing sent`. `--dry-run` prints the verdict too.

## Triggers

Two **system** units, enabled by the package (symlinks in `/usr/lib/systemd/system/*.target.wants`):

| Unit | Ordering | Command | Bound |
|---|---|---|---|
| `apple-kb-monitor-suspend.service` | `Before=sleep.target`, `WantedBy=sleep.target` | `suspend` | `TimeoutStartSec=2s`, `ExecStart=-…` (a failure never blocks the sleep) |
| `apple-kb-monitor-resume.service` | `After=` and `WantedBy=` `suspend`, `hibernate`, `hybrid-sleep`, `suspend-then-hibernate` targets | `exit-suspend` | `TimeoutStartSec=3s`, `ExecStart=-…` |

Hardening: `CapabilityBoundingSet=CAP_SYS_PTRACE CAP_DAC_READ_SEARCH` (`pidfd_getfd` needs the ptrace "attach" check; with Yama `ptrace_scope=1` a non-descendant needs `CAP_SYS_PTRACE`), `NoNewPrivileges=yes` (compatible: it does not affect ptrace), `ProtectSystem=strict`, `ProtectHome=read-only` (not `yes`: `yes` hides `/run/user`, so the helper could not read the daemon's breaker state and reported `NoState`; measured 2026-10-02), `PrivateTmp`, `PrivateDevices`, `ProtectKernel*`, `ProtectControlGroups`, `RestrictAddressFamilies=AF_UNIX` (the helper creates no Bluetooth socket: it borrows one; `AF_UNIX` is for `systemctl`), `RestrictNamespaces`, `MemoryDenyWriteExecute`, `SystemCallFilter=@system-service pidfd_getfd` (`pidfd_getfd` belongs to `@debug`). `ProtectProc` stays `default`: the helper must see `bluetoothd`. `systemd-analyze security`: 2.3 OK.

## Order with the user daemon (no race)

* **Sleep**: logind emits `PrepareForSleep(true)`; `apple-kb-monitord` (`sleep.rs`, #145) pauses its HID reads, waits for an in-flight read (≤ 3 s) and releases its `delay` inhibitor. Only when every delay inhibitor is released does logind start `suspend.target`, which pulls `sleep.target`: `apple-kb-monitor-suspend.service` runs then, before `systemd-suspend.service`, while the link is still up. The daemon reads nothing at that moment.
* **Wake**: `systemd-suspend.service` returns, the sleep target is reached again and `apple-kb-monitor-resume.service` starts; logind emits `PrepareForSleep(false)` and the daemon keeps hardware access paused for `RESUME_GRACE` (4 s). The helper takes well under a second, so `0x14` leaves before the reads resume. If the Bluetooth link did not survive the sleep (adapter powered off), no socket is found, nothing is sent, the unit succeeds; the keyboard reconnects by itself.

## Configuration and opt-out

`/etc/apple-kb-monitor/hid-suspend.conf` (packaged, `backup=`):

```
enabled = false
```

**Off by default since 2026-10-02**, in the packaged file and in the code: an absent file, an empty file or a file without the `enabled` key means **disabled**. Measured on an A1314 (firmware `0x0050`): after SUSPEND the keyboard stays connected but answers no GET_REPORT and no longer types, and EXIT_SUSPEND did not bring it back. `enabled = true` turns it on, at your own risk. A file edited before that date keeps its content at the upgrade (`backup=`, the new one is installed as `.pacnew`): if it still says `enabled = true`, the upgrade prints a warning. A file that is not root-owned, is group/world writable, a symlink, or malformed turns the feature **off** (fail closed, logged). Units: `sudo systemctl mask apple-kb-monitor-suspend.service apple-kb-monitor-resume.service`.

## Manual test

```
akmctl hid-control suspend --dry-run          # pid, fd, MAC, PSM, state; nothing written
akmctl hid-control suspend                    # sends 0x13 once (administrator authentication)
akmctl hid-control exit-suspend               # sends 0x14 once
```

Polkit action `com.agenceapi.AppleKbMonitor.hid-control`, `auth_admin` (no `_keep`). Exit 0 = sent or nothing to do (no keyboard, disabled, **breaker open**), 1 = refused or failed (reason printed and logged), 64 = usage.

Dry run on PC01 (BlueZ 5.87, `uhid`, kernel 7.1, Yama 1), 2026-10-01, read-only:

```
SUSPEND (0x13) --dry-run
bluetoothd pid 1144 (/usr/lib/bluetooth/bluetoothd): 11 L2CAP socket(s)
  AA:BB:CC:DD:EE:F1 fd 31: psm local 0x0011 peer 0x0011 cid 0x0041 state connected (hci handle 0x0100)
  AA:BB:CC:DD:EE:F1 fd 32: psm local 0x0013 peer 0x0013 cid 0x0042 state connected (hci handle 0x0100)
AA:BB:CC:DD:EE:F1 (Apple Wireless Keyboard (A1314, aluminum, ISO) 0x0256): pid 1144 fd 31 psm 0x0011 hci 0x0100 byte 0x13 SUSPEND: NOT sent (--dry-run)
```

## Limits

* If BlueZ hands the HID channels to the kernel (`UserspaceHID=false` in `/etc/bluetooth/input.conf`, kernel `hidp`), `bluetoothd` still keeps its descriptors (it watches them for hang-up), so the same socket is found; the byte then shares the channel with the kernel `hidp` session (one SEQPACKET packet, atomic).
* The effect on the keyboard is not observable without writing. Expected (Apple): the keyboard enters its low-power state cleanly instead of on supervision timeout. Risk: low (production traffic of macOS). No real write has been done by the tests: they use an `AF_UNIX` socketpair held by a child process as a fake `bluetoothd`.

## Tests

`cargo test -p akm-helper`: selection of the single candidate, refusal with 0 or 2+ candidates, refusal of every PSM other than `0x0011` (65 535 values each side), of a non-L2CAP or non-connected socket, of an executable other than `bluetoothd`, of a MAC outside the table, sweep of the 256 bytes (only `0x13` and `0x14` pass), strict arguments and configuration; `tests/hid_control_fake_bluetoothd.rs`: a child process holds the two ends of the fake channels, `pidfd_getfd` duplicates them, the peer of the control channel receives exactly one byte (`0x13`, then `0x14` in a fresh run), the interrupt peer nothing; dry run, a wrong executable, an unknown MAC and two candidates write nothing; `the_daemons_breaker_blocks_the_hid_control_byte`: a published state `open=1` -> the control peer receives **nothing** and the run exits 0, closed -> the byte, stale or malformed with a "live" daemon -> nothing, with a dead one -> the byte, another keyboard's state -> the byte. `breaker_state.rs` has its own tests (strict parsing, rule table, liveness on a fake `/proc`, atomic write, heartbeat).
