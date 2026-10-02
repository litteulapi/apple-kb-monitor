// SPDX-License-Identifier: GPL-2.0-or-later
// The "Name" tab's two buttons for the name stored inside the keyboard (#248):
//   akmctl rename --device-name=<name> --yes     (write, read back, verdict)
//   akmctl rename --device-name=<name> --check   (whole pre-flight, nothing written)
// run by AkmBridge::run (QProcess, argument list, bounded delay), never a
// shell and never a terminal. Pure functions here, unit-tested
// (tests/test_devicename.cpp): the name is validated with the rules of
// akm_core::devname::validate and travels as ONE argv element.
#pragma once

#include <QString>
#include <QStringList>

namespace AkmDeviceName
{
// Same rules as `akm_core::devname::validate`: 1 to 32 characters, printable
// ASCII only (0x20-0x7E), no leading or trailing space, no backslash.
bool validDeviceName(const QString &name);

// The akmctl arguments (program excluded): empty when the name is refused.
// The name is passed as `--device-name=<name>`, so a name starting with a
// dash can never be read as an option. checkOnly: `--check` (nothing
// written); else `--yes` (the page asked the confirmation itself).
QStringList akmctlArgs(const QString &name, bool checkOnly);
} // namespace AkmDeviceName
