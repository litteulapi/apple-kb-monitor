// SPDX-License-Identifier: GPL-2.0-or-later
#include "devicename.h"

namespace AkmDeviceName
{
bool validDeviceName(const QString &name)
{
    if (name.isEmpty() || name.size() > 32) {
        return false;
    }
    if (name.startsWith(QLatin1Char(' ')) || name.endsWith(QLatin1Char(' '))) {
        return false;
    }
    for (const QChar c : name) {
        const ushort u = c.unicode();
        if (u < 0x20 || u > 0x7e || u == '\\') {
            return false;
        }
    }
    return true;
}

QStringList akmctlArgs(const QString &name, bool checkOnly)
{
    if (!validDeviceName(name)) {
        return {};
    }
    return {QStringLiteral("rename"),
            QStringLiteral("--device-name=") + name,
            checkOnly ? QStringLiteral("--check") : QStringLiteral("--yes")};
}
} // namespace AkmDeviceName
