# Testing

Nothing in this page touches a real keyboard or needs root: every test uses fixtures, a spy (`FeatureSink`), an invalid descriptor, or a simulated keyboard under bubblewrap. The hardware checks at the end are read-only.

## One command: `scripts/ci-local.sh`

```bash
scripts/ci-local.sh            # full pipeline (static, build, tests, package, e2e)
scripts/ci-local.sh --fast     # pre-push subset: versions, secrets, fmt, clippy, test, kcm, shell
scripts/ci-local.sh --no-e2e   # everything but the Xvfb end-to-end tests
scripts/ci-local.sh --only claims,redaction,secrets,versions
scripts/ci-local.sh --list
```

Report in `scripts/out/<UTC stamp>/{report.json,report.txt,logs/*.log}`, `scripts/out/latest` → last run. Exit code 0 = every required step passed, 1 = one required step failed, 64 = usage. `AKM_CI_TEST_TIMEOUT` (default 900 s): a test suite that does not end in time is a **failure**. `CARGO_TARGET_DIR` is honoured.

The 22 steps (`--list`; "required" = fails the pipeline):

| Step | Required | What |
|---|---|---|
| `versions` | yes | one version everywhere: `[workspace.package] version` = `PKGBUILD pkgver` = `.SRCINFO` = first entry of `CHANGELOG.md`; `pkgrel` = `.SRCINFO` |
| `secrets` | yes | `scripts/qa_checks.py secrets`: gitleaks report parsed, optional allow-list `secrets-allow.tsv` in `$AKM_QA_ALLOW_DIR` |
| `fmt` | yes | `cargo fmt --check` on the workspace |
| `clippy` | yes | `cargo clippy --locked --workspace --all-targets -- -D warnings` |
| `test` | yes | `cargo test --workspace` with the hang timeout |
| `claims` | yes | `qa_checks.py claims`: documents, QML and Rust strings scanned for the five claims refuted by measurement (the `CLAIMS` table of `qa_checks.py`: the constant `0xF5`, the BCM2042 core, the firmware signature, the unit of the RSSI, the content of `0x4C`); a line that negates the claim passes; allow-list `claims-allow.tsv` |
| `redaction` | yes | `qa_checks.py redaction`: no Apple binary, no decompiled code committed (magic numbers, size, patterns) |
| `private` | yes | `qa_checks.py private`: no private IP, MAC, e-mail, home path or host name in the tree; `qa_checks.py tracker`: no issue number of the maintainer's private tracker in the public tree |
| `links` | yes | `qa_checks.py links`: every relative link of the `.md` files resolves |
| `deny` | yes | `cargo deny` with the policy and justified ignores of `apihub-app/deny.toml` |
| `audit` | no | `cargo audit` (advisory, network) |
| `udev` | yes | `udevadm verify` on the rules |
| `qml` | yes | `qmllint` on the widget's QML files (`plasma/com.agenceapi.devicehub/contents/ui/*.qml`, `plasma/tests/*.qml`) and `kcm/lint/qmllint.sh` on the KCM pages (any warning fails) |
| `kcm` | yes (also `--fast`) | build the daemon, configure + build the KCM plugin with cmake, `ctest` (translations, the real module against the real sealed daemon); skipped without cmake/ECM |
| `shell` | yes | `bash -n` / `sh -n` on every script and hook, plus `shellcheck -S warning` when installed, and `install.sh` piped into `bash` on a pseudo-terminal (`tests/check-install-sh.sh`) |
| `c` | yes | `gcc -Wall -Wextra -Werror rssi-helper.c` |
| `security` | yes | `tests/check-security-files.sh` (unit hardening, polkit `auth_admin` without `_keep`, no `rssi-helper` group left), `tests/check-trademarks.sh` (no third-party trademark shipped), `tests/check-data-files.sh` (every installed XML, JSON and `.desktop` file parsed), `tests/check-sleep-units.sh` (sleep / wake units), `tests/check-po.sh` (Rust catalogs complete, valid and loaded at run time), `plasma/tests/check_plaintext.py` (names shown as plain text in the widget), `tests/check-install-scriptlet.sh` (the pacman scriptlet on a fake root) `tests/keyd/check-config.sh` (the keyd example) and `tests/check-ci-workflow.sh` (the public CI runs every required step and fails on a skip) |
| `units` | yes | `systemd-analyze --user verify` on `systemd/*.service` and `*.timer` |
| `plasma` | yes | widget QML tests and catalogue completeness, `plasma/tests/run-widget-tests.sh` (needs `qml6`, `dbus-run-session`, python `dbus` + `gi`; skipped without them) |
| `re` | yes | `pytest tests/live/re`: fixture tests of the reverse-engineering tools (skipped without pytest) |
| `package` | yes | `makepkg -f --nodeps` in a copy of the tree (installs nothing; target dir reused through `AKM_CI_PKG_TARGET`); expected file list in `scripts/package-expected.txt`, optional allow-list `package-allow.tsv` |
| `e2e` | yes | `tests/e2e/run.sh` (below) |

## Rust tests (`cd apihub-app && cargo test --workspace`)

- `#[cfg(test)]` modules in every crate, plus integration tests: `akm-core/tests/` (`a1314_iso.rs` on the real capture, `read_policy_*`, `recovery_extra`, `forecast_extra`, `history_extra`, `audit_sec2_poc`), `apple-kb-monitord/tests/` (`dbus_session`, `dbus_v2`, `passive_dbus`, `first_connect`, `bluez_forget`, `notify_kde`, `notify_mute`, `desktop_i18n`, `audit_sec2_poc`), `crates/akmctl/tests/cli.rs` (the built binary on fixture files), `crates/akm-helper/tests/hid_control_fake_bluetoothd.rs` (a fake `bluetoothd` socket: breaker open → no byte, closed → the byte).
- Register-map invariants swept over the **256 ids in the three directions**: only `0x47 0x46 0x49` and the once-per-connection ids pass the Feature read gate, only Input `0x30` passes the Input gate, only the three named operations pass `check_write`, each id once per `WriteSession` with its exact length; a source scan fails the build if a second write ioctl appears (`this_build_has_one_write_path`, and the same for `akm-hid-control`).
- Apple model: one conformance test per rule R1-R8 of [APPLE-PARITY.md](APPLE-PARITY.md), full scenarios on a simulated clock, proptest properties over 512 random sequences (never an emission while the breaker is open, never two requests < 1 s apart, at most one disconnection request per connection).

## End-to-end: `tests/e2e/`

```
tests/e2e/run.sh [--out DIR]           # the widget in the notification area of a throwaway plasmashell
python3 tests/e2e/kcm.py [--only a,b]  # the System Settings module in kcmshell6 / systemsettings
```

Both run under **Xvfb** on a private session bus without any activation directory, with throwaway
`HOME`/`XDG_*` and a fake daemon (nothing of the real session is read or touched). `run.sh` checks
that the widget's right-click menu carries the daemon's entries. `kcm.py` runs 8 scenarios
(`screens`, `actions`, `systemsettings`, `absent`, `slow`, `error`, `garbage`, `akmctl_slow`, see
[KCM.md](KCM.md)) and fails on any QML error, any write during a passive look, or a GUI thread
blocked more than 100 ms. Exit 0 all passed, 1 one failed, 77 a tool is missing (skipped).

## Continuous self-check: `akmctl selftest`

`apple-kb-monitor-selfcheck.timer` runs `akmctl selftest --notify` 5 min after login and every 15 min (`Nice=10`, idle I/O). Controls: `daemon` (on the bus, `GetState` within 5 s), `daemon-restarts`, `freshness`, `versions`, `alias` (changed outside this monitor), `link` (`akmctl doctor`), `journal-daemon`, `panic`, `journal-bluetooth`, `coredumps`, `disk`, `history`. Verdict `ok` / `info` / `warn` / `bad`; each check of `--json` has `id`, `level`, `key`, `text` (English with `--json`; in the result file, the locale of the run that wrote it) and a message identifier `msg` with its values `args` (stable, translated by the System Settings module); result in `~/.local/state/apple-kb-monitor/selfcheck.json`; a desktop notification for each **new** grave problem; optionally, for the maintainer only, one Gitea issue (`--gitea-issue`, needs `AKM_GITEA_API` / `AKM_GITEA_REPO`). By hand: `akmctl selftest`, `akmctl selftest --json --no-save`.

## Before a push

`scripts/ci-local.sh --fast` (versions, secrets, fmt, clippy, the whole test suite with a hang timeout, kcm, shell) is the pre-push subset; it can be wired as a `pre-push` hook of your clone.

## CI: `.github/workflows/`

`ci.yml` runs in an `archlinux` container (with `--cap-add SYS_PTRACE`: the helper tests use `pidfd_getfd`, which Docker's default seccomp profile otherwise blocks) on every push and pull request: `cargo fmt --check` on the workspace, `cargo clippy -D warnings`, `cargo test` with a private `dbus-daemon`, shellcheck, `cargo deny`, `msgfmt --check` on every catalog, `qmllint` (Qt 6) on the widget and the KCM, the KCM build and `ctest` (catalogue completeness, the module on a private bus), the widget QML tests and catalogue checks, the `tests/live/re` fixture tests, the static checks of `ci-local.sh`, then fails if any of these reports has a skipped step. `package.yml` builds the Arch package with `scripts/release-build.sh` and publishes it on a `v*` tag. No job needs a keyboard.

## Fixtures (`tests/fixtures/`)

`a1314_iso/`: captured on the real A1314 ISO on 2026-10-01, read-only, without sudo (HID descriptor of 224 bytes, uevent of HID / hidraw / power_supply, `bluetoothctl info`, `upower -i`, udev properties, evdev attributes; the real `0x4C` fingerprint was removed). `synthetic/`: raw GET_REPORT answers. `devname/`: the Lion `setDeviceName:` frames established by disassembly. `models/`: uevent of the other product ids. Provenance: `tests/fixtures/README.md`.

## On the real keyboard, read-only

| Check | Command |
|---|---|
| Link and configuration | `akmctl doctor` (`sudo akmctl doctor` adds the stored vs kernel link-key comparison) |
| Health | `akmctl selftest` |
| Percentage consistency sysfs / UPower / BlueZ / CLI, hidraw rights, `uaccess` tag, services | `tests/live/check_keyboard.sh [--mac AA:BB:..] [--tolerance N] [--strict]`: `PASS\|FAIL\|WARN\|SKIP` per control, exit 0 without `FAIL`, 2 if the keyboard is absent |
| Keys | `akmctl keys --check`, then the checklist of [KEYS.md](KEYS.md) §7 |
| The three safe reports | `akmctl dump` (press a key first) |
| Sleep / wake byte without sending | `akmctl hid-control suspend --dry-run` |
| Reconnection probe | `tests/live/reconnect_probe.sh` (observes; see [RECONNECTION-PAIRING.md](RECONNECTION-PAIRING.md) §7 for the controlled test, to be run with the manager) |

The reverse-engineering tools of `tests/live/re/` read the hardware and are **not** installed by the package; their compile and fixture tests run in CI without a keyboard.

## Gaps

Hardware validation of 16 of the 17 models; the effect on the keyboard of `WillShutdown`, `RecantConnection` and the sleep / wake bytes is not observable from the host; whether the name written in the keyboard (`0x55`) survives a power-off or a battery change is not measured.
