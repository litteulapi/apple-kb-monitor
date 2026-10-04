// SPDX-License-Identifier: GPL-2.0-or-later
//
// The real module, loaded as System Settings loads it, driven with real clicks and key presses.
#include <KAbstractConfigModule>
#include <KCModule>
#include <KCModuleLoader>
#include <KLocalizedString>
#include <KPluginMetaData>

#include <QApplication>
#include <QClipboard>
#include <QDir>
#include <QFile>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWidget>
#include <QQuickWindow>
#include <QScopeGuard>
#include <QSignalSpy>
#include <QTest>

#include <functional>
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
    static QObject *findObject(QQuickItem *root, const QString &name)
    {
        if (!root) {
            return nullptr;
        }
        if (QObject *o = root->findChild<QObject *>(name, Qt::FindDirectChildrenOnly)) {
            return o;
        }
        const auto children = root->childItems();
        for (QQuickItem *c : children) {
            if (QObject *f = findObject(c, name)) {
                return f;
            }
        }
        return nullptr;
    }
    QObject *object(const QString &name)
    {
        return m_view ? findObject(m_view->quickWindow()->contentItem(), name) : nullptr;
    }
    bool waitAlertsReady()
    {
        QQuickItem *page = item(QStringLiteral("alertsPage"));
        if (!page) {
            return false;
        }
        // Ready only counts once the daemon is seen: before that the page shows the defaults and reloads.
        const auto ok = [page] {
            const QObject *store = page->property("store").value<QObject *>();
            return store && store->property("present").toBool() && page->property("ready").toBool();
        };
        if (QTest::qWaitFor(ok, 15000)) {
            return true;
        }
        // The CI artifact keeps the full output: say which half of the condition was missing.
        const QObject *store = page->property("store").value<QObject *>();
        qWarning() << "alerts page not ready: present" << (store ? store->property("present") : QVariant()) << "ready" << page->property("ready")
                   << "saving" << page->property("saving") << "configLoaded" << (store ? store->property("configLoaded") : QVariant()) << "configError"
                   << (store ? store->property("configError") : QVariant());
        return false;
    }
    bool click(const QString &name)
    {
        QQuickItem *it = item(name);
        if (!it || !it->isVisible() || !it->isEnabled()) {
            qWarning() << "cannot click" << name << (it ? "disabled or hidden" : "missing");
            return false;
        }
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
    void apply()
    {
        QQuickItem *page = item(QStringLiteral("alertsPage"));
        QVERIFY(page);
        QObject *kcm = nullptr;
        for (QQmlContext *c = qmlContext(page); c && !kcm; c = c->parentContext()) {
            kcm = c->contextProperty(QStringLiteral("kcm")).value<QObject *>();
        }
        QVERIFY2(kcm, "no module object with saveReturned()");
        QSignalSpy returned(kcm, SIGNAL(saveReturned()));
        m_module->save();
        // The write goes through the daemon: wait for its answer and the module's return, not for a fixed delay.
        QVERIFY(QTest::qWaitFor([&] { return returned.count() > 0 && !page->property("saving").toBool(); }, 30000));
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
        const QString locale = benchDir() + QStringLiteral("/locale/de/LC_MESSAGES");
        QDir().mkpath(locale);
        QFile::remove(locale + QStringLiteral("/kcm_applekeyboard.mo"));
        QVERIFY(QFile::copy(QStringLiteral(KCM_GMO_FILE), locale + QStringLiteral("/kcm_applekeyboard.mo")));
        KLocalizedString::addDomainLocaleDir("kcm_applekeyboard", benchDir() + QStringLiteral("/locale"));
        KLocalizedString::setLanguages({QStringLiteral("de")});
        // number formats do not depend on the host locale
        QLocale::setDefault(QLocale(QLocale::English, QLocale::UnitedStates));
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
        QCOMPARE(readFile(benchDir() + QStringLiteral("/refused.log")), QByteArray("<unreadable>"));
    }

    void loadsWithoutWarning()
    {
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        QVERIFY(!m_module->needsSave());
        QCOMPARE(text(QStringLiteral("alerts_thresholds")), QStringLiteral("30, 15, 5"));
        // $XDG_CONFIG_HOME, not a hard-coded ~/.config
        QCOMPARE(text(QStringLiteral("alertsFilePath")), configPath());
    }

    // a keyboard name, an alias set through BlueZ or a daemon message is never read as markup:
    // <img src=http://…> would make System Settings fetch it
    void namesArePlainText()
    {
        QVERIFY(open(QStringLiteral("name")));
        QObject *store = item(QStringLiteral("alertsPage"))->property("store").value<QObject *>();
        QVERIFY(store);
        const QString markup = QStringLiteral("<b>Desk</b><img src=\"http://127.0.0.1:9/leak.png\">");
        const QVariantMap device{{QStringLiteral("alias"), markup}, {QStringLiteral("name"), markup},
                                 {QStringLiteral("name_on_keyboard"), markup}, {QStringLiteral("mac"), QStringLiteral("AA:BB:CC:DD:EE:F1")}};
        store->setProperty("state", QVariantMap{{QStringLiteral("connected"), true}, {QStringLiteral("keyboard"), QVariantMap{{QStringLiteral("device"), device}}}});
        QObject *message = object(QStringLiteral("nameDeviceResult"));
        QVERIFY(message);
        message->setProperty("text", markup);
        message->setProperty("visible", true);
        QList<QQuickItem *> shown;
        const std::function<void(QQuickItem *)> walk = [&](QQuickItem *i) {
            if (i->property("textFormat").isValid() && i->property("text").toString().contains(QLatin1String("<b>Desk</b>"))) {
                shown << i;
            }
            const auto children = i->childItems();
            for (QQuickItem *c : children) {
                walk(c);
            }
        };
        QTRY_VERIFY_WITH_TIMEOUT((shown.clear(), walk(m_view->quickWindow()->contentItem()), shown.size() >= 5), 3000);
        for (QQuickItem *i : std::as_const(shown)) {
            QVERIFY2(i->property("textFormat").toInt() == Qt::PlainText, i->metaObject()->className());
            QCOMPARE(i->property("text").toString(), markup);
        }
    }

    // System Settings reuses an open module: the widget's "keys" argument still switches the tab
    void activationSwitchesTheTab()
    {
        QVERIFY(open(QStringLiteral("notifications")));
        QObject *tabs = object(QStringLiteral("tabs"));
        QVERIFY(tabs);
        QTRY_COMPARE(tabs->property("currentIndex").toInt(), 2);
        auto *cm = qobject_cast<KAbstractConfigModule *>(qmlContext(tabs)->contextProperty(QStringLiteral("kcm")).value<QObject *>());
        QVERIFY(cm);
        Q_EMIT cm->activationRequested({QStringLiteral("keys")});
        QTRY_COMPARE(tabs->property("currentIndex").toInt(), 1);
    }

    // every failure code of Store.cmd() is shown in words, never as "unknown error"
    void failureCodesInWords()
    {
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        QObject *store = item(QStringLiteral("alertsPage"))->property("store").value<QObject *>();
        QVERIFY(store);
        const auto words = [store](const char *code) {
            QVariant r;
            QMetaObject::invokeMethod(store, "failureText", Q_RETURN_ARG(QVariant, r), Q_ARG(QVariant, QString::fromUtf8(code)));
            return r.toString();
        };
        QCOMPARE(words("absent"), QStringLiteral("Der Tastaturdienst (apple-kb-monitord) läuft nicht"));
        QCOMPARE(words("forbidden"), QStringLiteral("Befehl nicht erlaubt"));
        QCOMPARE(words("timeout"), QStringLiteral("keine rechtzeitige Antwort"));
        QCOMPARE(words("invalid: Unexpected token"), QStringLiteral("unlesbare Antwort: Unexpected token"));
        QCOMPARE(words("org.freedesktop.DBus.Error.AccessDenied"), QStringLiteral("org.freedesktop.DBus.Error.AccessDenied"));
    }

    // a config.toml that could not be read is never written over, even after "Defaults" +
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
        QVERIFY2(text(QStringLiteral("alertsResult")).startsWith(QStringLiteral("Nicht gespeichert")), qPrintable(text(QStringLiteral("alertsResult"))));
    }

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
        QVERIFY2(text(QStringLiteral("alertsResult")).startsWith(QStringLiteral("Nicht gespeichert")), qPrintable(text(QStringLiteral("alertsResult"))));
    }

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

    void multiLineStringIsNotRewritten()
    {
        const QByteArray before = "[notifications]\nconnection = \"\"\"\ntrue\"\"\"\n[ddc]\nbrightness = 40\n";
        writeFile(configPath(), before);
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        QVERIFY(click(QStringLiteral("alerts_connection")));
        apply();
        QCOMPARE(readFile(configPath()), before);
        QVERIFY2(text(QStringLiteral("alertsResult")).startsWith(QStringLiteral("Nicht gespeichert")), qPrintable(text(QStringLiteral("alertsResult"))));
    }

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
        QVERIFY2(text(QStringLiteral("alertsResult")).startsWith(QStringLiteral("Nicht gespeichert")), qPrintable(text(QStringLiteral("alertsResult"))));
        QVERIFY2(m_module->needsSave(), "Apply was greyed out although nothing was saved");
        QCOMPARE(readFile(configPath()), QByteArray("[alerts]\nthresholds = [30, 15, 5]\n"));
        writeFile(configPath(), "[alerts]\nthresholds = [30, 15, 5]\n# changed\n");
        QVERIFY(typeInto(QStringLiteral("alerts_thresholds"), QStringLiteral("40, 20"), true));
        apply();
        QVERIFY2(m_module->needsSave(), "Apply was greyed out although the write was refused");
        QCOMPARE(readFile(configPath()), QByteArray("[alerts]\nthresholds = [30, 15, 5]\n# changed\n"));
    }

    void validFileOpensUnmodified_data()
    {
        QTest::addColumn<QByteArray>("line");
        QTest::addColumn<QString>("control");
        QTest::addColumn<QString>("shownAs");
        QTest::newRow("decimal-hysteresis") << QByteArray("hysteresis = 2.5") << QStringLiteral("alerts_hysteresis") << QStringLiteral("2.5");
        QTest::newRow("ascending-thresholds") << QByteArray("thresholds = [5, 15, 30]") << QStringLiteral("alerts_thresholds") << QStringLiteral("5, 15, 30");
        QTest::newRow("critical-99") << QByteArray("critical = 99") << QStringLiteral("alerts_critical") << QStringLiteral("99");
        QTest::newRow("thresholds-above-95") << QByteArray("thresholds = [97, 50]") << QStringLiteral("alerts_thresholds") << QStringLiteral("97, 50");
        QTest::newRow("chemistry-case") << QByteArray("chemistry = \"NiMH\"") << QString() << QString();
    }
    void validFileOpensUnmodified()
    {
        QFETCH(QByteArray, line);
        QFETCH(QString, control);
        QFETCH(QString, shownAs);
        const QByteArray section = line.startsWith("chemistry") ? "[battery]\r\n" : "[alerts]\r\n";
        const QByteArray before = "[notifications]\r\nconnection = true\r\n" + section + line + "\r\n[ddc]\r\nbrightness = 40\r\n";
        writeFile(configPath(), before);
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        QVERIFY2(!m_module->needsSave(), "opened as modified");
        QCOMPARE(m_module->representsDefaults(), line == "thresholds = [5, 15, 30]");
        if (!control.isEmpty()) {
            QQuickItem *it = item(control);
            QVERIFY(it);
            const QString shown = it->property("text").isValid() && !it->property("value").isValid() ? it->property("text").toString()
                                                                                                   : it->property("value").toString();
            QCOMPARE(shown, shownAs);
        }
        QVERIFY(!shown(QStringLiteral("alertsRangeWarning")));
        QVERIFY(click(QStringLiteral("alerts_connection")));
        QVERIFY(m_module->needsSave());
        apply();
        QVERIFY2(text(QStringLiteral("alertsResult")).startsWith(QStringLiteral("Gespeichert")), qPrintable(text(QStringLiteral("alertsResult"))));
        QByteArray after = before;
        after.replace("connection = true", "connection = false");
        QCOMPARE(readFile(configPath()), after);
        QVERIFY(!m_module->needsSave());
    }

    // never "corrected" behind the user's back.
    void outOfRangeValuesAreShownAndKept()
    {
        const QByteArray before = "[alerts]\ncritical = 120\nthresholds = [0, 50]\nhysteresis = 25\n[notifications]\nconnection = true\n";
        writeFile(configPath(), before);
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        QVERIFY2(!m_module->needsSave(), "opened as modified");
        QCOMPARE(item(QStringLiteral("alerts_critical"))->property("value").toInt(), 120);
        QCOMPARE(text(QStringLiteral("alerts_thresholds")), QStringLiteral("0, 50"));
        QCOMPARE(text(QStringLiteral("alerts_hysteresis")), QStringLiteral("25"));
        QVERIFY(shown(QStringLiteral("alertsRangeWarning")));
        const QString warning = text(QStringLiteral("alertsRangeWarning"));
        QVERIFY2(warning.contains(QLatin1String("critical = 120")) && warning.contains(QLatin1String("thresholds")) && warning.contains(QLatin1String("hysteresis")), qPrintable(warning));
        QVERIFY(click(QStringLiteral("alerts_connection")));
        apply();
        QByteArray after = before;
        after.replace("connection = true", "connection = false");
        QCOMPARE(readFile(configPath()), after);
    }

    void decimalFollowsTheLocale()
    {
        const auto restore = qScopeGuard([before = QLocale()] { QLocale::setDefault(before); });
        QLocale::setDefault(QLocale(QLocale::German));
        writeFile(configPath(), "[alerts]\nhysteresis = 2.5\n");
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        QCOMPARE(text(QStringLiteral("alerts_hysteresis")), QStringLiteral("2,5"));
        QVERIFY(!m_module->needsSave());
        QVERIFY(typeInto(QStringLiteral("alerts_hysteresis"), QStringLiteral("1,5"), true));
        apply();
        QCOMPARE(readFile(configPath()), QByteArray("[alerts]\nhysteresis = 1.5\n"));
    }

    void typedValuesAreWritten()
    {
        writeFile(configPath(), "[alerts]\nhysteresis = 3\nthresholds = [30, 15, 5]\n");
        QVERIFY(open(QStringLiteral("notifications")));
        QVERIFY(waitAlertsReady());
        QVERIFY(typeInto(QStringLiteral("alerts_hysteresis"), QStringLiteral("2.5"), true));
        QVERIFY(typeInto(QStringLiteral("alerts_thresholds"), QStringLiteral("97, 50"), true));
        QVERIFY(m_module->needsSave());
        apply();
        QCOMPARE(readFile(configPath()), QByteArray("[alerts]\nhysteresis = 2.5\nthresholds = [97, 50]\n"));
        QVERIFY(!m_module->needsSave());
        QVERIFY(typeInto(QStringLiteral("alerts_hysteresis"), QStringLiteral("25"), true));
        apply();
        QVERIFY2(text(QStringLiteral("alertsResult")).startsWith(QStringLiteral("Nicht gespeichert")), qPrintable(text(QStringLiteral("alertsResult"))));
        QVERIFY(m_module->needsSave());
    }

    void crashDuringNameWriteIsUncertain_data()
    {
        QTest::addColumn<QByteArray>("mode");
        QTest::newRow("sigsegv") << QByteArray("segv");
        QTest::newRow("sigkill") << QByteArray("kill");
        QTest::newRow("panic-101") << QByteArray("code101");
        QTest::newRow("preflight-11") << QByteArray("code11");
    }
    void crashDuringNameWriteIsUncertain()
    {
        QFETCH(QByteArray, mode);
        setMode(mode);
        QVERIFY(open(QStringLiteral("name")));
        QVERIFY(typeInto(QStringLiteral("nameDeviceField"), QStringLiteral("Bob's KB")));
        QVERIFY(click(QStringLiteral("nameWriteBtn")));
        QObject *dialog = object(QStringLiteral("nameConfirmDialog"));
        QVERIFY(dialog);
        QTRY_VERIFY(dialog->property("opened").toBool());
        QObject *write = dialog->findChild<QObject *>(QStringLiteral("nameConfirmWrite"));
        QVERIFY(write);
        QCOMPARE(write->property("text").toString(), QStringLiteral("Schreiben"));
        QVERIFY(QMetaObject::invokeMethod(write, "trigger"));
        QTRY_VERIFY_WITH_TIMEOUT(shown(QStringLiteral("nameDeviceResult")), 10000);
        const QString verdict = text(QStringLiteral("nameDeviceResult"));
        QCOMPARE(akmctlLog().count("rename"), 1);
        QVERIFY(akmctlLog().endsWith("\nrename --device-name=Bob's KB --yes\n"));
        if (mode == "code11") {
            QVERIFY2(verdict.startsWith(QStringLiteral("Nicht geschrieben")), qPrintable(verdict));
        } else {
            QVERIFY2(!verdict.startsWith(QStringLiteral("Nicht geschrieben")), qPrintable(verdict));
            QVERIFY2(verdict.contains(QStringLiteral("möglicherweise geschrieben oder auch nicht")), qPrintable(verdict));
            QVERIFY2(verdict.contains(QStringLiteral("akmctl wurde abrupt beendet")), qPrintable(verdict));
        }
    }

    void noCommandToCopyWithoutAName()
    {
        QVERIFY(open(QStringLiteral("name")));
        QTest::qWait(200);
        for (int i = 0; i < 3; ++i) {
            const QString cmd = text(QStringLiteral("nameCmd%1").arg(i));
            QVERIFY2(!cmd.contains(QLatin1String("NOM")) && !cmd.contains(QLatin1String("NAME")) && !cmd.contains(QLatin1String("rename")), qPrintable(cmd));
            QVERIFY2(!item(QStringLiteral("nameCopy%1").arg(i))->isEnabled(), "copy enabled without a name");
        }
        QVERIFY(item(QStringLiteral("nameCopy3"))->isEnabled()); // akmctl repair --force needs no name
        QVERIFY(typeInto(QStringLiteral("nameDeviceField"), QStringLiteral("Office")));
        QCOMPARE(text(QStringLiteral("nameCmd2")), QStringLiteral("akmctl rename --device-name='Office'"));
        QVERIFY(click(QStringLiteral("nameCopy2")));
        QCOMPARE(QGuiApplication::clipboard()->text(), QStringLiteral("akmctl rename --device-name='Office'"));
    }

    // "Undo changes" keeps the hid_apple controls bound: a later reload of the table moves them.
    void undoKeepsParamsFollowingTheTable()
    {
        QVERIFY(open(QStringLiteral("keys")));
        QQuickItem *page = item(QStringLiteral("keysPage"));
        QVERIFY(page);
        QObject *store = page->property("store").value<QObject *>();
        QVERIFY(store);
        QTRY_VERIFY_WITH_TIMEOUT(!store->property("keyTableBusy").toBool(), 15000);
        const auto table = [](int fnmode, int ctrlCmd) {
            return QVariant(QVariantMap{{QStringLiteral("params"),
                                         QVariantMap{{QStringLiteral("fnmode"), fnmode},
                                                     {QStringLiteral("swap_opt_cmd"), 0},
                                                     {QStringLiteral("swap_ctrl_cmd"), ctrlCmd},
                                                     {QStringLiteral("swap_fn_leftctrl"), 0},
                                                     {QStringLiteral("iso_layout"), -1}}}});
        };
        store->setProperty("keyTable", table(1, 0));
        QQuickItem *fnmode = item(QStringLiteral("keysFnmode"));
        QQuickItem *ctrlCmd = item(QStringLiteral("keysCtrlCmd"));
        QQuickItem *applyBtn = item(QStringLiteral("keysApplyParamsBtn"));
        QVERIFY(fnmode && ctrlCmd && applyBtn);
        QTRY_COMPARE(fnmode->property("currentIndex").toInt(), 1);
        QVERIFY(QMetaObject::invokeMethod(fnmode, "incrementCurrentIndex"));
        QVERIFY(ctrlCmd->setProperty("checked", true));
        QTRY_VERIFY(applyBtn->isEnabled());
        QVERIFY(click(QStringLiteral("keysUndoBtn")));
        QTRY_VERIFY(!applyBtn->isEnabled());
        store->setProperty("keyTable", table(2, 1));
        QTRY_COMPARE(fnmode->property("currentIndex").toInt(), 2);
        QVERIFY(ctrlCmd->property("checked").toBool());
        QVERIFY2(!applyBtn->isEnabled(), "a reload after Undo offers to write the old values back");
    }

    // A failed hid_apple parameter: the store token is translated, the parameter named apart.
    void paramFailureIsTranslated()
    {
        QVERIFY(open(QStringLiteral("keys")));
        QQuickItem *page = item(QStringLiteral("keysPage"));
        QVERIFY(page);
        const auto shown = [&](const char *token) {
            QVERIFY(QMetaObject::invokeMethod(page, "report", Q_ARG(QVariant, false), Q_ARG(QVariant, QString()),
                                              Q_ARG(QVariant, QString::fromUtf8(token)), Q_ARG(QVariant, QStringLiteral("fnmode"))));
        };
        shown("timeout");
        QVERIFY2(!text(QStringLiteral("keysResult")).contains(QStringLiteral("timeout")), qPrintable(text(QStringLiteral("keysResult"))));
        QVERIFY2(!text(QStringLiteral("keysResult")).contains(QStringLiteral("fnmode")), qPrintable(text(QStringLiteral("keysResult"))));
        shown("forbidden");
        QVERIFY2(text(QStringLiteral("keysResult")).contains(QStringLiteral("fnmode: Befehl nicht erlaubt")), qPrintable(text(QStringLiteral("keysResult"))));
        // A failed Keymap read is said, not shown as an empty mapping.
        QObject *store = page->property("store").value<QObject *>();
        QVERIFY(store);
        QVERIFY(store->setProperty("keymapError", QStringLiteral("timeout")));
        QTRY_VERIFY(item(QStringLiteral("keysKeymapError"))->isVisible());
        QVERIFY2(!text(QStringLiteral("keysKeymapError")).contains(QStringLiteral("timeout")), qPrintable(text(QStringLiteral("keysKeymapError"))));
        QVERIFY(store->setProperty("keymapError", QString()));
    }

    void doctorVerdictIsTranslated()
    {
        writeFile(benchDir() + QStringLiteral("/doctor.json"),
                  "{\"verdict\":{\"level\":\"warn\",\"advice\":\"(English advice)\",\"id\":\"link-up-fix\",\"args\":[\"journal\",\"adapter-pm\"]},"
                  "\"findings\":["
                  "{\"level\":\"info\",\"topic\":\"link-key\",\"text\":\"(English)\",\"fix\":null,\"id\":\"link-key.unchecked\",\"args\":[]},"
                  "{\"level\":\"warn\",\"topic\":\"journal\",\"text\":\"(English)\",\"fix\":\"(English fix)\",\"id\":\"journal.storage-error\",\"args\":[\"2\",\"08:12\"]},"
                  "{\"level\":\"ok\",\"topic\":\"pairing\",\"text\":\"(English)\",\"fix\":null,\"id\":\"pairing.ok\",\"args\":[\"AA:BB:CC:DD:EE:F1\",\"alex\",\"1\",\"\"]},"
                  "{\"level\":\"info\",\"topic\":\"future\",\"text\":\"raw text of a newer akmctl\",\"fix\":\"raw fix\",\"id\":\"future.thing\",\"args\":[]}"
                  "]}");
        QVERIFY(open(QStringLiteral("diagnostics")));
        QVERIFY(click(QStringLiteral("diagDoctorBtn")));
        QTRY_VERIFY2_WITH_TIMEOUT(text(QStringLiteral("diagDoctorVerdict")).contains(QStringLiteral("—")), qPrintable(text(QStringLiteral("diagDoctorVerdict"))), 10000);
        QCOMPARE(text(QStringLiteral("diagDoctorVerdict")),
                 QStringLiteral("Ergebnis: Warnung — Verbindung steht; zu prüfen: Journal, Stromversorgung des Adapters (akmctl doctor)"));
        QCOMPARE(text(QStringLiteral("diagFinding0")),
                 QStringLiteral("[Verbindungsschlüssel] Verbindungsschlüssel nicht geprüft (benötigt root: sudo akmctl doctor)"));
        QCOMPARE(text(QStringLiteral("diagFinding1")),
                 QStringLiteral("[Journal] 2 Schreibfehler im BlueZ-Speicher (ein neuer Verbindungsschlüssel kann beim Neustart verloren gehen), zuletzt 08:12\n"
                                "→ geben Sie Speicherplatz frei (btrfs: Metadaten prüfen) und prüfen Sie dann mit sudo akmctl doctor"));
        QCOMPARE(text(QStringLiteral("diagFinding2")), QStringLiteral("[Kopplung] AA:BB:XX:XX:XX:F1 „alex“ gekoppelt, gebunden"));
        QCOMPARE(text(QStringLiteral("diagFinding3")), QStringLiteral("[future] raw text of a newer akmctl\n→ raw fix"));
        QVERIFY2(!text(QStringLiteral("diagDoctorVerdict")).contains(QStringLiteral("English")), "verdict from its phrase");
        QVariant report;
        QVERIFY(QMetaObject::invokeMethod(item(QStringLiteral("diagPage")), "report", Q_RETURN_ARG(QVariant, report)));
        QVERIFY2(report.toString().contains(QStringLiteral("AA:BB:XX:XX:XX:F1")), qPrintable(report.toString()));
        QVERIFY2(!report.toString().contains(QStringLiteral("CC:DD:EE")), "copied diagnosis carries a full Bluetooth address");
    }

    void signalAgeIsTheSignals()
    {
        QVERIFY(open(QStringLiteral("state")));
        QQuickItem *page = item(QStringLiteral("statePage"));
        QVERIFY(page);
        QObject *store = page->property("store").value<QObject *>();
        QVERIFY(store);
        const double now = page->property("now").toDouble() / 1000;
        QVERIFY(store->setProperty("state", QVariantMap{{QStringLiteral("connected"), true},
                                                        {QStringLiteral("rssi_at"), qint64(now) - 120},
                                                        {QStringLiteral("last_update"), qint64(now) - 3 * 3600}}));
        QVariant expected;
        QVERIFY(QMetaObject::invokeMethod(page, "ago", Q_RETURN_ARG(QVariant, expected), Q_ARG(QVariant, now - (qint64(now) - 120))));
        QCOMPARE(text(QStringLiteral("stateSignalAge")), expected.toString());
    }

    void percentMatchesTheWidgetInEnglish()
    {
        const auto restore = qScopeGuard([] { KLocalizedString::setLanguages({QStringLiteral("de")}); });
        KLocalizedString::setLanguages({QStringLiteral("en_US")});
        QVERIFY(open(QStringLiteral("state")));
        QVariant pct;
        QVERIFY(QMetaObject::invokeMethod(item(QStringLiteral("statePage")), "pct", Q_RETURN_ARG(QVariant, pct), Q_ARG(QVariant, 86)));
        QCOMPARE(pct.toString(), QStringLiteral("86%"));
    }

    void diagDecimalsFollowTheLocale()
    {
        const auto restore = qScopeGuard([before = QLocale()] { QLocale::setDefault(before); });
        QLocale::setDefault(QLocale(QLocale::German));
        QVERIFY(open(QStringLiteral("diagnostics")));
        QObject *diag = item(QStringLiteral("diagPage"));
        const QVariantMap check{{QStringLiteral("msg"), QStringLiteral("freshness.stale")}, {QStringLiteral("args"), QStringList{QStringLiteral("7.5")}}};
        const QVariantMap finding{{QStringLiteral("id"), QStringLiteral("link-quality")},
                                  {QStringLiteral("args"), QStringList{QStringLiteral("0"), QStringLiteral("2"), QStringLiteral("5"), QStringLiteral("-3.4"),
                                                                       QStringLiteral("-9"), QStringLiteral("120"), QString()}}};
        QVariant stale, quality;
        QVERIFY(QMetaObject::invokeMethod(diag, "selftestText", Q_RETURN_ARG(QVariant, stale), Q_ARG(QVariant, check)));
        QVERIFY2(stale.toString().contains(QStringLiteral("7,5")), qPrintable(stale.toString()));
        QVERIFY(QMetaObject::invokeMethod(diag, "findingText", Q_RETURN_ARG(QVariant, quality), Q_ARG(QVariant, finding)));
        QVERIFY2(quality.toString().contains(QStringLiteral("3,4 dB")), qPrintable(quality.toString()));
    }
};

QTEST_MAIN(TestKcmModule)

#include "test_kcm_module.moc"
