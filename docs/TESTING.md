# Testing

| Suite | Location | Count | Command |
|---|---|---|---|
| Rust DDC tests | `apihub-app/tests/test_ddc.rs` | 27 `#[test]` | `cd apihub-app && cargo test` |
| Python CLI tests | `tests/test_apple_kb.py` | 46 `test_*` | `python -m pytest tests/` |

Build checks: `cd apihub-app && cargo build --release && cargo clippy`; `cd ddc-tool && cargo build --release`.

Gaps (Gitea issues): no unit tests inside `apihub-app/src` or `ddc-tool` (#11), no CI (#9), no hardware-less integration harness (#24), compatibility of 9 of the 10 keyboard models unverified (#23).

Manual hardware checklist: `apple-kb-monitor --once/--status/--dump`, apihub-app tabs (Keyboard values, Display sliders, tray menu), F1/F2 brightness, MQTT discovery visible in Home Assistant, `ddc-tool json <bus>`.

Safety: do not run write tests against a production monitor without restoring values; do not send factory reset VCPs (0x04/0x05/0x08) in automated tests.

## Tests sur matériel réel (lecture seule)

`tests/live/check_keyboard.sh [--mac AA:BB:CC:DD:EE:FF] [--tolerance N] [--strict] [--bin PATH]`

Sans sudo, sans écriture sur le clavier, sans toucher aux services. Retrouve hidraw / evdev / power_supply
à partir du MAC (`HID_UNIQ`), compare le pourcentage batterie entre sysfs (référence), UPower, BlueZ `Battery1`
et `apple-kb-monitor --json` (écart toléré : `--tolerance`, 5 points par défaut, ou `KB_TOLERANCE`), vérifie
les droits `/dev/hidraw*`, la règle udev `uaccess` installée et son tag effectif, keyd et les services.
Une ligne `PASS|FAIL|WARN|SKIP` par contrôle ; code retour 0 s'il n'y a aucun `FAIL` (`--strict` : WARN aussi),
1 sinon, 2 si le clavier n'est pas connecté. Variables : `KB_MAC`, `KB_TOLERANCE`, `KB_BIN`, `KB_RUST_BIN`.

## Fixtures sans matériel

`tests/fixtures/a1314_iso/` : capture réelle d'un A1314 ISO (descripteur HID 224 o, uevent HID/hidraw/power_supply,
`bluetoothctl info`, `upower -i`, propriétés udev, attributs evdev). `tests/fixtures/synthetic/` : GET_REPORT bruts
synthétiques (illisibles sans accès hidraw). Détail et provenance : `tests/fixtures/README.md`.

## CI

`.gitea/workflows/ci.yml` (issue #9) : `cargo build/clippy -D warnings/test` sur chaque crate, `py_compile`, `pytest`,
`shellcheck`, `gcc -Wall -Wextra -Werror rssi-helper.c`, `udevadm verify`. Le job ne requiert aucun matériel.
Il nécessite un runner Gitea Actions avec Docker (images `rust:1-bookworm`, `debian:bookworm`).
