// SPDX-License-Identifier: GPL-2.0-or-later
// The real module, loaded as System Settings loads it (KCModuleLoader ->
// KCModuleQml -> QQuickWidget), driven with real clicks and key presses.
// Run by tests/private_bus.sh only: private session bus without any
// activation directory, temporary HOME/XDG, fake akmctl/pkexec/systemctl
// first in PATH (nothing real is ever written, no HID access). French UI.
#include <KCModule>
#include <KCModuleLoader>
#include <KLocalizedString>
#include <KPluginMetaData>

#include <QApplication>
#include <QDir>
#include <QFile>
#include <QQuickItem>
#include <QQuickWidget>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QTest>

#include <memory>

namespace
{
QString benchDir()
{
    return qEnvironmentVariable("AKM_BENCH_DIR");
}

QString configPath()
{
    return qEnvironmentVariable("XDG_CONFIG_HOME") + QStringLiteral("/apple-kb-monitor/config.toml");
}

QByteArray readFile(const QString &path)
{
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly)) {
        return QByteArray("<unreadable>");
    }
    return f.readAll();
}

void writeFile(const QString &path, const QByteArray &data)
{
    QDir().mkpath(QFileInfo(path).absolutePath());
    QFile f(path);
    QVERIFY(f.open(QIODevice::WriteOnly | QIODevice::Truncate));
    QCOMPARE(f.write(data), data.size());
}

QStringList g_qmlWarnings;
QtMessageHandler g_previous = nullptr;
void handler(QtMsgType type, const QMessageLogContext &ctx, const QString &msg)
{
    if (type != QtDebugMsg && type != QtInfoMsg && msg.contains(QLatin1String(".qml"))) {
        g_qmlWarnings << msg;
    }
    if (g_previous) {
        g_previous(type, ctx, msg);
    }
}

QQuickItem *findItem(QQuickItem *root, const QString &name)
{
    if (!root) {
        return nullptr;
    }
    if (root->objectName() == name) {
        return root;
    }
    const auto children = root->childItems();
    for (QQuickItem *c : children) {
        if (QQuickItem *f = findItem(c, name)) {
            return f;
        }
    }
    return nullptr;
}
} // namespace

class TestKcmModule : public QObject
{
    Q_OBJECT

    std::unique_ptr<KCModule> m_module;
    QQuickWidget *m_view = nullptr;

    // The module, as System Settings opens it, on the given tab.
    bool open(const QString &tab)
    {
        g_qmlWarnings.clear();
        const KPluginMetaData md(QStringLiteral(KCM_PLUGIN_FILE));
        if (!md.isValid()) {
            qWarning() << "invalid plugin" << KCM_PLUGIN_FILE;
            return false;
        }
        m_module.reset(KCModuleLoader::loadModule(md, nullptr, {tab}));
        if (!m_module) {
            return false;
        }
        QWidget *w = m_module->widget();
        w->resize(1100, 2600);
        w->show();
        m_view = w->findChild<QQuickWidget *>();
        if (!m_view) {
            qWarning() << "no QQuickWidget: the module failed to load";
            return false;
        }
        m_module->load();
        return true;
    }
    QQuickItem *item(const QString &name)
    {
        return m_view ? findItem(m_view->quickWindow()->contentItem(), name) : nullptr;
    }
    QObject *object(const QString &name)
    {
        return m_view && m_view->rootObject() ? m_view->rootObject()->findChild<QObject *>(name) : nullptr;
    }
    bool waitAlertsReady()
    {
        QQuickItem *page = item(QStringLiteral("alertsPage"));
        if (!page) {
            return false;
        }
        return QTest::qWaitFor([page] { return page->property("ready").toBool(); }, 5000);
    }
    // A real left click in the middle of the item.
    bool click(const QString &name)
    {
        QQuickItem *it = item(name);
        if (!it || !it->isVisible() || !it->isEnabled()) {
            qWarning() << "cannot click" << name << (it ? "disabled or hidden" : "missing");
            return false;
        }
        // the layout settles first (an inline message may have just appeared)
        QTest::qWait(100);
        const QPointF p = it->mapToScene(QPointF(it->width() / 2, it->height() / 2));
        const QVariant before = it->property("checked");
        QTest::mouseClick(m_view, Qt::LeftButton, Qt::NoModifier, p.toPoint());
        QTest::qWait(50);
        if (before.isValid() && it->property("checked") == before && it->property("checkable").toBool()) {
            qWarning() << "click on" << name << "at" << p << "did not toggle it";
            return false;
        }
        return true;
    }
    // Real key presses at the end of a text field.
    bool typeInto(const QString &name, const QString &text, bool replace = false)
    {
        if (!click(name)) {
            return false;
        }
        QTest::keyClick(m_view, Qt::Key_End);
        if (replace) {
            QTest::keyClick(m_view, Qt::Key_A, Qt::ControlModifier);
            QTest::keyClick(m_view, Qt::Key_Delete);
        }
        QTest::keyClicks(m_view, text);
        QTest::qWait(50);
        return true;
    }
    QString text(const QString &name)
    {
        QQuickItem *it = item(name);
        return it ? it->property("text").toString() : QStringLiteral("<missing %1>").arg(name);
    }
    bool shown(const QString &name)
    {
        QQuickItem *it = item(name);
        return it && it->isVisible();
    }
    // Apply, as the System Settings button does.
    void apply()
    {
        m_module->save();
        QTest::qWait(300);
    }
    void setMode(const QByteArray &mode)
    {
        writeFile(benchDir() + QStringLiteral("/akmctl.mode"), mode);
    }
    QByteArray akmctlLog()
    {
        return readFile(benchDir() + QStringLiteral("/akmctl.log"));
    }

private Q_SLOTS:
    void initTestCase()
    {
        QVERIFY2(!benchDir().isEmpty(), "run through tests/private_bus.sh");
        QVERIFY2(qEnvironmentVariable("XDG_CONFIG_HOME").startsWith(QFileInfo(benchDir()).absolutePath()), "temporary XDG only");
        g_previous = qInstallMessageHandler(handler);
        // Fake programs first in PATH: akmctl simulated (never the real one),
        // pkexec and systemctl refused and logged.
        const QString bin = benchDir() + QStringLiteral("/bin");
        QDir().mkpath(bin);
        const QByteArray akmctl =
            "#!/bin/sh\n"
            "d=\"$(dirname \"$0\")/..\"\n"
            "printf '%s\\n' \"$*\" >> \"$d/akmctl.log\"\n"
            "mode=$(cat \"$d/akmctl.mode\" 2>/dev/null)\n"
            "case \"$1\" in\n"
            "  rename)\n"
            "    case \"$mode\" in\n"
            "      segv) kill -SEGV $$ ;;\n"
            "      kill) kill -KILL $$ ;;\n"
            "      code*) echo \"fake akmctl: exit ${mode#code}\"; exit \"${mode#code}\" ;;\n"
            "    esac\n"
            "    exit 11 ;;\n"
            "  doctor) cat \"$d/doctor.json\"; exit 0 ;;\n"
            "  *) echo \"fake akmctl: $1 not simulated\" >&2; exit 2 ;;\n"
            "esac\n";
        const QByteArray refuse = "#!/bin/sh\nprintf '%s %s\\n' \"$(basename \"$0\")\" \"$*\" >> \"$(dirname \"$0\")/../refused.log\"\nexit 1\n";
        writeFile(bin + QStringLiteral("/akmctl"), akmctl);
        writeFile(bin + QStringLiteral("/pkexec"), refuse);
        writeFile(bin + QStringLiteral("/systemctl"), refuse);
        for (const char *p : {"akmctl", "pkexec", "systemctl"}) {
            QFile::setPermissions(bin + QLatin1Char('/') + QLatin1String(p), QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner);
        }
        qputenv("PATH", QByteArray((bin + QLatin1Char(':')).toUtf8() + qgetenv("PATH")));
        // French, from the catalogue built with the module.
        const QString locale = benchDir() + QStringLiteral("/locale/fr/LC_MESSAGES");
        QDir().mkpath(locale);
        QFile::remove(locale + QStringLiteral("/kcm_applekeyboard.mo"));
        QVERIFY(QFile::copy(QStringLiteral(KCM_GMO_FILE), locale + QStringLiteral("/kcm_applekeyboard.mo")));
        KLocalizedString::addDomainLocaleDir("kcm_applekeyboard", benchDir() + QStringLiteral("/locale"));
        KLocalizedString::setLanguages({QStringLiteral("fr")});
    }

    void init()
    {
        QFile::remove(configPath());
        QFile::remove(benchDir() + QStringLiteral("/akmctl.log"));
        QFile::remove(benchDir() + QStringLiteral("/akmctl.mode"));
    }

    void cleanup()
    {
        m_view = nullptr;
        m_module.reset();
        QFile f(configPath());
        f.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner);
        QVERIFY2(g_qmlWarnings.isEmpty(), qPrintable(g_qmlWarnings.join(QLatin1Char('\n'))));
    }

    void cleanupTestCase()
    {
        // No real privileged program was reached.
        QCOMPARE(readFile(benchDir() + QStringLiteral("/refused.log")), QByteArray("<unreadable>"));
    }

    void loadsWithoutWarning()
    {
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        QVERIFY(!m_module->needsSave());
        QCOMPARE(text(QStringLiteral("alerts_thresholds")), QStringLiteral("30, 15, 5"));
    }

    // #284 (K3): a config.toml that could not be read is never written over,
    // even after "Defaults" + "Apply"; the refusal is said on the page.
    void unreadableFileIsNeverRewritten_data()
    {
        QTest::addColumn<QByteArray>("content");
        QTest::addColumn<bool>("noAccess");
        const QByteArray head = "[alerts]\nenabled = false\n";
        const QByteArray tail = "\n[ddc]\nbrightness = 40\n";
        QTest::newRow("too-large") << (head + "# " + QByteArray(300 * 1024, 'x') + tail) << false;
        QTest::newRow("no-read-access") << (head + tail) << true;
        QTest::newRow("invalid-utf8") << (head + "# caf\xe9" + tail) << false;
    }
    void unreadableFileIsNeverRewritten()
    {
        QFETCH(QByteArray, content);
        QFETCH(bool, noAccess);
        writeFile(configPath(), content);
        if (noAccess) {
            QFile::setPermissions(configPath(), QFileDevice::WriteOwner);
        }
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        QVERIFY(shown(QStringLiteral("alertsFileWarning")));
        m_module->defaults();
        click(QStringLiteral("alerts_connection")); // refused or not, nothing may be written
        apply();
        QFile::setPermissions(configPath(), QFileDevice::ReadOwner | QFileDevice::WriteOwner);
        QVERIFY2(readFile(configPath()) == content, "config.toml was rewritten");
        QVERIFY(shown(QStringLiteral("alertsResult")));
        QVERIFY2(text(QStringLiteral("alertsResult")).startsWith(QStringLiteral("Non enregistré")), qPrintable(text(QStringLiteral("alertsResult"))));
    }

    // #284 (K3): the file changed on disk after it was read: not overwritten.
    void fileChangedSinceReadIsNotOverwritten()
    {
        writeFile(configPath(), "[alerts]\nenabled = true\n");
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        const QByteArray other = "[alerts]\nenabled = true\n\n[ddc]\nbrightness = 41\n";
        writeFile(configPath(), other);
        QVERIFY(click(QStringLiteral("alerts_connection")));
        QVERIFY(m_module->needsSave());
        apply();
        QCOMPARE(readFile(configPath()), other);
        QVERIFY2(text(QStringLiteral("alertsResult")).startsWith(QStringLiteral("Non enregistré")), qPrintable(text(QStringLiteral("alertsResult"))));
    }

    // #284 (K3): only the changed key is written; every other byte is kept
    // (foreign sections, CRLF, missing final newline, comments).
    void onlyTheChangedLineIsRewritten()
    {
        const QByteArray before =
            "# shared file\r\n[mqtt]\r\npassword = \"bench-not-a-secret\" # x\r\n\r\n[notifications]\r\n"
            "connection = true   # mine\r\nbattery_replaced = true\r\n[ddc]\r\nbrightness = 40";
        writeFile(configPath(), before);
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        QVERIFY(!m_module->needsSave());
        QVERIFY(click(QStringLiteral("alerts_connection")));
        QVERIFY(m_module->needsSave());
        apply();
        QByteArray after = before;
        after.replace("connection = true   # mine", "connection = false  # mine");
        QCOMPARE(readFile(configPath()), after);
        QVERIFY(!m_module->needsSave());
    }

    // #287 (K4): a value written as a string on several lines is not
    // rewritten (its second line would stay behind): refused, file intact.
    void multiLineStringIsNotRewritten()
    {
        const QByteArray before = "[notifications]\nconnection = \"\"\"\ntrue\"\"\"\n[ddc]\nbrightness = 40\n";
        writeFile(configPath(), before);
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        QVERIFY(click(QStringLiteral("alerts_connection")));
        apply();
        QCOMPARE(readFile(configPath()), before);
        QVERIFY2(text(QStringLiteral("alertsResult")).startsWith(QStringLiteral("Non enregistré")), qPrintable(text(QStringLiteral("alertsResult"))));
    }

    // #285 (K1): a refused save keeps the module "modified" for System
    // Settings (KCModule::needsSave, the Apply button and the "unsaved
    // changes" question), not only for the QML page.
    void refusedSaveKeepsApplyEnabled()
    {
        writeFile(configPath(), "[alerts]\nthresholds = [30, 15, 5]\n");
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        QVERIFY(typeInto(QStringLiteral("alerts_thresholds"), QStringLiteral(", abc")));
        QCOMPARE(text(QStringLiteral("alerts_thresholds")), QStringLiteral("30, 15, 5, abc"));
        QVERIFY(m_module->needsSave());
        QSignalSpy spy(m_module.get(), &KCModule::needsSaveChanged);
        apply();
        QVERIFY2(text(QStringLiteral("alertsResult")).startsWith(QStringLiteral("Non enregistré")), qPrintable(text(QStringLiteral("alertsResult"))));
        QVERIFY2(m_module->needsSave(), "Apply was greyed out although nothing was saved");
        QCOMPARE(readFile(configPath()), QByteArray("[alerts]\nthresholds = [30, 15, 5]\n"));
        // a write refused by the bridge (file changed meanwhile) as well
        writeFile(configPath(), "[alerts]\nthresholds = [30, 15, 5]\n# changed\n");
        QVERIFY(typeInto(QStringLiteral("alerts_thresholds"), QStringLiteral("40, 20"), true));
        apply();
        QVERIFY2(m_module->needsSave(), "Apply was greyed out although the write was refused");
        QCOMPARE(readFile(configPath()), QByteArray("[alerts]\nthresholds = [30, 15, 5]\n# changed\n"));
    }
};

QTEST_MAIN(TestKcmModule)

#include "test_kcm_module.moc"
