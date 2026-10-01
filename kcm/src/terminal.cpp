// SPDX-License-Identifier: GPL-2.0-or-later
#include "terminal.h"

#include <QFileInfo>
#include <QStandardPaths>

namespace AkmTerminal
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
            checkOnly ? QStringLiteral("--check") : QStringLiteral("--write-device-name")};
}

Launch launchFor(const QString &terminal, const QString &akmctl, const QStringList &args)
{
    Launch l;
    // Only the two device-name commands can be launched from here.
    if (terminal.isEmpty() || akmctl.isEmpty() || args.size() != 3 || args[0] != QLatin1String("rename")
        || !args[1].startsWith(QLatin1String("--device-name=")) || !validDeviceName(args[1].mid(14))
        || (args[2] != QLatin1String("--check") && args[2] != QLatin1String("--write-device-name"))) {
        return l;
    }
    const QString base = QFileInfo(terminal).fileName();
    l.program = terminal;
    if (base == QLatin1String("konsole")) {
        l.args << QStringLiteral("--hold");
    } else if (base == QLatin1String("xterm")) {
        l.args << QStringLiteral("-hold");
    }
    l.args << QStringLiteral("-e") << akmctl << args;
    return l;
}

QString findTerminal(const QString &overrideExe)
{
    if (!overrideExe.isEmpty()) {
        const QFileInfo fi(overrideExe);
        return fi.isAbsolute() && fi.isFile() && fi.isExecutable() ? fi.absoluteFilePath() : QString();
    }
    for (const char *name : {"konsole", "xterm", "x-terminal-emulator"}) {
        const QString exe = QStandardPaths::findExecutable(QLatin1String(name));
        if (!exe.isEmpty()) {
            return exe;
        }
    }
    return {};
}
} // namespace AkmTerminal
