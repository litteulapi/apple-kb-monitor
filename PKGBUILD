# Maintainer: Han <han@agenceapi.com>
pkgname=apple-kb-monitor
pkgver=3.0.0
pkgrel=1
pkgdesc="Full telemetry + key mapping for Apple Wireless Keyboards (BCM2042/BCM20733) — battery, voltage, RSSI, DDC brightness, MQTT Home Assistant, KDE integration"
arch=('x86_64')
url="https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor"
license=('GPL-2.0-or-later')
depends=('python' 'bluez' 'dbus' 'python-dbus-fast' 'python-dbus' 'keyd')
makedepends=('rust' 'gcc')
optdepends=(
    'bluez-utils: bluetoothctl CLI for BT management'
    'libnotify: desktop notifications on low battery'
    'python-paho-mqtt: MQTT Home Assistant integration'
    'mosquitto: MQTT broker (local or for testing)'
    'python-pyside6: legacy apihub-settings desktop app'
)
install=apple-kb-monitor.install
source=(
    'apple-kb-monitor'
    'apihub-settings'
    'rssi-helper.c'
    'mqtt-bridge.py'
    'config.toml.example'
    'systemd/apple-kb-monitor.service'
    'systemd/apple-brightness.service'
    'systemd/mqtt-bridge.service'
    'udev/99-apple-kb-hidraw.rules'
    'keyd/apple-keyboard.conf'
    'modprobe/hid_apple.conf'
    'kde/shortcuts/apple-brightness-daemon'
    'kde/shortcuts/apple-brightness-down'
    'kde/shortcuts/apple-brightness-up'
    'kde/DeviceItem.qml'
    'icons/apihub-scarab.svg'
    'apihub-app.desktop'
)
sha256sums=('SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP')

build() {
    # RSSI helper (C)
    gcc -O2 -Wall -o rssi-helper "$srcdir/rssi-helper.c"

    # ddc-tool (Rust)
    cd "$srcdir/../ddc-tool"
    cargo build --release

    # apihub-app (Rust GUI)
    cd "$srcdir/../apihub-app"
    cargo build --release
}

package() {
    # ── Binaries ────────────────────────────────────────────────────────
    install -Dm755 "$srcdir/apple-kb-monitor"                  "$pkgdir/usr/bin/apple-kb-monitor"
    install -Dm755 "$srcdir/apihub-settings"                   "$pkgdir/usr/bin/apihub-settings"
    install -Dm755 "$srcdir/../ddc-tool/target/release/ddc-tool" "$pkgdir/usr/bin/ddc-tool"
    install -Dm755 "$srcdir/../apihub-app/target/release/apihub-app" "$pkgdir/usr/bin/apihub-app"

    # RSSI helper (needs setcap post-install)
    install -Dm755 rssi-helper                                 "$pkgdir/usr/lib/apple-kb-monitor/rssi-helper"

    # MQTT bridge
    install -Dm755 "$srcdir/mqtt-bridge.py"                    "$pkgdir/usr/lib/apple-kb-monitor/mqtt-bridge.py"

    # ── Config ──────────────────────────────────────────────────────────
    install -Dm644 "$srcdir/config.toml.example"               "$pkgdir/etc/apple-kb-monitor/config.toml.example"

    # ── Brightness scripts ──────────────────────────────────────────────
    install -Dm755 "$srcdir/apple-brightness-daemon"           "$pkgdir/usr/bin/apple-brightness-daemon"
    install -Dm755 "$srcdir/apple-brightness-down"             "$pkgdir/usr/bin/apple-brightness-down"
    install -Dm755 "$srcdir/apple-brightness-up"               "$pkgdir/usr/bin/apple-brightness-up"

    # ── systemd user services ───────────────────────────────────────────
    install -Dm644 "$srcdir/apple-kb-monitor.service"          "$pkgdir/usr/lib/systemd/user/apple-kb-monitor.service"
    install -Dm644 "$srcdir/apple-brightness.service"          "$pkgdir/usr/lib/systemd/user/apple-brightness.service"
    install -Dm644 "$srcdir/mqtt-bridge.service"               "$pkgdir/usr/lib/systemd/user/mqtt-bridge.service"

    # ── udev + keyd + modprobe ──────────────────────────────────────────
    install -Dm644 "$srcdir/99-apple-kb-hidraw.rules"          "$pkgdir/usr/lib/udev/rules.d/99-apple-kb-hidraw.rules"
    install -Dm644 "$srcdir/apple-keyboard.conf"               "$pkgdir/etc/keyd/apple-keyboard.conf"
    install -Dm644 "$srcdir/hid_apple.conf"                    "$pkgdir/etc/modprobe.d/hid_apple.conf"

    # ── KDE integration ─────────────────────────────────────────────────
    install -Dm644 "$srcdir/DeviceItem.qml"                    "$pkgdir/usr/share/apple-kb-monitor/kde/DeviceItem.qml"
    install -Dm644 "$srcdir/apihub-scarab.svg"                 "$pkgdir/usr/share/icons/hicolor/scalable/apps/apihub-scarab.svg"

    # ── Desktop entry ───────────────────────────────────────────────────
    install -Dm644 "$srcdir/apihub-app.desktop"                "$pkgdir/usr/share/applications/apihub-app.desktop"

    # ── Plasma widget ───────────────────────────────────────────────────
    local plasma_dir="$pkgdir/usr/share/plasma/plasmoids/com.agenceapi.devicehub"
    install -dm755 "$plasma_dir/contents/ui"
    install -Dm644 "$srcdir/../plasma/com.agenceapi.devicehub/metadata.json" "$plasma_dir/metadata.json"
    install -Dm644 "$srcdir/../plasma/com.agenceapi.devicehub/contents/ui/"*.qml "$plasma_dir/contents/ui/"
}
