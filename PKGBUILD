# Maintainer: Han <han@agenceapi.com>
pkgname=apple-kb-monitor
pkgver=2.0.0
pkgrel=1
pkgdesc="Full telemetry monitor for Apple Wireless Keyboards (BCM2042/BCM20733) — battery, voltage, RSSI, firmware via reverse-engineered HID Feature Reports. Native BlueZ/KDE integration via Battery Provider API."
arch=('any')
url="https://github.com/litteulapi/apple-kb-monitor"
license=('GPL-2.0-or-later')
depends=('python' 'bluez' 'dbus' 'python-dbus-fast')
optdepends=(
    'bluez-utils: bluetoothctl CLI for BT management'
    'libnotify: desktop notifications on low battery'
)
install=apple-kb-monitor.install
source=(
    'apple-kb-monitor'
    'systemd/apple-kb-monitor.service'
    'udev/99-apple-kb-hidraw.rules'
)
sha256sums=('SKIP' 'SKIP' 'SKIP')

package() {
    install -Dm755 "$srcdir/apple-kb-monitor" "$pkgdir/usr/bin/apple-kb-monitor"
    install -Dm644 "$srcdir/apple-kb-monitor.service" "$pkgdir/usr/lib/systemd/user/apple-kb-monitor.service"
    install -Dm644 "$srcdir/99-apple-kb-hidraw.rules" "$pkgdir/usr/lib/udev/rules.d/99-apple-kb-hidraw.rules"
}
