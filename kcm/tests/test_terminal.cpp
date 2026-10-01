// SPDX-License-Identifier: GPL-2.0-or-later
// The "Name" tab's terminal buttons never run a shell (#248): a name holding
// quotes, a semicolon, spaces, `$(...)` or a leading dash is either refused by
// the validation or reaches the launched program as ONE literal argument.
#include "../src/terminal.h"

#include <QDir>
#include <QFile>
#include <QProcess>
#include <QTemporaryDir>
#include <QTest>

using namespace AkmTerminal;

class TestTerminal : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void validation()
    {
        QVERIFY(validDeviceName(QStringLiteral("Bureau")));
        QVERIFY(validDeviceName(QStringLiteral("Clavier de alice #1")));
        QVERIFY(validDeviceName(QStringLiteral("a; rm -rf ~")));
        QVERIFY(validDeviceName(QStringLiteral("$(reboot)")));
        QVERIFY(validDeviceName(QStringLiteral("it's \"ok\"")));
        QVERIFY(validDeviceName(QStringLiteral("-x")));
        QVERIFY(validDeviceName(QString(32, QLatin1Char('a'))));
        QVERIFY(!validDeviceName(QString()));
        QVERIFY(!validDeviceName(QString(33, QLatin1Char('a'))));
        QVERIFY(!validDeviceName(QStringLiteral(" Bureau")));
        QVERIFY(!validDeviceName(QStringLiteral("Bureau ")));
        QVERIFY(!validDeviceName(QStringLiteral("café")));
        QVERIFY(!validDeviceName(QStringLiteral("a\\b")));
        QVERIFY(!validDeviceName(QStringLiteral("a\nb")));
        QVERIFY(!validDeviceName(QStringLiteral("a\tb")));
        QVERIFY(!validDeviceName(QStringLiteral("a\x7f")));
        QVERIFY(!validDeviceName(QStringLiteral(" ")));
    }

    void argumentsAreFixedAndLiteral()
    {
        QCOMPARE(akmctlArgs(QStringLiteral("Bureau"), false),
                 QStringList({QStringLiteral("rename"), QStringLiteral("--device-name=Bureau"), QStringLiteral("--write-device-name")}));
        QCOMPARE(akmctlArgs(QStringLiteral("Bureau"), true),
                 QStringList({QStringLiteral("rename"), QStringLiteral("--device-name=Bureau"), QStringLiteral("--check")}));
        // Shell metacharacters stay inside the single argument.
        const QString nasty = QStringLiteral("a\"; rm -rf ~; echo '$(x)' |&");
        const QStringList a = akmctlArgs(nasty, false);
        QCOMPARE(a.size(), 3);
        QCOMPARE(a[1], QStringLiteral("--device-name=") + nasty);
        // A leading dash is glued to the option: never a flag of its own.
        QCOMPARE(akmctlArgs(QStringLiteral("--write-device-name"), true)[1], QStringLiteral("--device-name=--write-device-name"));
        QVERIFY(akmctlArgs(QString(), false).isEmpty());
        QVERIFY(akmctlArgs(QStringLiteral("x "), false).isEmpty());
        QVERIFY(akmctlArgs(QStringLiteral("café"), false).isEmpty());
    }

    void launchKeepsEveryArgumentAndRefusesAnythingElse()
    {
        const QStringList a = akmctlArgs(QStringLiteral("a b; c"), false);
        Launch k = launchFor(QStringLiteral("/usr/bin/konsole"), QStringLiteral("/usr/bin/akmctl"), a);
        QCOMPARE(k.program, QStringLiteral("/usr/bin/konsole"));
        QCOMPARE(k.args, QStringList({QStringLiteral("--hold"), QStringLiteral("-e"), QStringLiteral("/usr/bin/akmctl"), QStringLiteral("rename"),
                                      QStringLiteral("--device-name=a b; c"), QStringLiteral("--write-device-name")}));
        Launch x = launchFor(QStringLiteral("/usr/bin/xterm"), QStringLiteral("/usr/bin/akmctl"), a);
        QCOMPARE(x.args.first(), QStringLiteral("-hold"));
        QCOMPARE(x.args.size(), 6);
        Launch o = launchFor(QStringLiteral("/usr/bin/x-terminal-emulator"), QStringLiteral("/usr/bin/akmctl"), a);
        QCOMPARE(o.args.first(), QStringLiteral("-e"));
        QCOMPARE(o.args.size(), 5);
        // Refused: no terminal, no akmctl, another akmctl command, a tampered option.
        QVERIFY(launchFor(QString(), QStringLiteral("/usr/bin/akmctl"), a).program.isEmpty());
        QVERIFY(launchFor(QStringLiteral("/usr/bin/konsole"), QString(), a).program.isEmpty());
        QVERIFY(launchFor(QStringLiteral("/usr/bin/konsole"), QStringLiteral("/usr/bin/akmctl"), {}).program.isEmpty());
        QVERIFY(launchFor(QStringLiteral("/usr/bin/konsole"), QStringLiteral("/usr/bin/akmctl"),
                          {QStringLiteral("repair"), QStringLiteral("--device-name=x"), QStringLiteral("--check")})
                    .program.isEmpty());
        QVERIFY(launchFor(QStringLiteral("/usr/bin/konsole"), QStringLiteral("/usr/bin/akmctl"),
                          {QStringLiteral("rename"), QStringLiteral("--device-name=x"), QStringLiteral("--restore")})
                    .program.isEmpty());
        QVERIFY(launchFor(QStringLiteral("/usr/bin/konsole"), QStringLiteral("/usr/bin/akmctl"),
                          {QStringLiteral("rename"), QStringLiteral("--device-name=x "), QStringLiteral("--check")})
                    .program.isEmpty());
        QVERIFY(launchFor(QStringLiteral("/usr/bin/konsole"), QStringLiteral("/usr/bin/akmctl"),
                          {QStringLiteral("rename"), QStringLiteral("x"), QStringLiteral("--check")})
                    .program.isEmpty());
    }

    void findTerminalHonoursTheTestHookOnly()
    {
        QTemporaryDir dir;
        QVERIFY(dir.isValid());
        const QString fake = dir.filePath(QStringLiteral("fake-terminal"));
        QVERIFY(findTerminal(fake).isEmpty()); // missing
        QFile f(fake);
        QVERIFY(f.open(QIODevice::WriteOnly));
        f.write("#!/bin/sh\nexit 0\n");
        f.close();
        QVERIFY(findTerminal(fake).isEmpty()); // not executable
        QVERIFY(f.setPermissions(QFile::ReadOwner | QFile::WriteOwner | QFile::ExeOwner));
        QCOMPARE(findTerminal(fake), fake);
        QVERIFY(findTerminal(QStringLiteral("relative/fake")).isEmpty()); // never relative
    }

    // The real process boundary: a fake "terminal" records its argv. The name
    // with quotes, a semicolon, spaces and $(...) arrives as one line, intact.
    void aFakeTerminalReceivesTheLiteralArguments()
    {
        QTemporaryDir dir;
        QVERIFY(dir.isValid());
        const QString out = dir.filePath(QStringLiteral("argv.txt"));
        const QString fake = dir.filePath(QStringLiteral("fake-terminal"));
        QFile f(fake);
        QVERIFY(f.open(QIODevice::WriteOnly));
        // No QString::arg here: `%s` must reach the script untouched.
        const QString script = QStringLiteral("#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '") + out + QStringLiteral("'\n");
        f.write(script.toUtf8());
        f.close();
        QVERIFY(f.setPermissions(QFile::ReadOwner | QFile::WriteOwner | QFile::ExeOwner));
        const QString name = QStringLiteral("a\"; rm -rf ~; $(echo x) 'q' |&");
        const Launch l = launchFor(findTerminal(fake), QStringLiteral("/usr/bin/akmctl"), akmctlArgs(name, true));
        QCOMPARE(l.program, fake);
        QVERIFY(QProcess::startDetached(l.program, l.args));
        QTRY_VERIFY_WITH_TIMEOUT(QFile::exists(out) && QFile(out).size() > 0, 5000);
        QFile r(out);
        QVERIFY(r.open(QIODevice::ReadOnly));
        const QStringList lines = QString::fromUtf8(r.readAll()).split(QLatin1Char('\n'), Qt::SkipEmptyParts);
        QCOMPARE(lines, QStringList({QStringLiteral("-e"), QStringLiteral("/usr/bin/akmctl"), QStringLiteral("rename"),
                                     QStringLiteral("--device-name=") + name, QStringLiteral("--check")}));
    }
};

QTEST_GUILESS_MAIN(TestTerminal)
#include "test_terminal.moc"
