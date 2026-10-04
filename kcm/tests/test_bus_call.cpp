// SPDX-License-Identifier: GPL-2.0-or-later
// BusCall against fake_slow_daemon.py on a private bus (store_timeouts.sh): the
// deadline given is the one applied, above and below QtDBus's 25 s default.
#include "buscall.h"

#include <QElapsedTimer>
#include <QRegularExpression>
#include <QJSEngine>
#include <QTest>

class TestBusCall : public QObject
{
    Q_OBJECT

    QJSEngine m_engine;
    BusCall m_call;

    // records (ok, value, errorName) and the elapsed time into the engine's global "r<n>"
    QJSValue recorder(const QString &name)
    {
        return m_engine.evaluate(QStringLiteral("(function (t0) { return function (ok, value, name) { %1 = { ok: ok, value: value, name: name, ms: Date.now() - t0 }; }; })(Date.now())").arg(name));
    }

private Q_SLOTS:
    void deadlineIsTheOneGiven()
    {
        QVERIFY2(qEnvironmentVariableIsSet("DBUS_SESSION_BUS_ADDRESS"), "run through tests/store_timeouts.sh");
        const QString bus = QStringLiteral("com.agenceapi.AppleKbMonitor1");
        const QString root = QStringLiteral("/com/agenceapi/AppleKbMonitor1");
        m_call.call(bus, root, bus, QStringLiteral("Slow"), QStringLiteral("u"), {27000}, 40000, recorder(QStringLiteral("slow")));
        m_call.call(bus, root, bus, QStringLiteral("Slow"), QStringLiteral("u"), {10000}, 2000, recorder(QStringLiteral("cut")));
        const auto done = [this](const char *n) { return m_engine.globalObject().hasProperty(QLatin1String(n)); };
        QVERIFY(QTest::qWaitFor([&] { return done("cut"); }, 6000));
        const QJSValue cut = m_engine.globalObject().property(QStringLiteral("cut"));
        QVERIFY(!cut.property(QStringLiteral("ok")).toBool());
        QCOMPARE(cut.property(QStringLiteral("name")).toString(), QStringLiteral("org.freedesktop.DBus.Error.NoReply"));
        QVERIFY2(cut.property(QStringLiteral("ms")).toInt() < 4000, "a 2 s deadline must not wait for the 25 s default");
        QVERIFY(QTest::qWaitFor([&] { return done("slow"); }, 35000));
        const QJSValue slow = m_engine.globalObject().property(QStringLiteral("slow"));
        QVERIFY2(slow.property(QStringLiteral("ok")).toBool(), qPrintable(slow.property(QStringLiteral("name")).toString()));
        QCOMPARE(slow.property(QStringLiteral("value")).toString(), QStringLiteral("slow 27000"));
    }

    void throwingCallbackIsReported()
    {
        const QString bus = QStringLiteral("com.agenceapi.AppleKbMonitor1");
        const QJSValue cb = m_engine.evaluate(QStringLiteral("(function () { thrown = true; throw new Error('boom'); })"));
        QTest::ignoreMessage(QtWarningMsg, QRegularExpression(QStringLiteral("apple-kb-monitor: Error: boom")));
        m_call.call(bus, QStringLiteral("/com/agenceapi/AppleKbMonitor1"), bus, QStringLiteral("Slow"), QStringLiteral("u"), {10}, 5000, cb);
        QVERIFY(QTest::qWaitFor([&] { return m_engine.globalObject().hasProperty(QStringLiteral("thrown")); }, 5000));
    }
};

QTEST_GUILESS_MAIN(TestBusCall)

#include "test_bus_call.moc"
