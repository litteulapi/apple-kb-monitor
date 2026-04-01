# Maintainer: Han <han@agenceapi.com>
pkgname=apple-kb-monitor
pkgver=2.4.0
pkgrel=1
pkgdesc="Full telemetry + key mapping for Apple Wireless Keyboards (BCM2042/BCM20733) — battery, voltage, RSSI, DDC brightness, KDE integration via BlueZ Battery Provider + keyd"
arch=('x86_64')
url="https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor"
license=('GPL-2.0-or-later')
depends=('python' 'bluez' 'dbus' 'python-dbus-fast' 'python-dbus' 'keyd' 'ddcutil')
optdepends=(
    'bluez-utils: bluetoothctl CLI for BT management'
    'libnotify: desktop notifications on low battery'
)
install=apple-kb-monitor.install
source=(
    'apple-kb-monitor'
    'rssi-helper.c'
    'systemd/apple-kb-monitor.service'
    'systemd/apple-brightness.service'
    'udev/99-apple-kb-hidraw.rules'
    'keyd/apple-keyboard.conf'
    'modprobe/hid_apple.conf'
    'kde/shortcuts/apple-brightness-daemon'
    'kde/shortcuts/apple-brightness-down'
    'kde/shortcuts/apple-brightness-up'
    'kde/DeviceItem.qml'
)
sha256sums=('SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP' 'SKIP')

build() {
    gcc -O2 -Wall -o rssi-helper "$srcdir/rssi-helper.c"
}

package() {
    # Main daemon
    install -Dm755 "$srcdir/apple-kb-monitor" "$pkgdir/usr/bin/apple-kb-monitor"

    # RSSI helper (needs setcap post-install)
    install -Dm755 "$srcdir/../rssi-helper" "$pkgdir/usr/lib/apple-kb-monitor/rssi-helper"

    # Brightness scripts
    install -Dm755 "$srcdir/apple-brightness-daemon" "$pkgdir/usr/bin/apple-brightness-daemon"
    install -Dm755 "$srcdir/apple-brightness-down" "$pkgdir/usr/bin/apple-brightness-down"
    install -Dm755 "$srcdir/apple-brightness-up" "$pkgdir/usr/bin/apple-brightness-up"

    # systemd user services
    install -Dm644 "$srcdir/apple-kb-monitor.service" "$pkgdir/usr/lib/systemd/user/apple-kb-monitor.service"
    install -Dm644 "$srcdir/apple-brightness.service" "$pkgdir/usr/lib/systemd/user/apple-brightness.service"

    # udev rules
    install -Dm644 "$srcdir/99-apple-kb-hidraw.rules" "$pkgdir/usr/lib/udev/rules.d/99-apple-kb-hidraw.rules"

    # keyd config
    install -Dm644 "$srcdir/apple-keyboard.conf" "$pkgdir/etc/keyd/apple-keyboard.conf"

    # modprobe (fnmode=1)
    install -Dm644 "$srcdir/hid_apple.conf" "$pkgdir/etc/modprobe.d/hid_apple.conf"

    # KDE Bluedevil patch
    install -Dm644 "$srcdir/DeviceItem.qml" "$pkgdir/usr/share/apple-kb-monitor/kde/DeviceItem.qml"
}
