// SPDX-License-Identifier: GPL-2.0-or-later
#include "buscall.h"

#include <QDBusMessage>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QDBusVariant>
#include <QDebug>

namespace
{
QVariant typed(QChar sig, const QVariant &v)
{
    switch (sig.unicode()) {
    case 's':
        return v.toString();
    case 'b':
        return v.toBool();
    case 'u':
        return QVariant::fromValue(uint(qMax(0.0, v.toDouble())));
    case 'i':
        return QVariant::fromValue(int(v.toDouble()));
    }
    return v;
}

QJSValue toJs(QVariant v)
{
    if (v.canConvert<QDBusVariant>()) {
        v = v.value<QDBusVariant>().variant();
    }
    switch (v.typeId()) {
    case QMetaType::UnknownType:
        return QJSValue(QJSValue::NullValue);
    case QMetaType::Bool:
        return QJSValue(v.toBool());
    case QMetaType::Int:
    case QMetaType::UInt:
    case QMetaType::LongLong:
    case QMetaType::ULongLong:
    case QMetaType::Double:
        return QJSValue(v.toDouble());
    }
    return QJSValue(v.toString());
}
} // namespace

BusCall::BusCall(QObject *parent, const QDBusConnection &bus)
    : QObject(parent)
    , m_bus(bus)
{
}

void BusCall::call(const QString &service,
                   const QString &path,
                   const QString &iface,
                   const QString &method,
                   const QString &sig,
                   const QVariantList &args,
                   int timeoutMs,
                   const QJSValue &callback)
{
    QDBusMessage msg = QDBusMessage::createMethodCall(service, path, iface, method);
    QVariantList typedArgs;
    for (qsizetype i = 0; i < args.size(); ++i) {
        typedArgs << typed(i < sig.size() ? sig.at(i) : QChar(), args.at(i));
    }
    msg.setArguments(typedArgs);
    // A stopped daemon stays stopped: only systemd StartUnit starts it.
    msg.setAutoStartService(false);
    auto *watcher = new QDBusPendingCallWatcher(m_bus.asyncCall(msg, timeoutMs), this);
    connect(watcher, &QDBusPendingCallWatcher::finished, this, [callback](QDBusPendingCallWatcher *w) {
        w->deleteLater();
        const QDBusMessage reply = w->reply();
        QJSValue cb = callback;
        const QJSValue r = reply.type() == QDBusMessage::ReplyMessage
            ? cb.call({QJSValue(true), toJs(reply.arguments().value(0)), QJSValue(QString()), QJSValue(QString())})
            : cb.call({QJSValue(false), QJSValue(QJSValue::NullValue), QJSValue(reply.errorName()), QJSValue(reply.errorMessage())});
        // call() returns what the script threw instead of reporting it
        if (r.isError()) {
            qWarning().noquote() << "apple-kb-monitor:" << r.toString();
        }
    });
}
