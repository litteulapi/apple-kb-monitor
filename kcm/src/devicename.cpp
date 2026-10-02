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

QString shellQuote(const QString &text)
{
    QString quoted = text;
    quoted.replace(QLatin1Char('\''), QStringLiteral("'\\''"));
    return QLatin1Char('\'') + quoted + QLatin1Char('\'');
}

QString copyCommand(const QString &name, const QString &flag)
{
    QString cmd = QStringLiteral("akmctl rename --device-name=") + shellQuote(name);
    if (!flag.isEmpty()) {
        cmd += QLatin1Char(' ') + flag;
    }
    return cmd;
}

QString verdict(int exitCode, bool checkOnly, bool timedOut)
{
    if (timedOut) {
        return QStringLiteral("timeout");
    }
    if (exitCode == 0) {
        return checkOnly ? QStringLiteral("check-ok") : QStringLiteral("written");
    }
    if (checkOnly) {
        return QStringLiteral("check-failed");
    }
    switch (exitCode) {
    case 13:
        return QStringLiteral("unverified");
    case 14:
        return QStringLiteral("mismatch");
    case 15:
        return QStringLiteral("uncertain");
    default:
        return QStringLiteral("not-written");
    }
}
} // namespace AkmDeviceName
