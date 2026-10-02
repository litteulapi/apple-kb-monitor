// SPDX-License-Identifier: GPL-2.0-or-later
// The "Name" tab never runs a shell (#248): a name holding quotes, a
// semicolon, spaces, `$(...)` or a leading dash is either refused by the
// validation or reaches the launched program as ONE literal argument.
#include "../src/devicename.h"

#include <QFile>
#include <QProcess>
#include <QTemporaryDir>
#include <QTest>

using namespace AkmDeviceName;

class TestDeviceName : public QObject
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
        QVERIFY(!validDeviceName(QStringLiteral(" ")));
    }

    void argumentsAreFixedAndLiteral()
    {
        QCOMPARE(akmctlArgs(QStringLiteral("Bureau"), false),
                 QStringList({QStringLiteral("rename"), QStringLiteral("--device-name=Bureau"), QStringLiteral("--yes")}));
        QCOMPARE(akmctlArgs(QStringLiteral("Bureau"), true),
                 QStringList({QStringLiteral("rename"), QStringLiteral("--device-name=Bureau"), QStringLiteral("--check")}));
        // Shell metacharacters stay inside the single argument.
        const QString nasty = QStringLiteral("a\"; rm -rf ~; echo '$(x)' |&");
        const QStringList a = akmctlArgs(nasty, false);
        QCOMPARE(a.size(), 3);
        QCOMPARE(a[1], QStringLiteral("--device-name=") + nasty);
        // A leading dash is glued to the option: never a flag of its own.
        QCOMPARE(akmctlArgs(QStringLiteral("--yes"), true)[1], QStringLiteral("--device-name=--yes"));
        QCOMPARE(akmctlArgs(QStringLiteral("--yes"), true)[2], QStringLiteral("--check"));
        QVERIFY(akmctlArgs(QString(), false).isEmpty());
        QVERIFY(akmctlArgs(QStringLiteral("x "), false).isEmpty());
        QVERIFY(akmctlArgs(QStringLiteral("café"), false).isEmpty());
    }

    // The real process boundary, as AkmBridge::run crosses it (QProcess with a
    // program and an argument list): a fake "akmctl" records its argv. The
    // name with quotes, a semicolon, spaces and $(...) arrives as one line,
    // intact, and nothing it contains is executed.
    void aFakeAkmctlReceivesTheNameAsOneLiteralArgument()
    {
        QTemporaryDir dir;
        QVERIFY(dir.isValid());
        const QString out = dir.filePath(QStringLiteral("argv.txt"));
        const QString canary = dir.filePath(QStringLiteral("canary"));
        const QString fake = dir.filePath(QStringLiteral("akmctl"));
        QFile f(fake);
        QVERIFY(f.open(QIODevice::WriteOnly));
        // No QString::arg here: `%s` must reach the script untouched.
        const QString script = QStringLiteral("#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '") + out + QStringLiteral("'\n");
        f.write(script.toUtf8());
        f.close();
        QVERIFY(f.setPermissions(QFile::ReadOwner | QFile::WriteOwner | QFile::ExeOwner));
        for (const bool checkOnly : {false, true}) {
            QFile::remove(out);
            // 32 characters at most: quotes, `;`, spaces, a command substitution.
            const QString name = QStringLiteral("a\"; $(touch ") + QStringLiteral("c) 'q' |& x");
            QVERIFY(validDeviceName(name));
            QProcess p;
            p.setWorkingDirectory(dir.path());
            p.setProgram(fake);
            p.setArguments(akmctlArgs(name, checkOnly));
            p.setStandardInputFile(QProcess::nullDevice());
            p.start();
            QVERIFY(p.waitForFinished(5000));
            QCOMPARE(p.exitCode(), 0);
            QFile r(out);
            QVERIFY(r.open(QIODevice::ReadOnly));
            const QStringList lines = QString::fromUtf8(r.readAll()).split(QLatin1Char('\n'), Qt::SkipEmptyParts);
            QCOMPARE(lines, QStringList({QStringLiteral("rename"), QStringLiteral("--device-name=") + name,
                                         checkOnly ? QStringLiteral("--check") : QStringLiteral("--yes")}));
            QVERIFY(!QFile::exists(dir.filePath(QStringLiteral("c"))));
            QVERIFY(!QFile::exists(canary));
        }
    }
};

QTEST_GUILESS_MAIN(TestDeviceName)
#include "test_devicename.moc"
