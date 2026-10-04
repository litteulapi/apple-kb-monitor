// SPDX-License-Identifier: GPL-2.0-or-later
// Asynchronous session-bus call with its own timeout (QtDBus's default is 25 s).
#pragma once

#include <QDBusConnection>
#include <QJSValue>
#include <QObject>
#include <QVariantList>

class BusCall : public QObject
{
    Q_OBJECT

public:
    explicit BusCall(QObject *parent = nullptr, const QDBusConnection &bus = QDBusConnection::sessionBus());

    // sig types args one character each (s, b, u, i); callback(ok, value, errorName, errorMessage)
    // runs once, with org.freedesktop.DBus.Error.NoReply when timeoutMs passes without a reply.
    Q_INVOKABLE void call(const QString &service,
                          const QString &path,
                          const QString &iface,
                          const QString &method,
                          const QString &sig,
                          const QVariantList &args,
                          int timeoutMs,
                          const QJSValue &callback);

private:
    QDBusConnection m_bus;
};
