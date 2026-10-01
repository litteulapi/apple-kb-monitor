// SPDX-License-Identifier: GPL-2.0-or-later
#include "bridge.h"

#include <QClipboard>
#include <QDBusArgument>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusMetaType>
#include <QDBusObjectPath>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QDBusVariant>
#include <QDateTime>
#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QFutureWatcher>
#include <QGuiApplication>
#include <QProcess>
#include <QSaveFile>
#include <QStandardPaths>
#include <QTimer>
#include <QtConcurrentRun>

namespace
{
const QString kBus = QStringLiteral("com.agenceapi.AppleKbMonitor1");
const QString kPath = QStringLiteral("/com/agenceapi/AppleKbMonitor1");
constexpr int kMinTimeout = 100;
constexpr int kMaxTimeout = 180000; // polkit dialogs can stay open a while
constexpr qint64 kMaxOutput = 1 << 20; // per stream
constexpr qint64 kMaxConfig = 256 * 1024;

int clampTimeout(int ms)
{
    return qBound(kMinTimeout, ms, kMaxTimeout);
}

// D-Bus reply value -> plain QVariant usable from JavaScript.
QVariant toPlain(const QVariant &v);

QVariant demarshal(const QDBusArgument &arg)
{
    switch (arg.currentType()) {
    case QDBusArgument::BasicType:
    case QDBusArgument::VariantType:
        return toPlain(arg.asVariant());
    case QDBusArgument::ArrayType: {
        QVariantList list;
        arg.beginArray();
        while (!arg.atEnd()) {
            list << demarshal(arg);
        }
        arg.endArray();
        return list;
    }
    case QDBusArgument::StructureType: {
        QVariantList list;
        arg.beginStructure();
        while (!arg.atEnd()) {
            list << demarshal(arg);
        }
        arg.endStructure();
        return list;
    }
    case QDBusArgument::MapType: {
        QVariantMap map;
        arg.beginMap();
        while (!arg.atEnd()) {
            arg.beginMapEntry();
            const QVariant k = demarshal(arg);
            const QVariant v = demarshal(arg);
            arg.endMapEntry();
            map.insert(k.toString(), v);
        }
        arg.endMap();
        return map;
    }
    default:
        return {};
    }
}

QVariant toPlain(const QVariant &v)
{
    if (v.metaType() == QMetaType::fromType<QDBusVariant>()) {
        return toPlain(v.value<QDBusVariant>().variant());
    }
    if (v.metaType() == QMetaType::fromType<QDBusObjectPath>()) {
        return v.value<QDBusObjectPath>().path();
    }
    if (v.metaType() == QMetaType::fromType<QList<QDBusObjectPath>>()) {
        QVariantList out;
        const auto paths = v.value<QList<QDBusObjectPath>>();
        for (const auto &p : paths) {
            out << p.path();
        }
        return out;
    }
    if (v.metaType() == QMetaType::fromType<QDBusArgument>()) {
        return demarshal(v.value<QDBusArgument>());
    }
    if (v.metaType() == QMetaType::fromType<qulonglong>() || v.metaType() == QMetaType::fromType<qlonglong>()) {
        return v.toDouble(); // JS numbers; timestamps fit in 53 bits
    }
    return v;
}

// QML value -> D-Bus typed value for one signature character.
bool typed(QChar t, const QVariant &in, QVariant *out)
{
    bool ok = true;
    switch (t.toLatin1()) {
    case 's':
        *out = in.toString();
        return true;
    case 'b':
        *out = in.toBool();
        return true;
    case 'i':
        *out = static_cast<int>(in.toLongLong(&ok));
        return ok;
    case 'u':
        *out = static_cast<uint>(in.toULongLong(&ok));
        return ok;
    case 'x':
        *out = in.toLongLong(&ok);
        return ok;
    case 't':
        *out = in.toULongLong(&ok);
        return ok;
    case 'd':
        *out = in.toDouble(&ok);
        return ok;
    default:
        return false;
    }
}

// Programs the pages may start. No shell, absolute path from $PATH.
bool allowed(const QString &program, const QStringList &args)
{
    if (program == QLatin1String("akmctl")) {
        return true;
    }
    if (program == QLatin1String("systemctl")) {
        // only the user's own daemon unit
        static const QStringList verbs = {QStringLiteral("start"), QStringLiteral("try-restart"), QStringLiteral("is-active")};
        return args.size() == 3 && args[0] == QLatin1String("--user") && verbs.contains(args[1])
            && args[2] == QLatin1String("apple-kb-monitord.service");
    }
    return false;
}

struct FileResult {
    bool ok = false;
    QString text;
    QString error;
};
} // namespace

AkmBridge::AkmBridge(QObject *parent)
    : QObject(parent)
{
    // Registration and unregistration of the daemon, without polling.
    m_watcher = new QDBusServiceWatcher(kBus, QDBusConnection::sessionBus(),
                                        QDBusServiceWatcher::WatchForOwnerChange, this);
    connect(m_watcher, &QDBusServiceWatcher::serviceOwnerChanged, this,
            [this](const QString &, const QString &, const QString &newOwner) {
                m_known = true;
                setPresent(!newOwner.isEmpty());
                Q_EMIT daemonPresentChanged();
            });
    QDBusConnection::sessionBus().connect(kBus, kPath, kBus, QStringLiteral("StateChanged"), this,
                                          SLOT(onStateChanged(qulonglong, QString)));
    checkDaemon();
    const QString hb = qEnvironmentVariable("AKM_KCM_HEARTBEAT");
    if (!hb.isEmpty()) {
        startHeartbeat(hb);
    }
}

AkmBridge::~AkmBridge() = default;

void AkmBridge::startHeartbeat(const QString &path)
{
    constexpr int period = 16;
    auto *tick = new QTimer(this);
    tick->setTimerType(Qt::PreciseTimer);
    tick->setInterval(period);
    auto clock = std::make_shared<QElapsedTimer>();
    auto last = std::make_shared<qint64>(0);
    auto flushed = std::make_shared<qint64>(0);
    auto worst = std::make_shared<qint64>(0);
    auto ticks = std::make_shared<int>(0);
    clock->start();
    connect(tick, &QTimer::timeout, this, [=] {
        const qint64 now = clock->elapsed();
        if (*last > 0) {
            *worst = std::max(*worst, now - *last - period);
        }
        *last = now;
        ++*ticks;
        if (now - *flushed >= 500) {
            QFile f(path);
            if (f.open(QIODevice::Append)) {
                f.write(QStringLiteral("{\"ts_ms\":%1,\"ticks\":%2,\"max_late_ms\":%3}\n")
                            .arg(QDateTime::currentMSecsSinceEpoch())
                            .arg(*ticks)
                            .arg(*worst)
                            .toUtf8());
            }
            *flushed = now;
            *worst = 0;
            *ticks = 0;
        }
    });
    tick->start();
}

QString AkmBridge::busName() const
{
    return kBus;
}

QString AkmBridge::objectPath() const
{
    return kPath;
}

QString AkmBridge::configPath() const
{
    return QStandardPaths::writableLocation(QStandardPaths::GenericConfigLocation)
        + QStringLiteral("/apple-kb-monitor/config.toml");
}

int AkmBridge::nextId()
{
    m_lastId = m_lastId == std::numeric_limits<int>::max() ? 1 : m_lastId + 1;
    return m_lastId;
}

void AkmBridge::setPresent(bool present)
{
    if (m_present != present) {
        m_present = present;
    }
}

void AkmBridge::checkDaemon()
{
    QDBusMessage msg = QDBusMessage::createMethodCall(QStringLiteral("org.freedesktop.DBus"),
                                                      QStringLiteral("/org/freedesktop/DBus"),
                                                      QStringLiteral("org.freedesktop.DBus"),
                                                      QStringLiteral("NameHasOwner"));
    msg << kBus;
    auto *w = new QDBusPendingCallWatcher(QDBusConnection::sessionBus().asyncCall(msg, 3000), this);
    connect(w, &QDBusPendingCallWatcher::finished, this, [this](QDBusPendingCallWatcher *w) {
        QDBusPendingReply<bool> r = *w;
        m_known = true;
        setPresent(!r.isError() && r.value());
        Q_EMIT daemonPresentChanged();
        w->deleteLater();
    });
}

int AkmBridge::call(const QString &service,
                    const QString &path,
                    const QString &iface,
                    const QString &method,
                    const QString &signature,
                    const QVariantList &args,
                    int timeoutMs)
{
    const int id = nextId();
    if (signature.size() != args.size()) {
        QTimer::singleShot(0, this, [this, id] {
            Q_EMIT callFinished(id, false, {}, QStringLiteral("internal: signature/argument mismatch"));
        });
        return id;
    }
    QDBusMessage msg = QDBusMessage::createMethodCall(service, path, iface, method);
    QVariantList typedArgs;
    for (int i = 0; i < args.size(); ++i) {
        QVariant v;
        if (!typed(signature[i], args[i], &v)) {
            QTimer::singleShot(0, this, [this, id] {
                Q_EMIT callFinished(id, false, {}, QStringLiteral("internal: bad argument type"));
            });
            return id;
        }
        typedArgs << v;
    }
    msg.setArguments(typedArgs);
    auto *w = new QDBusPendingCallWatcher(QDBusConnection::sessionBus().asyncCall(msg, clampTimeout(timeoutMs)), this);
    connect(w, &QDBusPendingCallWatcher::finished, this, [this, id](QDBusPendingCallWatcher *w) {
        const QDBusMessage reply = w->reply();
        if (reply.type() == QDBusMessage::ErrorMessage) {
            QString err = reply.errorMessage();
            if (reply.errorName() == QLatin1String("org.freedesktop.DBus.Error.NoReply")) {
                err = QStringLiteral("timeout");
            } else if (reply.errorName() == QLatin1String("org.freedesktop.DBus.Error.ServiceUnknown")) {
                err = QStringLiteral("absent");
            }
            Q_EMIT callFinished(id, false, {}, err.isEmpty() ? reply.errorName() : err);
        } else {
            const QVariantList out = reply.arguments();
            QVariant value;
            if (out.size() == 1) {
                value = toPlain(out.first());
            } else if (out.size() > 1) {
                QVariantList l;
                for (const auto &a : out) {
                    l << toPlain(a);
                }
                value = l;
            }
            Q_EMIT callFinished(id, true, value, QString());
        }
        w->deleteLater();
    });
    return id;
}

int AkmBridge::getProperty(const QString &service, const QString &path, const QString &iface, const QString &name, int timeoutMs)
{
    return call(service, path, QStringLiteral("org.freedesktop.DBus.Properties"), QStringLiteral("Get"), QStringLiteral("ss"),
                {iface, name}, timeoutMs);
}

int AkmBridge::run(const QString &program, const QStringList &args, int timeoutMs)
{
    const int id = nextId();
    const QString exe = allowed(program, args) ? QStandardPaths::findExecutable(program) : QString();
    if (exe.isEmpty()) {
        const QString why = allowed(program, args) ? QStringLiteral("%1: not found").arg(program)
                                                   : QStringLiteral("%1: not allowed").arg(program);
        QTimer::singleShot(0, this, [this, id, why] {
            Q_EMIT runFinished(id, -1, QString(), why, false);
        });
        return id;
    }
    auto *p = new QProcess(this);
    p->setProgram(exe);
    p->setArguments(args);
    p->setStandardInputFile(QProcess::nullDevice());
    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    env.insert(QStringLiteral("NO_COLOR"), QStringLiteral("1"));
    p->setProcessEnvironment(env);
    auto timedOut = std::make_shared<bool>(false);
    auto *timer = new QTimer(p);
    timer->setSingleShot(true);
    connect(timer, &QTimer::timeout, p, [p, timedOut] {
        *timedOut = true;
        p->kill(); // finished() follows, asynchronously
    });
    connect(p, &QProcess::finished, this, [this, id, p, timedOut](int code, QProcess::ExitStatus st) {
        const QString out = QString::fromUtf8(p->readAllStandardOutput().left(kMaxOutput));
        const QString err = QString::fromUtf8(p->readAllStandardError().left(kMaxOutput));
        Q_EMIT runFinished(id, st == QProcess::NormalExit ? code : -1, out, err, *timedOut);
        p->deleteLater();
    });
    connect(p, &QProcess::errorOccurred, this, [this, id, p](QProcess::ProcessError e) {
        if (e == QProcess::FailedToStart) {
            Q_EMIT runFinished(id, -1, QString(), p->errorString(), false);
            p->deleteLater();
        }
    });
    timer->start(clampTimeout(timeoutMs));
    p->start();
    return id;
}

int AkmBridge::readConfig()
{
    const int id = nextId();
    const QString path = configPath();
    auto *w = new QFutureWatcher<FileResult>(this);
    connect(w, &QFutureWatcher<FileResult>::finished, this, [this, id, w] {
        const FileResult r = w->result();
        Q_EMIT fileFinished(id, r.ok, r.text, r.error);
        w->deleteLater();
    });
    w->setFuture(QtConcurrent::run([path] {
        FileResult r;
        QFile f(path);
        if (!f.exists()) {
            r.ok = true;
            return r;
        }
        if (f.size() > kMaxConfig) {
            r.error = QStringLiteral("%1: file too large").arg(path);
            return r;
        }
        if (!f.open(QIODevice::ReadOnly)) {
            r.error = f.errorString();
            return r;
        }
        r.text = QString::fromUtf8(f.readAll());
        r.ok = true;
        return r;
    }));
    return id;
}

int AkmBridge::writeConfig(const QString &text)
{
    const int id = nextId();
    const QString path = configPath();
    auto *w = new QFutureWatcher<FileResult>(this);
    connect(w, &QFutureWatcher<FileResult>::finished, this, [this, id, w] {
        const FileResult r = w->result();
        Q_EMIT fileFinished(id, r.ok, r.text, r.error);
        w->deleteLater();
    });
    w->setFuture(QtConcurrent::run([path, text] {
        FileResult r;
        const QByteArray data = text.toUtf8();
        if (data.size() > kMaxConfig) {
            r.error = QStringLiteral("text too large");
            return r;
        }
        if (!QDir().mkpath(QFileInfo(path).absolutePath())) {
            r.error = QStringLiteral("cannot create %1").arg(QFileInfo(path).absolutePath());
            return r;
        }
        QSaveFile f(path);
        if (!f.open(QIODevice::WriteOnly) || f.write(data) != data.size() || !f.commit()) {
            r.error = f.errorString();
            return r;
        }
        r.ok = true;
        r.text = text;
        return r;
    }));
    return id;
}

void AkmBridge::copyText(const QString &text)
{
    if (auto *cb = QGuiApplication::clipboard()) {
        cb->setText(text);
    }
}

void AkmBridge::onStateChanged(qulonglong revision, const QString &)
{
    Q_EMIT daemonStateChanged(revision);
}
