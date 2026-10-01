// SPDX-License-Identifier: GPL-2.0-or-later
// Asynchronous I/O for the "Clavier Apple" settings module (#250).
//
// Everything the QML pages do goes through this object and NEVER blocks the
// GUI thread: D-Bus calls are QDBusPendingCall with an explicit timeout,
// commands are QProcess (started, never waited for) killed at their deadline,
// file reads and writes run on the global thread pool. Each request returns
// an id; its answer arrives later as a signal carrying the same id.
#pragma once

#include <QDBusServiceWatcher>
#include <QHash>
#include <QObject>
#include <QStringList>
#include <QVariant>

class QProcess;
class QTimer;

class AkmBridge : public QObject
{
    Q_OBJECT
    // The daemon's well-known name is owned on the session bus.
    Q_PROPERTY(bool daemonPresent READ daemonPresent NOTIFY daemonPresentChanged)
    // false until the first NameHasOwner answer (or its timeout) arrived.
    Q_PROPERTY(bool daemonKnown READ daemonKnown NOTIFY daemonPresentChanged)
    Q_PROPERTY(QString busName READ busName CONSTANT)
    Q_PROPERTY(QString objectPath READ objectPath CONSTANT)
    // $XDG_CONFIG_HOME/apple-kb-monitor/config.toml (the daemon's file).
    Q_PROPERTY(QString configPath READ configPath CONSTANT)

public:
    explicit AkmBridge(QObject *parent = nullptr);
    ~AkmBridge() override;

    bool daemonPresent() const { return m_present; }
    bool daemonKnown() const { return m_known; }
    QString busName() const;
    QString objectPath() const;
    QString configPath() const;

    // D-Bus method call on the session bus. `signature` gives the D-Bus type of
    // each argument ("s", "b", "i", "u", "t", "x", "d"), one character per
    // argument; "" = no argument. Answer: callFinished(id, ok, value, error).
    Q_INVOKABLE int call(const QString &service,
                         const QString &path,
                         const QString &iface,
                         const QString &method,
                         const QString &signature,
                         const QVariantList &args,
                         int timeoutMs);
    // org.freedesktop.DBus.Properties.Get, answered by callFinished.
    Q_INVOKABLE int getProperty(const QString &service, const QString &path, const QString &iface, const QString &name, int timeoutMs);

    // Run an allowed program (akmctl, or `systemctl --user ...`), never through
    // a shell. Answer: runFinished(id, exitCode, stdout, stderr, timedOut).
    // exitCode -1 = could not start (message in stderr).
    Q_INVOKABLE int run(const QString &program, const QStringList &args, int timeoutMs);

    // config.toml of the daemon, read / written atomically (QSaveFile) on a
    // worker thread. Answer: fileFinished(id, ok, text, error); a missing file
    // reads as ok with an empty text.
    Q_INVOKABLE int readConfig();
    Q_INVOKABLE int writeConfig(const QString &text);

    Q_INVOKABLE void copyText(const QString &text);
    // Ask the bus again whether the daemon is there (asynchronous).
    Q_INVOKABLE void checkDaemon();

Q_SIGNALS:
    void daemonPresentChanged();
    void callFinished(int id, bool ok, const QVariant &value, const QString &error);
    void runFinished(int id, int exitCode, const QString &out, const QString &err, bool timedOut);
    void fileFinished(int id, bool ok, const QString &text, const QString &error);
    // StateChanged(t revision, s json) of the daemon (root object).
    void daemonStateChanged(qulonglong revision);

private Q_SLOTS:
    void onStateChanged(qulonglong revision, const QString &json);

private:
    void setPresent(bool present);
    // Test hook (tests/e2e/kcm.py): with AKM_KCM_HEARTBEAT=<file>, a 16 ms
    // timer measures how late the GUI event loop serves it and appends one
    // JSON line per 500 ms ({"ts_ms", "ticks", "max_late_ms"}). Off otherwise.
    void startHeartbeat(const QString &path);
    int nextId();

    QDBusServiceWatcher *m_watcher = nullptr;
    bool m_present = false;
    bool m_known = false;
    int m_lastId = 0;
};
