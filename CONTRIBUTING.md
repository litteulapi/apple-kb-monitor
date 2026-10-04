# Contributing to apple-kb-monitor

Thanks for helping! This project targets **Arch Linux / Manjaro** with **KDE
Plasma 6** and the aluminium Apple Wireless Keyboard (A1314 and its family).
Everything below runs as a normal user and never writes to a real keyboard.

## Reporting a bug

Open a [bug report](https://github.com/litteulapi/apple-kb-monitor/issues/new/choose)
and paste the output of:

```bash
akmctl doctor
akmctl status --json
journalctl --user -u apple-kb-monitord.service -b --no-pager | tail -n 100
```

These outputs contain your keyboard's Bluetooth address: replace it with
`AA:BB:CC:DD:EE:FF` if you prefer, and read the journal excerpt before posting. Security problems go through [SECURITY.md](SECURITY.md),
not public issues.

## Building

Dependencies (Arch / Manjaro):

```bash
sudo pacman -S --needed base-devel git rust gcc gettext cmake extra-cmake-modules libcap \
    bluez polkit dbus qt6-base qt6-declarative kcmutils ki18n kcoreaddons kirigami \
    libplasma plasma-workspace
```

Build and run the Rust workspace (daemon `apple-kb-monitord`, CLI `akmctl`,
helper `akm-helper`):

```bash
cd apihub-app
cargo build --locked --workspace
cargo run -p akmctl -- status
```

Build the Arch package from your clone (the PKGBUILD refuses an uncommitted
tree: commit first, then build):

```bash
makepkg -si
```

## Testing

```bash
cd apihub-app
cargo fmt --all --check          # the whole workspace must be rustfmt-clean
cargo clippy --locked --workspace --all-targets -- -D warnings
AKM_REQUIRE_DBUS=1 cargo test --locked --workspace
```

The D-Bus tests start their own private `dbus-daemon`; they never touch your
session bus or a real keyboard. The whole pipeline (static checks, KCM build,
QML lint, catalogs, shellcheck, package, end-to-end tests under Xvfb) is one
command, with a report in `scripts/out/latest/report.txt`:

```bash
scripts/ci-local.sh            # everything
scripts/ci-local.sh --fast     # before a push: the steps tagged `fast` in --list
scripts/ci-local.sh --list     # the steps
```

Missing tools (for example `qmllint` or `makepkg`) make a step **skip**, not
fail. See [docs/TESTING.md](docs/TESTING.md) for what each step checks.

## Translations

User-facing text lives in gettext catalogs, 19 languages besides English
(cs, da, de, es, fi, fr, it, ja, ko, nb, nl, pl, pt, pt_BR, ru, sv, tr, uk, zh_CN):

| Component | Template | Catalogs | Refresh after a source change |
|---|---|---|---|
| `akmctl`, daemon, notifications | `po/apple-kb-monitor.pot` | `po/<lang>.po` | `scripts/update-po.sh` |
| System Settings module | `kcm/po/kcm_applekeyboard.pot` | `kcm/po/<lang>/kcm_applekeyboard.po` | `kcm/Messages.sh` |
| Plasma widget | `plasma/po/plasma_applet_com.agenceapi.devicehub.pot` | `plasma/po/<lang>.po` | `plasma/Messages.sh` (check: `python3 plasma/tests/check_i18n.py`) |

To improve a translation, edit the `msgstr` lines of your language with any
PO editor (Lokalize, Poedit) or a text editor, then check it:

```bash
msgfmt --check --statistics -o /dev/null po/fr.po
sh tests/check-po.sh             # akmctl/daemon catalogs complete and valid
bash kcm/Messages.sh --check      # System Settings module catalogs
python3 plasma/tests/check_i18n.py  # widget catalogs
```

To add a language, copy the `.pot` to the new catalog, set its `Language:`
header and translate every entry; keep placeholders (`{}` / `{name}` in the
daemon and akmctl, `%1`…`%9` in the module and the widget) and plural forms
intact. Partial catalogs are welcome in a draft pull request.

## Pull requests

- One topic per pull request, with tests for the behaviour you change.
- `scripts/ci-local.sh --fast` passes; CI runs the same checks in an Arch container.
- Never add code that writes to the keyboard outside the named operations of
  `akm-core` (`docs/APPLE-PARITY.md` explains why).
- Keep comments short: one line saying why, not what.

### Commit style

- English, imperative mood, a summary line under ~72 characters:
  `Fix the battery estimate after a battery swap`.
- Reference the issue it fixes in the summary or body (`Fixes #123`).
- A body when the why is not obvious; wrap it at 72 columns.

## License

By contributing you agree that your contribution is licensed under
[GPL-2.0-or-later](LICENSE), like the rest of the project.
