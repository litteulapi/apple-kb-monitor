# Documentation index

One line per document of `docs/`. Language in brackets. **Living** documents are kept in step with the code (`main` @ `8cb044e`, 3.1.0-15); **evidence** documents are dated (reverse engineering, audits, measurements): their measurements and method are never rewritten, but a claim refuted by a later study is corrected in place, and each corrected document opens with one line per correction, "Corrigé le YYYY-MM-DD (issue #N) : …" — their claims are checked against the registry by `scripts/qa_checks.py claims` ([CONTRE-AUDIT.md](CONTRE-AUDIT.md) is the registry itself).

## User guides (living)

| Document | One line |
|---|---|
| [../README.md](../README.md) [EN] | What the project does, what works (with evidence level), install, daily use, KDE, troubleshooting, security, limits, architecture, links |
| [INSTALL.md](INSTALL.md) [EN] | Package build, installed files, what `post_install` does, group `akm`, enabled units, BlueZ / UPower settings, keyd optional, upgrade notices, uninstall |
| [CONFIGURATION.md](CONFIGURATION.md) [EN] | Every key of `config.toml` with its default (`[alerts] [battery] [notifications] [display] [apple]`), the battery estimate, the write locks, the other files (`hid-suspend.conf`, `keymap.toml`, `selfcheck.env`, modprobe, hwdb) |
| [FEATURES.md](FEATURES.md) [EN] | Feature → evidence level → module; CLI commands; D-Bus API; what is not done |
| [TROUBLESHOOTING.md](TROUBLESHOOTING.md) [EN] | Symptom → cause → command: reconnection (switch the keyboard off and on, `akmctl repair`), battery, signal / group `akm`, keys / keyd, daemon, disk and BlueZ |
| [TOUCHES.md](TOUCHES.md) [FR] | Special keys (#247): what each key does, the kernel → evdev → KDE chain, `akmctl keys`, pitfalls, manual mapping without keyd, missing KDE shortcuts, test checklist |
| [KCM.md](KCM.md) [FR] | System Settings module "Apple Keyboard" (#250): pages, writes and authentication, responsiveness, languages, tests, limits |
| [NOTIFICATIONS.md](NOTIFICATIONS.md) [FR] | KNotification integration (#249): the 18 events, buttons, replacement, language, configuration, test |
| [INTEGRATION-KDE.md](INTEGRATION-KDE.md) [FR] | Fixes of the KDE audit: Forget from Plasma (#252), one icon (#253), PowerDevil (#254), two names (#248), F4 / Eject (#247), French (#114), window start (#21) |
| [RECONNEXION-PAIRAGE.md](RECONNEXION-PAIRAGE.md) [FR] | Reconnection, pairing and sleep (#142): symptom, what the errors mean, measurements, root cause, fixes (system, daemon, tools, clean forget), expected behaviour, controlled test, risks |
| [RENOMMER-CLAVIER.md](RENOMMER-CLAVIER.md) [FR] | The two names (#141, #192, #248): alias delivered; name stored in the keyboard: validation, three locks, Lion frames, risks, manager's procedure, rollback |
| [KEYD.md](KEYD.md) [FR] | keyd 2.6.0 crash on `keyd reload` (#246): evidence, reproduction, bisection, fix in the package, keyd optional, upstream report |

## Developer guides (living)

| Document | One line |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) [EN] | Components (daemon, clients, five privileged helpers, KCM, widget), `akm-core` decision modules, daemon loop, data flow, privilege model, installed and repository layout |
| [TESTING.md](TESTING.md) [EN] | `scripts/ci-local.sh` (18 steps), Rust tests and invariants, e2e under Xvfb + bubblewrap, `akmctl selftest`, pre-push hook, CI jobs, fixtures, read-only hardware checks |
| [QA-AUTOMATIQUE.md](QA-AUTOMATIQUE.md) [FR] | Rationale and contract of the automatic QA (#241): pipeline, e2e, self-check controls and heartbeat, git guard, CI, defects caught |
| [PARITE-APPLE.md](PARITE-APPLE.md) [EN] | Parity with Apple: what is sent to the keyboard and nothing more; rules R1-R8 of the model with sources and tests; what is deliberately not implemented; the write doors |
| [FIRMWARE.md](FIRMWARE.md) [FR] | Firmware version check (#227): `0x4F` once per connection, embedded table and its sources, statuses, where it is shown, how to update the table, register classes summary |
| [VEILLE-HID.md](VEILLE-HID.md) [EN] | Sleep and wake HID_CONTROL bytes (#244): why a root helper borrows `bluetoothd`'s socket, safety rules, breaker, triggers, configuration, manual test, limits, tests |
| [ROADMAP-FONCTIONNALITES.md](ROADMAP-FONCTIONNALITES.md) [FR] | Feature roadmap (2026-10-01): starting point, 2026 benchmark, what the hardware allows, prioritised features F01-F46, acceptance criteria, delivery order |
| [../udev/README.md](../udev/README.md) [FR] | The `uaccess` rule (#155) and the accepted keylogger trade-off; multi-user caveat (#204) |
| [../polkit/README.md](../polkit/README.md) | Polkit actions and the optional rules example |
| [../tests/fixtures/README.md](../tests/fixtures/README.md) [FR] | Provenance of the fixtures (real A1314 capture, synthetic, devname, models) |

## Reverse engineering (evidence, dated 2026-10-01)

| Document | One line |
|---|---|
| [HARDWARE-RAPPORTS-HID.md](HARDWARE-RAPPORTS-HID.md) [FR] | Map of the HID Feature reports of the A1314 (BCM2042) read on the real keyboard, with evidence level per report |
| [HARDWARE-ENTREES-MODELES.md](HARDWARE-ENTREES-MODELES.md) [FR] | Input reports of the A1314, keyd remapping as read in the code, LED path, model × function matrix (what is tested) |
| [RE-HID-EXHAUSTIF.md](RE-HID-EXHAUSTIF.md) [FR] | Exhaustive read-only HID pass (256 Feature ids, Input GET) on the A1314 ISO |
| [RE-LIAISON-BLUETOOTH.md](RE-LIAISON-BLUETOOTH.md) [FR] | The BR/EDR HID link of the A1314: SDP, L2CAP channels, sniff, supervision, what `btmon` shows |
| [RE-COMMANDES-VENDEUR.md](RE-COMMANDES-VENDEUR.md) [FR] | Vendor commands, write-only registers and hidden reports (documentary, no hardware access) |
| [RE-PILOTE-MACOS.md](RE-PILOTE-MACOS.md) [FR] | Static analysis of Apple's macOS drivers: what macOS sends to the A1314 |
| [RE-MACOS-SILICON.md](RE-MACOS-SILICON.md) [FR] | What the arm64e code actually executed on Apple Silicon adds to the x86_64 analysis |
| [RE-GHIDRA-IOBLUETOOTH.md](RE-GHIDRA-IOBLUETOOTH.md) [FR] | Ghidra decompilation of the macOS 26.5 Bluetooth HID stack (IOBluetooth, `bluetoothd`, CoreBluetooth) for the A1314 |
| [RE-GHIDRA-KEXT.md](RE-GHIDRA-KEXT.md) [FR] | Ghidra decompilation of the macOS 26.5 kernel Bluetooth HID driver (PID `0x0256`): breaker, sleep / wake, battery, `WillShutdown` |
| [RE-PILOTES-ANCIENS.md](RE-PILOTES-ANCIENS.md) [FR] | Apple drivers and updaters of 2009-2012 for the A1314 (firmware `0x0050`, updater `Parameters.plist`) |
| [RE-SYSTEMES-ANCIENS.md](RE-SYSTEMES-ANCIENS.md) [FR] | Mac OS X 10.2.8 to 10.6.8: which component calls which register of the A1314 |
| [RE-NOM-PROPRE-E1.md](RE-NOM-PROPRE-E1.md) [FR] | Experiment E1: the exact SET Feature `0x55` frames Lion sends to rename a keyboard, by disassembly |
| [RE-FIRMWARE-EXTRACTION.md](RE-FIRMWARE-EXTRACTION.md) [FR] | Obtaining and disassembling the BCM2042 firmware: feasibility and plan (documentary) |
| [RE-FIRMWARE-MAINTENANCE.md](RE-FIRMWARE-MAINTENANCE.md) [FR] | Hidden functions, maintenance and diagnostic modes of the BCM2042 keyboards (documentary) |
| [VERIF-BATTERIE.md](VERIF-BATTERIE.md) [FR] | Adversarial verification of the battery percentage: firmware interpolation, steps at reconnection, voltage vs real charge |
| [CONTRE-AUDIT.md](CONTRE-AUDIT.md) [FR] | Counter-audit of the reverse-engineering documents: the registry of refuted and confirmed claims used by `qa_checks.py claims` |

## Audits and reviews (evidence, dated 2026-10-01)

| Document | One line |
|---|---|
| [AUDIT-SECURITE-2.md](AUDIT-SECURITE-2.md) [FR] | Independent security counter-audit (#202-#212, #214): findings and their status |
| [AUDIT-DECODAGE-HID.md](AUDIT-DECODAGE-HID.md) [FR] | Audit of every code path that queries and decodes the keyboard |
| [AUDIT-INTEGRATION-KDE.md](AUDIT-INTEGRATION-KDE.md) [FR] | Read-only audit of the Plasma 6 integration on the real session (led to #252-#254, #248, #247, #114) |
| [AUDIT-OUTILLE.md](AUDIT-OUTILLE.md) [FR] | Tooled audit: miri, udeps, cargo-fuzz, proptest, UI harness |
| [AUDIT-UI.md](AUDIT-UI.md) [FR] | Window audit: freezes, per-frame costs, degenerate cases (led to #230-#236) |
| [REVUE-ARCHITECTURE-GLOBALE.md](REVUE-ARCHITECTURE-GLOBALE.md) [FR] | Adversarial architecture review: scope reduction then split (#59-#62) |
| [REVUE-ARCHITECTURE-CLAVIER.md](REVUE-ARCHITECTURE-CLAVIER.md) [FR] | Architecture review of the keyboard driver: keep the principle, rebuild the backbone |
| [REVUE-CORRECTIFS.md](REVUE-CORRECTIFS.md) [FR] | Adversarial review of the keyboard fixes (#72-#81) |
| [REVUE-UI-TRAY.md](REVUE-UI-TRAY.md) [FR] | UI and tray review: no toolkit change, the centre of gravity moves to the daemon and Plasma |

`captures/`: screenshots of the window before / after the 3.1.0-8 fixes (#230: overlap, freeze, narrow window, light and dark themes) and `2026-10-01-coupure-12h13.txt`, the link loss captured live (RECONNEXION-PAIRAGE.md §3.6). `../kcm/captures/`: the five pages of the System Settings module.
