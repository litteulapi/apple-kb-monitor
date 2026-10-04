// SPDX-License-Identifier: GPL-2.0-or-later
// "Apple Keyboard" module of System Settings.
#include "buscall.h"

#include <KPluginFactory>
#include <KQuickConfigModule>

#include <QTimer>

class AppleKeyboardKcm : public KQuickConfigModule
{
    Q_OBJECT
    Q_PROPERTY(QStringList args READ args NOTIFY argsChanged)
    // D-Bus calls of Store.qml with their real deadlines
    Q_PROPERTY(QObject *busCall READ busCall CONSTANT)

public:
    AppleKeyboardKcm(QObject *parent, const KPluginMetaData &data, const QVariantList &args)
        : KQuickConfigModule(parent, data)
    {
        setArgs(args);
        setButtons(Apply | Default);
        // System Settings already showing the module passes the new arguments (widget's "Edit the keys").
        connect(this, &KAbstractConfigModule::activationRequested, this, [this](const QVariantList &a) {
            setArgs(a);
            Q_EMIT argsChanged();
        });
    }

    QStringList args() const { return m_args; }
    QObject *busCall() const { return m_busCall; }

    void load() override
    {
        KQuickConfigModule::load();
        Q_EMIT loadRequested();
    }
    void save() override
    {
        Q_EMIT saveRequested();
        QTimer::singleShot(0, this, &AppleKeyboardKcm::saveReturned);
    }
    void defaults() override
    {
        KQuickConfigModule::defaults();
        Q_EMIT defaultsRequested();
    }

Q_SIGNALS:
    void argsChanged();
    void loadRequested();
    void saveRequested();
    void saveReturned();
    void defaultsRequested();

private:
    void setArgs(const QVariantList &args)
    {
        m_args.clear();
        for (const QVariant &a : args) {
            m_args << a.toString();
        }
    }

    QStringList m_args;
    BusCall *m_busCall = new BusCall(this);
};

K_PLUGIN_CLASS_WITH_JSON(AppleKeyboardKcm, "kcm_applekeyboard.json")

#include "kcm.moc"
