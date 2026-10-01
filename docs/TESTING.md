# Testing

| Suite | Location | Command |
|---|---|---|
| Rust unit tests | `#[cfg(test)]` modules in `apihub-app/src/` (keyboard model table and parsing, `power.rs` on a fake sysfs tree, `rssi.rs` with stub helpers, `bluez.rs` state machine) | `cd apihub-app && cargo test` |
| Python CLI tests | `tests/test_apple_kb.py` | `python -m pytest tests/` |
| Live read-only harness | `tests/live/check_keyboard.sh` | see below |

Build checks: `cd apihub-app && cargo build --release && cargo clippy --all-targets -- -D warnings`; `gcc -Wall -Wextra -Werror -o /tmp/rssi-helper rssi-helper.c`; `udevadm verify udev/70-apple-kb-hidraw.rules`.

Packaging check: `makepkg -f --nodeps --skipinteg` in a copy of the tree (installs nothing); regenerate `.SRCINFO` with `makepkg --printsrcinfo > .SRCINFO`.

Gaps (Gitea issues): compatibility of 16 of the 17 keyboard models unverified (#23).

Manual hardware checklist: `apple-kb-monitor --once/--status/--dump`, apihub-app Keyboard and Diag tabs, tray menu, RSSI, `busctl tree org.bluez` shows `Battery1`.

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

`.gitea/workflows/ci.yml` (issues #9, #24) : `cargo build/clippy -D warnings/test` sur chaque crate, `py_compile`, `pytest`,
`shellcheck`, `gcc -Wall -Wextra -Werror rssi-helper.c`, `udevadm verify`. Le job ne requiert aucun matériel.
Il nécessite un runner Gitea Actions avec Docker (images `rust:1-bookworm`, `debian:bookworm`).
