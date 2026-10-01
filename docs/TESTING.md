# Testing

Nothing in this page touches a real keyboard or needs root: every test uses fixtures, a spy (`FeatureSink`), an invalid descriptor, or a simulated keyboard under bubblewrap. The hardware checks at the end are read-only. Reference and rationale: [QA-AUTOMATIQUE.md](QA-AUTOMATIQUE.md) (French).

## One command: `scripts/ci-local.sh`

```bash
scripts/ci-local.sh            # full pipeline (static, build, tests, package, e2e)
scripts/ci-local.sh --fast     # pre-push subset: versions, secrets, fmt, clippy, test, shell
scripts/ci-local.sh --no-e2e   # everything but the Xvfb end-to-end tests
scripts/ci-local.sh --only claims,redaction,secrets,versions
scripts/ci-local.sh --list
```

Report in `scripts/out/<UTC stamp>/{report.json,report.txt,logs/*.log}`, `scripts/out/latest` → last run. Exit code 0 = every required step passed, 1 = one required step failed, 64 = usage. `AKM_CI_TEST_TIMEOUT` (default 900 s): a test suite that does not end in time is a **failure**. `CARGO_TARGET_DIR` is honoured.

The 18 steps (`--list`; "required" = fails the pipeline):

| Step | Required | What |
|---|---|---|
| `versions` | yes | one version everywhere: `[workspace.package] version` = `PKGBUILD pkgver` = `.SRCINFO` = first entry of `CHANGELOG.md`; `pkgrel` = `.SRCINFO` |
| `secrets` | yes | `scripts/qa_checks.py secrets`: gitleaks report parsed, allow-list `secrets-allow.tsv` |
| `fmt` | yes | `cargo fmt --check` on the workspace |
| `clippy` | yes | `cargo clippy --locked --workspace --all-targets -- -D warnings` |
| `test` | yes | `cargo test --workspace` with the hang timeout |
| `claims` | yes | `qa_checks.py claims`: documents, QML and Rust strings scanned for the five claims refuted by [CONTRE-AUDIT.md](CONTRE-AUDIT.md) (the `CLAIMS` table of `qa_checks.py`: the constant `0xF5`, the BCM2042 core, the firmware signature, the unit of the RSSI, the content of `0x4C`); a line that negates the claim passes; allow-list `claims-allow.tsv` |
| `redaction` | yes | `qa_checks.py redaction`: no Apple binary, no decompiled code committed (magic numbers, size, patterns) |
| `deny` | yes | `cargo deny` with the policy and justified ignores of `apihub-app/deny.toml` |
| `audit` | no | `cargo audit` (advisory, network) |
| `udev` | yes | `udevadm verify` on the rules |
| `qml` | yes | `qmllint` on the widget's QML files (`plasma/com.agenceapi.devicehub/contents/ui/*.qml`, `plasma/tests/*.qml`) |
| `shell` | yes | `bash -n` / `sh -n` on every script and hook, plus `shellcheck -S warning` when installed |
| `c` | yes | `gcc -Wall -Wextra -Werror rssi-helper.c` |
| `security` | yes | `tests/check-security-files.sh` (unit hardening, polkit `auth_admin` without `_keep`, `rssi-helper` group) and `plasma/tests/check_plaintext.py` (names shown as plain text in the widget) |
| `units` | yes | `systemd-analyze --user verify` on `systemd/*.service` and `*.timer` |
| `plasma` | no | widget QML tests, `plasma/tests/run-widget-tests.sh` (needs `qmltestrunner`) |
| `package` | yes | `makepkg -f --nodeps` in a copy of the tree (installs nothing; target dir reused through `AKM_CI_PKG_TARGET`); expected file list in `scripts/package-expected.txt`, allow-list `package-allow.tsv` |
| `e2e` | yes | `tests/e2e/run.sh` (below) |

## Rust tests (`cd apihub-app && cargo test --workspace`)

- `#[cfg(test)]` modules in every crate, plus integration tests: `akm-core/tests/` (`a1314_iso.rs` on the real capture, `read_policy_*`, `recovery_extra`, `forecast_extra`, `history_extra`, `audit_sec2_poc`), `apple-kb-monitord/tests/` (`dbus_session`, `dbus_v2`, `passive_dbus`, `first_connect`, `bluez_forget`, `notify_kde`, `notify_mute`, `desktop_i18n`, `audit_sec2_poc`), `crates/akmctl/tests/cli.rs` (the built binary on fixture files), `crates/akm-helper/tests/hid_control_fake_bluetoothd.rs` (a fake `bluetoothd` socket: breaker open → no byte, closed → the byte).
- Register-map invariants swept over the **256 ids in the three directions**: only `0x47 0x46 0x49` and the once-per-connection ids pass the Feature read gate, only Input `0x30` passes the Input gate, only the three named operations pass `check_write`, each id once per `WriteSession` with its exact length; a source scan fails the build if a second write ioctl appears (`this_build_has_one_write_path`, and the same for `akm-hid-control`).
- Apple model: one conformance test per rule R1-R8 of [PARITE-APPLE.md](PARITE-APPLE.md), full scenarios on a simulated clock, proptest properties over 512 random sequences (never an emission while the breaker is open, never two requests < 1 s apart, at most one disconnection request per connection).
- `audit-props/`, `audit-fuzz/` (cargo-fuzz targets: `config`, `decode`, `history`, `rdesc`), `audit-ui/` (eframe shim and harness): used by the tooled audits ([AUDIT-OUTILLE.md](AUDIT-OUTILLE.md)), not part of the package.
- Smoke tests of the window: `apihub-app/tests/smoke_xvfb.rs`, `smoke_wayland.rs` (`LC_ALL=C`, they look for the English title).

## End-to-end: `tests/e2e/run.sh`

```bash
tests/e2e/run.sh [--out DIR] [--only a,b] [--duration S] [--update-refs] [--bin-dir DIR]
```

The real window and the real daemon run under **Xvfb** inside **bubblewrap** (unprivileged user namespace): `/sys/class/hidraw`, `/sys/bus/hid/devices` and `/sys/class/power_supply` are fake trees built from `tests/fixtures/a1314_iso` (anonymised MAC); `/dev/hidraw7` is an empty regular file standing for the keyboard (**any write makes it grow**, which fails the run), `/dev/hidraw3` a fake mouse that must never be opened; `/usr/lib/apple-kb-monitor` is hidden and `pkexec` is `false`; a private session bus and a private "system" bus with a fake BlueZ / UPower (`fakes.py`). Scenarios (`e2e.py`): `responsive`, `open_close`, `daemon_absent`, `daemon_slow`, `daemon_dies`, `daemon_garbage`, `history_50k`, `history_corrupt`, `resize`, `spy`, `screens`; `kcm.py` drives the System Settings module. Exit 0 all passed, 1 one failed (message and capture path printed, details in `DIR/<scenario>/result.json`), 77 a tool is missing (skipped, never a silent pass). `known-failures.tsv` lists scenarios failing on a known open bug: they are run and reported `KNOWN-FAIL` without failing the pipeline, and the run says when one passes again. Reference screenshots live in `refs/` (`--update-refs`).

## Continuous self-check: `akmctl selftest`

`apple-kb-monitor-selfcheck.timer` runs `akmctl selftest --notify` 5 min after login and every 15 min (`Nice=10`, idle I/O). Controls: `daemon` (on the bus, `GetState` within 5 s), `daemon-restarts`, `freshness`, `versions`, `link` (`akmctl doctor`), `journal-daemon`, `panic`, `journal-bluetooth`, `coredumps`, `ui-heartbeat` (window frozen > 10 s), `ui-cpu`, `disk`, `history`. Verdict `ok` / `info` / `warn` / `bad`; result in `~/.local/state/apple-kb-monitor/selfcheck.json`; a desktop notification for each **new** grave problem; optionally one Gitea issue (`--gitea-issue`). By hand: `akmctl selftest`, `akmctl selftest --json --no-save`.

## Git guard: `.githooks/pre-push`

```bash
git config core.hooksPath .githooks     # once per clone
```

The hook runs `scripts/ci-local.sh --fast` (versions, secrets, fmt on changed files, clippy, the whole test suite with a 600 s hang timeout, shell) and **refuses the push** on a failure; it also refuses to run with uncommitted changes in tracked files (it would not test what is pushed). Bypass for an emergency only: `git push --no-verify`, with the reason in the commit.

## CI: `.gitea/workflows/ci.yml`

Five jobs, no hardware: `rust` (build, clippy `-D warnings`, test, `cargo deny`), `shell-c-udev` (single version, no interpreter dependency in the package, shellcheck, security files, `gcc -Werror`, `udevadm verify`), `reverse-tools` (`py_compile` and fixture tests of `tests/live/re`), `qa-static` (`ci-local.sh` static steps, report kept as artefact), `e2e` (Xvfb, simulated keyboard). Needs a Gitea Actions runner with Docker.

## Fixtures (`tests/fixtures/`)

`a1314_iso/`: captured on the real A1314 ISO on 2026-10-01, read-only, without sudo (HID descriptor of 224 bytes, uevent of HID / hidraw / power_supply, `bluetoothctl info`, `upower -i`, udev properties, evdev attributes; the real `0x4C` fingerprint was removed, #210). `synthetic/`: raw GET_REPORT answers. `devname/`: the Lion `setDeviceName:` frames established by disassembly. `models/`: uevent of the other product ids. Provenance: `tests/fixtures/README.md`.

## On the real keyboard, read-only

| Check | Command |
|---|---|
| Link and configuration | `akmctl doctor` (`sudo akmctl doctor` adds the stored vs kernel link-key comparison) |
| Health | `akmctl selftest` |
| Percentage consistency sysfs / UPower / BlueZ / CLI, hidraw rights, `uaccess` tag, services | `tests/live/check_keyboard.sh [--mac AA:BB:..] [--tolerance N] [--strict]`: `PASS\|FAIL\|WARN\|SKIP` per control, exit 0 without `FAIL`, 2 if the keyboard is absent |
| Keys | `akmctl keys --check`, then the checklist of [TOUCHES.md](TOUCHES.md) §7 |
| The three safe reports | `akmctl dump` (press a key first) |
| Sleep / wake byte without sending | `akmctl hid-control suspend --dry-run` |
| Reconnection probe | `tests/live/reconnect_probe.sh` (observes; see [RECONNEXION-PAIRAGE.md](RECONNEXION-PAIRAGE.md) §7 for the controlled test, to be run with the manager) |

The reverse-engineering tools of `tests/live/re/` read the hardware and are **not** installed by the package; their compile and fixture tests run in CI without a keyboard.

## Gaps

Hardware validation of 16 of the 17 models ([#23](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/23)); the effect on the keyboard of `WillShutdown`, `RecantConnection` and the sleep / wake bytes is not observable from the host; the name write (`0x55`) was never exercised on hardware ([#248](https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues/248)).
