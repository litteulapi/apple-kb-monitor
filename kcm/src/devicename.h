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

// POSIX single quotes: the quoted text is one literal shell word.
QString shellQuote(const QString &text);

// The command shown "to copy into a terminal": `akmctl rename
// --device-name='<name>'`, then ` <flag>` when given (`--check`,
// `--dry-run`). The name is glued to the option with `=`, as akmctlArgs
// does: a name starting with a dash (`-x`) is otherwise read by akmctl as
// an option and the copied command fails.
QString copyCommand(const QString &name, const QString &flag);

// What an akmctl exit code means for the page (exit codes of `akmctl rename
// --device-name`, docs/RENOMMER-CLAVIER.md):
//   "timeout"      no answer in time
//   "check-ok"     0 with --check: nothing written
//   "written"      0: written and read back identical
//   "unverified"   13: written, not read back
//   "mismatch"     14: written, read back different
//   "uncertain"    15: the write failed on its way, a frame may have been
//                  sent: NEVER shown as "not written"
//   "check-failed" anything else with --check: nothing written
//   "not-written"  anything else: nothing written
QString verdict(int exitCode, bool checkOnly, bool timedOut);
} // namespace AkmDeviceName
