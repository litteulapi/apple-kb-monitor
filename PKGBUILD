# Maintainer: Han <han@agenceapi.com>
pkgname=apple-kb-monitor
pkgver=3.0.0
pkgrel=1
pkgdesc="Full telemetry + key mapping for Apple Wireless Keyboards (BCM2042/BCM20733) — battery, voltage, RSSI, DDC brightness, MQTT Home Assistant, KDE integration"
arch=('x86_64')
url="https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor"
license=('GPL-2.0-or-later')
depends=('python' 'python-dbus-fast' 'bluez' 'keyd')
makedepends=('rust' 'gcc')
optdepends=(
    'bluez-utils: bluetoothctl CLI for BT management'
    'libnotify: desktop notifications on low battery'
    'python-paho-mqtt: MQTT publishing from the CLI daemon (--mqtt)'
)
options=('!lto')   # C-LTO objects from ring break the Rust link (rust-lld)
backup=('etc/keyd/apple-keyboard.conf' 'etc/modprobe.d/hid_apple.conf')
install=apple-kb-monitor.install
# makepkg only resolves local sources by basename in $startdir, so files in
# subdirectories (systemd/, udev/, ...) are installed directly from $startdir.
source=(
    'apple-kb-monitor'
    'config.toml.example'
    'apihub-app.desktop'
)
sha256sums=('SKIP' 'SKIP' 'SKIP')

build() {
    # ddc-tool (Rust)
    cd "$startdir/ddc-tool"
    cargo build --release --target-dir target

    # apihub-app (Rust GUI)
    cd "$startdir/apihub-app"
    cargo build --release --target-dir target
}

package() {
    # ── Binaries ────────────────────────────────────────────────────────
    install -Dm755 "$srcdir/apple-kb-monitor"                        "$pkgdir/usr/bin/apple-kb-monitor"
    install -Dm755 "$startdir/ddc-tool/target/release/ddc-tool"     "$pkgdir/usr/bin/ddc-tool"
    install -Dm755 "$startdir/apihub-app/target/release/apihub-app" "$pkgdir/usr/bin/apihub-app"

    # ── Config ──────────────────────────────────────────────────────────
    install -Dm644 "$srcdir/config.toml.example"               "$pkgdir/etc/apple-kb-monitor/config.toml.example"

    # ── systemd user service (CLI daemon) ───────────────────────────────
    install -Dm644 "$startdir/systemd/apple-kb-monitor.service"          "$pkgdir/usr/lib/systemd/user/apple-kb-monitor.service"

    # ── udev + keyd + modprobe ──────────────────────────────────────────
    install -Dm644 "$startdir/udev/99-apple-kb-hidraw.rules"          "$pkgdir/usr/lib/udev/rules.d/99-apple-kb-hidraw.rules"
    install -Dm644 "$startdir/keyd/apple-keyboard.conf"               "$pkgdir/etc/keyd/apple-keyboard.conf"
    install -Dm644 "$startdir/modprobe/hid_apple.conf"                    "$pkgdir/etc/modprobe.d/hid_apple.conf"

    # ── KDE integration ─────────────────────────────────────────────────
    install -Dm644 "$startdir/kde/DeviceItem.qml"                    "$pkgdir/usr/share/apple-kb-monitor/kde/DeviceItem.qml"
    install -Dm644 "$startdir/icons/apihub-scarab.svg"                 "$pkgdir/usr/share/icons/hicolor/scalable/apps/apihub-scarab.svg"

    # ── Desktop entry ───────────────────────────────────────────────────
    install -Dm644 "$srcdir/apihub-app.desktop"                "$pkgdir/usr/share/applications/apihub-app.desktop"

    # ── D-Bus policy ────────────────────────────────────────────────────
    install -Dm644 "$startdir/dbus/com.agenceapi.AppleKbMonitor.conf" "$pkgdir/etc/dbus-1/system.d/com.agenceapi.AppleKbMonitor.conf"

    # ── Plasma widget ───────────────────────────────────────────────────
    local plasma_dir="$pkgdir/usr/share/plasma/plasmoids/com.agenceapi.devicehub"
    install -dm755 "$plasma_dir/contents/ui"
    install -Dm644 "$startdir/plasma/com.agenceapi.devicehub/metadata.json" "$plasma_dir/metadata.json"
    for qml in "$startdir/plasma/com.agenceapi.devicehub/contents/ui/"*.qml; do
        install -Dm644 "$qml" "$plasma_dir/contents/ui/$(basename "$qml")"
    done
}
