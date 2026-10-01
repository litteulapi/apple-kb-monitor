// SPDX-License-Identifier: GPL-2.0-or-later
// The "Name" tab's two terminal buttons (#248): the module never writes into
// the keyboard. It only opens a terminal emulator running
//   akmctl rename --device-name=<name> --write-device-name   (or --check)
// where akmctl shows the plan, makes the backup and demands the typed consent
// (ECRIRE) and the name again before its single write. Pure functions here,
// unit-tested (tests/test_terminal.cpp): the name is validated with the rules
// of akm_core::devname::validate and travels as ONE argv element, never
// through a shell.
#pragma once

#include <QString>
#include <QStringList>

namespace AkmTerminal
{
// Same rules as `akm_core::devname::validate`: 1 to 32 characters, printable
// ASCII only (0x20-0x7E), no leading or trailing space, no backslash.
bool validDeviceName(const QString &name);

// The akmctl arguments (program excluded): empty when the name is refused.
// The name is passed as `--device-name=<name>`, so a name starting with a
// dash can never be read as an option.
QStringList akmctlArgs(const QString &name, bool checkOnly);

struct Launch {
    QString program; // absolute path of the terminal emulator; empty = refused
    QStringList args;
};

// The terminal emulator command running `akmctl <args>`; `terminal` and
// `akmctl` are absolute paths (what the caller found). konsole and xterm keep
// the window open at the end (`--hold` / `-hold`) so the verdict stays
// readable. Empty program if anything is missing or the arguments are not
// a device-name command.
Launch launchFor(const QString &terminal, const QString &akmctl, const QStringList &args);

// The first terminal emulator found in $PATH: konsole, then xterm, then
// x-terminal-emulator. `overrideExe` (test hook AKM_KCM_TERMINAL) replaces the
// search when it names an executable file.
QString findTerminal(const QString &overrideExe = QString());
} // namespace AkmTerminal
