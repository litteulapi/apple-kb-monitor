# Testing

| Suite | Location | Count | Command |
|---|---|---|---|
| Rust DDC tests | `apihub-app/tests/test_ddc.rs` | 27 `#[test]` | `cd apihub-app && cargo test` |
| Python CLI tests | `tests/test_apple_kb.py` | 46 `test_*` | `python -m pytest tests/` |

Build checks: `cd apihub-app && cargo build --release && cargo clippy`; `cd ddc-tool && cargo build --release`.

Gaps (Gitea issues): no unit tests inside `apihub-app/src` or `ddc-tool` (#11), no CI (#9), no hardware-less integration harness (#24), compatibility of 9 of the 10 keyboard models unverified (#23).

Manual hardware checklist: `apple-kb-monitor --once/--status/--dump`, apihub-app tabs (Keyboard values, Display sliders, tray menu), F1/F2 brightness, MQTT discovery visible in Home Assistant, `ddc-tool json <bus>`.

Safety: do not run write tests against a production monitor without restoring values; do not send factory reset VCPs (0x04/0x05/0x08) in automated tests.
