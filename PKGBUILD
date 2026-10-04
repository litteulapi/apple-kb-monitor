# Maintainer: litteulapi <124197991+litteulapi@users.noreply.github.com>
pkgname=apple-kb-monitor
pkgver=3.3.0
pkgrel=1
pkgdesc="Telemetry and key mapping for Apple Bluetooth keyboards (BCM2042/BCM20733): battery, voltage, RSSI, BlueZ battery provider, KDE integration"
arch=('x86_64')
url="https://github.com/litteulapi/apple-kb-monitor"
license=('GPL-2.0-or-later' 'OFL-1.1')
depends=('bluez' 'polkit' 'dbus' 'systemd'
         # System Settings module "Clavier Apple" (kcm/): Plasma 6 only
         # loads a KCM from a plugin, linked to these (QML pages inside)
         'qt6-base' 'qt6-declarative' 'kcmutils' 'ki18n' 'kcoreaddons' 'kirigami'
         # QML org.kde.plasma.* (widget) and org.kde.plasma.workspace.dbus
         # (widget and KCM data layer); icons under hicolor
         'libplasma' 'plasma-workspace' 'hicolor-icon-theme')
makedepends=('rust' 'cmake' 'extra-cmake-modules' 'git' 'libcap')
optdepends=(
    'bluez-utils: bluetoothctl CLI for BT management'
    'keyd: optional F3-F6 macros (example config in /usr/share/doc/apple-kb-monitor/examples/keyd)'
)
backup=('etc/modprobe.d/hid_apple.conf' 'etc/apple-kb-monitor/hid-suspend.conf')
install=apple-kb-monitor.install
# makepkg only resolves local sources by basename in $startdir, so files in
# subdirectories (systemd/, udev/, ...) are installed directly from $startdir.
#
# Sources and integrity. This PKGBUILD lives INSIDE the source tree and
# builds that tree (release packages: scripts/release-build.sh; the AUR
# variant that clones the public repository instead is packaging/aur-git,
# generated from this file by scripts/gen-aur-git.py), so there is nothing to
# download and nothing to checksum: source=() is empty, every file is installed
# from $startdir (a local source would need its real sha256, never SKIP; checked
# by tests/check-pkgbuild.sh). What a published source
# repository would allow, and what cannot be done without one:
#   - source=("git+$url.git#tag=v$pkgver") with its b2sum, or a release tarball;
#   - building in "$srcdir" only, hence in a clean chroot (extra-x86_64-build):
#     today build() and package() read "$startdir", which a chroot build
#     does not have;
#   - a pkgver() checked against the tag.
# A local `git+file://$startdir` source is not used either: it would package
# HEAD and silently drop the working copy that scripts/ci-local.sh packages.
# What is done meanwhile: the Rust dependencies are the ones of Cargo.lock and
# no others (`cargo fetch --locked` in prepare(), `cargo build --frozen` in
# build(), offline: a Cargo.lock that does not match Cargo.toml fails the build instead
# of being silently rewritten), each crate being verified by cargo against the
# checksum recorded in Cargo.lock.
source=()
sha256sums=()

# A package must be the image of ONE commit: on 2026-10-03 two different
# packages were installed under the same 3.1.0-26, the second one built from
# uncommitted changes, so no commit tells what code runs. The build stops when
# $startdir is not a git work tree or has any change or untracked file (the
# globs of package() would ship an untracked .qml or .svg). AKM_ALLOW_DIRTY=1
# lifts the guard, explicitly: scripts/ci-local.sh does, it packages a copy of
# the working tree on purpose. A release also takes a new pkgrel.
_akm_require_clean_tree() {
    if [ "${AKM_ALLOW_DIRTY:-0}" = 1 ]; then
        warning "AKM_ALLOW_DIRTY=1: this package is not the image of a commit"
        return 0
    fi
    local top here dirty
    here=$(realpath "$startdir")
    top=$(git -C "$startdir" rev-parse --show-toplevel 2>/dev/null) || top=
    if [ -z "$top" ] || [ "$(realpath "$top")" != "$here" ]; then
        error "$startdir is not a git work tree: the package would match no commit"
        plain "Build from a git checkout, or set AKM_ALLOW_DIRTY=1 knowingly."
        return 1
    fi
    # src/ and pkg/ are makepkg's own directories when BUILDDIR is $startdir
    dirty=$(git -C "$startdir" status --porcelain --untracked-files=all -- . ':(exclude)src/' ':(exclude)pkg/')
    if [ -n "$dirty" ]; then
        error "uncommitted changes in $startdir: commit them first, a package must be one commit"
        printf '%s\n' "$dirty" | head -20 >&2
        plain "Building them anyway (local test only): AKM_ALLOW_DIRTY=1 makepkg ..."
        return 1
    fi
    msg2 "source tree: commit $(git -C "$startdir" rev-parse --short HEAD), clean"
}

prepare() {
    _akm_require_clean_tree
    cd "$startdir/apihub-app"
    cargo fetch --locked --target "$(rustc --print host-tuple)"
}

build() {
    # daemon, akmctl, helpers (Rust)
    cd "$startdir/apihub-app"
    # Full version in the binaries (akm_core::PKG_VERSION): akmctl selftest
    # compares it with pacman -Q, release included.
    export AKM_PKG_VERSION="$pkgver-$pkgrel"
    cargo build --frozen --release --workspace --target-dir target

    # rssi-helper (C): the only binary carrying cap_net_admin
    gcc $CFLAGS $LDFLAGS -Wall -Wextra -o "$startdir/rssi-helper" "$startdir/rssi-helper.c"

    # System Settings module: minimal C++ plugin + QML pages + .mo
    cmake -S "$startdir/kcm" -B "$srcdir/kcm-build" -DCMAKE_INSTALL_PREFIX=/usr \
        -DCMAKE_BUILD_TYPE=None -DKDE_INSTALL_USE_QT_SYS_PATHS=ON -DBUILD_TESTING=OFF
    cmake --build "$srcdir/kcm-build"

    # Catalogs of the widget and of akmctl, the daemon and akm-helper
    local po
    mkdir -p "$srcdir/mo/plasma" "$srcdir/mo/apple-kb-monitor"
    for po in "$startdir/plasma/po/"*.po; do
        msgfmt --check -o "$srcdir/mo/plasma/$(basename "$po" .po).mo" "$po"
    done
    for po in "$startdir/po/"*.po; do
        msgfmt --check -o "$srcdir/mo/apple-kb-monitor/$(basename "$po" .po).mo" "$po"
    done
}

package() {
    # ── Binaries ────────────────────────────────────────────────────────
    install -Dm755 "$startdir/apihub-app/target/release/apple-kb-monitord" "$pkgdir/usr/bin/apple-kb-monitord"
    install -Dm755 "$startdir/apihub-app/target/release/akmctl"             "$pkgdir/usr/bin/akmctl"
    # The one privileged program: argv[1] selects the polkit action (exec.argv1)
    install -Dm755 "$startdir/apihub-app/target/release/akm-helper"         "$pkgdir/usr/lib/apple-kb-monitor/akm-helper"

    # akmctl: shell completions and man page, generated by the binary itself
    "$pkgdir/usr/bin/akmctl" completions bash | install -Dm644 /dev/stdin "$pkgdir/usr/share/bash-completion/completions/akmctl"
    "$pkgdir/usr/bin/akmctl" completions zsh  | install -Dm644 /dev/stdin "$pkgdir/usr/share/zsh/site-functions/_akmctl"
    "$pkgdir/usr/bin/akmctl" completions fish | install -Dm644 /dev/stdin "$pkgdir/usr/share/fish/vendor_completions.d/akmctl.fish"
    "$pkgdir/usr/bin/akmctl" man              | install -Dm644 /dev/stdin "$pkgdir/usr/share/man/man1/akmctl.1"

    # polkit: one action per command of akm-helper
    install -Dm644 "$startdir/polkit/com.agenceapi.AppleKbMonitor.policy" "$pkgdir/usr/share/polkit-1/actions/com.agenceapi.AppleKbMonitor.policy"

    # ── RSSI helper: root:root 0755 + cap_net_admin, carried by the package
    #    (xattr); it answers only for a connected Apple keyboard ──
    install -Dm755 "$startdir/rssi-helper"                          "$pkgdir/usr/lib/apple-kb-monitor/rssi-helper"
    setcap cap_net_admin+ep "$pkgdir/usr/lib/apple-kb-monitor/rssi-helper"

    # ── systemd user service: apple-kb-monitord (single keyboard owner) ──
    install -Dm644 "$startdir/systemd/apple-kb-monitord.service"         "$pkgdir/usr/lib/systemd/user/apple-kb-monitord.service"
    # User units are enabled by a preset, never by vendor .wants links (Arch,
    # systemd.preset(5)): the user enables them, `disable` works.
    install -Dm644 "$startdir/systemd/90-apple-kb-monitor.preset"        "$pkgdir/usr/lib/systemd/user-preset/90-apple-kb-monitor.preset"
    install -Dm644 "$startdir/dbus/com.agenceapi.AppleKbMonitor1.service" "$pkgdir/usr/share/dbus-1/services/com.agenceapi.AppleKbMonitor1.service"
    # ── WillShutdown at shutdown (what macOS sends): the unit stays active
    #    and its ExecStop= asks the daemon; [apple] will_shutdown = false in
    #    config.toml sends nothing; opt out of the unit too:
    #    systemctl --user disable --now apple-kb-monitor-shutdown.service ──
    install -Dm644 "$startdir/systemd/apple-kb-monitor-shutdown.service" "$pkgdir/usr/lib/systemd/user/apple-kb-monitor-shutdown.service"
    # ── HID_CONTROL SUSPEND (0x13) before sleep / EXIT_SUSPEND (0x14) at wake,
    #    as macOS bluetoothd (docs/HID-SLEEP.md): SYSTEM units (root,
    #    pidfd_getfd on bluetoothd). OFF by default since 2026-10-02 (SUSPEND
    #    measured harmful: the keyboard stays mute), in the packaged file AND in
    #    the code (file or key absent = disabled); opt in:
    #    enabled = true in /etc/apple-kb-monitor/hid-suspend.conf ──
    install -Dm644 "$startdir/systemd/apple-kb-monitor-suspend.service" "$pkgdir/usr/lib/systemd/system/apple-kb-monitor-suspend.service"
    install -Dm644 "$startdir/systemd/apple-kb-monitor-resume.service"  "$pkgdir/usr/lib/systemd/system/apple-kb-monitor-resume.service"
    install -Dm644 "$startdir/systemd/hid-suspend.conf"                 "$pkgdir/etc/apple-kb-monitor/hid-suspend.conf"
    install -dm755 "$pkgdir/usr/lib/systemd/system/sleep.target.wants"
    ln -s ../apple-kb-monitor-suspend.service "$pkgdir/usr/lib/systemd/system/sleep.target.wants/apple-kb-monitor-suspend.service"
    for t in suspend hibernate hybrid-sleep suspend-then-hibernate; do
        install -dm755 "$pkgdir/usr/lib/systemd/system/$t.target.wants"
        ln -s ../apple-kb-monitor-resume.service "$pkgdir/usr/lib/systemd/system/$t.target.wants/apple-kb-monitor-resume.service"
    done
    install -Dm644 "$startdir/docs/HID-SLEEP.md" "$pkgdir/usr/share/doc/apple-kb-monitor/HID-SLEEP.md"
    # ── self-check every 15 min (akmctl selftest), in the preset;
    #    opt out: systemctl --user disable --now apple-kb-monitor-selfcheck.timer ──
    install -Dm644 "$startdir/systemd/apple-kb-monitor-selfcheck.service" "$pkgdir/usr/lib/systemd/user/apple-kb-monitor-selfcheck.service"
    install -Dm644 "$startdir/systemd/apple-kb-monitor-selfcheck.timer"   "$pkgdir/usr/lib/systemd/user/apple-kb-monitor-selfcheck.timer"
    install -Dm644 "$startdir/docs/TESTING.md" "$pkgdir/usr/share/doc/apple-kb-monitor/TESTING.md"

    # ── udev + modprobe (+ keyd example, never active by default) ──
    install -Dm644 "$startdir/udev/70-apple-kb-hidraw.rules"          "$pkgdir/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules"
    install -Dm644 "$startdir/udev/README.md"                         "$pkgdir/usr/share/doc/apple-kb-monitor/udev-README.md"
    install -Dm644 "$startdir/keyd/apple-keyboard.conf"               "$pkgdir/usr/share/doc/apple-kb-monitor/examples/keyd/apple-keyboard.conf"
    install -Dm644 "$startdir/docs/KEYD.md"                           "$pkgdir/usr/share/doc/apple-kb-monitor/KEYD.md"
    # special keys and manual mapping; the hwdb file itself is written
    # only by `akmctl keymap apply`, never by the package (default = no change)
    install -Dm644 "$startdir/docs/KEYS.md"                        "$pkgdir/usr/share/doc/apple-kb-monitor/KEYS.md"
    install -Dm644 "$startdir/modprobe/hid_apple.conf"                    "$pkgdir/etc/modprobe.d/hid_apple.conf"

    # ── Font of the widget: VT323 (SIL Open Font License 1.1), its licence (apihub-app/assets/fonts/OFL.txt).
    install -Dm644 "$startdir/apihub-app/assets/fonts/OFL.txt"         "$pkgdir/usr/share/licenses/apple-kb-monitor/OFL.txt"

    # ── Icons ───────────────────────────────────────────────────────────
    install -Dm644 "$startdir/icons/apihub-scarab.svg"                 "$pkgdir/usr/share/icons/hicolor/scalable/apps/apihub-scarab.svg"

    # ── Notifications and global shortcuts ─────────────────────────────
    # KNotification events (docs/NOTIFICATIONS.md): makes the daemon an
    # application of System Settings > Notifications (popup, sound, history, DND).
    install -Dm644 "$startdir/data/apple-kb-monitor.notifyrc"                    "$pkgdir/usr/share/knotifications6/apple-kb-monitor.notifyrc"
    install -Dm644 "$startdir/data/com.agenceapi.AppleKbMonitor.shortcuts.desktop" "$pkgdir/usr/share/kglobalaccel/com.agenceapi.AppleKbMonitor.shortcuts.desktop"

    # ── System Settings → Input & Output → Keyboard → Apple Keyboard:
    #    lib/qt6/plugins/plasma/kcms/systemsettings/kcm_applekeyboard.so,
    #    share/applications/kcm_applekeyboard.desktop, one .mo per kcm/po/<lang> ──
    DESTDIR="$pkgdir" cmake --install "$srcdir/kcm-build"
    install -Dm644 "$startdir/docs/KCM.md" "$pkgdir/usr/share/doc/apple-kb-monitor/KCM.md"

    # ── Plasma widget ───────────────────────────────────────────────────
    local plasma_dir="$pkgdir/usr/share/plasma/plasmoids/com.agenceapi.devicehub"
    install -dm755 "$plasma_dir/contents/ui"
    install -Dm644 "$startdir/plasma/com.agenceapi.devicehub/metadata.json" "$plasma_dir/metadata.json"
    for qml in "$startdir/plasma/com.agenceapi.devicehub/contents/ui/"*.qml "$startdir/plasma/com.agenceapi.devicehub/contents/ui/"*.js; do
        install -Dm644 "$qml" "$plasma_dir/contents/ui/$(basename "$qml")"
    done
    # VT323 of the Pip-Boy popup (SIL OFL 1.1, licence installed above),
    # loaded by the widget with FontLoader, never system-wide.
    install -Dm644 "$startdir/plasma/com.agenceapi.devicehub/contents/fonts/VT323-Regular.ttf" \
        "$plasma_dir/contents/fonts/VT323-Regular.ttf"
    # Widget translations: one gettext catalogue per plasma/po/<lang>.po,
    # loaded by Plasma's i18n() from the system locale directory.
    local mo
    for mo in "$srcdir/mo/plasma/"*.mo; do
        install -Dm644 "$mo" \
            "$pkgdir/usr/share/locale/$(basename "$mo" .mo)/LC_MESSAGES/plasma_applet_com.agenceapi.devicehub.mo"
    done
    # Translations of akmctl, the daemon and akm-helper (built in build()).
    for mo in "$srcdir/mo/apple-kb-monitor/"*.mo; do
        install -Dm644 "$mo" \
            "$pkgdir/usr/share/locale/$(basename "$mo" .mo)/LC_MESSAGES/apple-kb-monitor.mo"
    done
}
