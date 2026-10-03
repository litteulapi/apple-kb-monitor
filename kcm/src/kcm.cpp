// SPDX-License-Identifier: GPL-2.0-or-later
// "Clavier Apple" module of System Settings (#250).
//
// Plasma 6 / KF6 only loads a KCM from a plugin (KQuickConfigModuleLoader ->
// KPluginFactory); the QML pages are compiled into it as resources under
// qrc:/kcm/kcm_applekeyboard/. This class only bridges the KCM life cycle
// (load / save / defaults) to QML and owns the asynchronous I/O object.
#include <KLocalizedString>
#include <KPluginFactory>
#include <KQuickConfigModule>

#include <QTimer>

#include "bridge.h"

class AppleKeyboardKcm : public KQuickConfigModule
{
    Q_OBJECT
    Q_PROPERTY(AkmBridge *bridge READ bridge CONSTANT)
    // Arguments given by the caller (kcmshell6 kcm_applekeyboard --args keys
    // opens the Keys tab).
    Q_PROPERTY(QStringList args READ args CONSTANT)

public:
    AppleKeyboardKcm(QObject *parent, const KPluginMetaData &data, const QVariantList &args)
        : KQuickConfigModule(parent, data)
        , m_bridge(new AkmBridge(this))
    {
        for (const QVariant &a : args) {
            m_args << a.toString();
        }
        // "Apply" saves the notification settings (config.toml); every
        // privileged action has its own button and its own polkit prompt.
        setButtons(Apply | Default);
    }

    AkmBridge *bridge() const { return m_bridge; }
    QStringList args() const { return m_args; }

    void load() override
    {
        KQuickConfigModule::load();
        Q_EMIT loadRequested();
    }
    void save() override
    {
        // needsSave is cleared by QML once the file is really written.
        Q_EMIT saveRequested();
        // #285: System Settings (KCModuleQml) sets needsSave to false right
        // after this returns, whatever the page did. The page then says again
        // whether it is still modified (save refused, or still writing), so
        // that Apply and the "unsaved changes" question stay.
        QTimer::singleShot(0, this, &AppleKeyboardKcm::saveReturned);
    }
    void defaults() override
    {
        KQuickConfigModule::defaults();
        Q_EMIT defaultsRequested();
    }

Q_SIGNALS:
    void loadRequested();
    void saveRequested();
    void saveReturned();
    void defaultsRequested();

private:
    AkmBridge *m_bridge;
    QStringList m_args;
};

K_PLUGIN_CLASS_WITH_JSON(AppleKeyboardKcm, "kcm_applekeyboard.json")

#include "kcm.moc"
