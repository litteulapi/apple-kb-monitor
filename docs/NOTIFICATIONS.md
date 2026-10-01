# Notifications et intégration KDE (#249)

Le démon `apple-kb-monitord` parle le dialecte de `KNotification` à
`org.freedesktop.Notifications` : Plasma le traite comme une application à part
entière (Paramètres système → Notifications → Moniteur de clavier Apple).

## Ce qui relie le démon à Plasma

| Élément | Valeur | Rôle |
|---|---|---|
| `data/apple-kb-monitor.notifyrc` | installé dans `/usr/share/knotifications6/` | déclare l'application et ses événements (noms FR/EN) |
| hint `desktop-entry` | `com.agenceapi.AppleKbMonitor` | rattache à `com.agenceapi.AppleKbMonitor.desktop` (nom, icône) |
| hint `x-kde-appname` | `apple-kb-monitor` | nom du fichier notifyrc |
| hint `x-kde-eventId` | `BatteryLow`, ... | section `[Event/<id>]` : Plasma applique popup, son, historique, « Ne pas déranger » de l'utilisateur |
| `app_name` | `Apple Keyboard Monitor` | nom affiché (= `Name=` du `.desktop`) |

Les trois hints sont ceux qu'émet `libKF6Notifications` (vérifiés dans le binaire
installé). Les réglages de l'utilisateur sont écrits par Plasma dans
`~/.config/apple-kb-monitor.notifyrc`.

## Événements

| Id | Quand | Urgence | Boutons | Emplacement (remplacement) |
|---|---|---|---|---|
| `BatteryLow` | seuil 30 / 15 % franchi | normale, 12 s | Ouvrir | `battery` |
| `BatteryCritical` | seuil 5 % franchi | critique, persistante | Ouvrir | `battery` |
| `KeyboardAlert` | alerte du clavier (Input `0x30`) | normale ou critique | Ouvrir | `battery` |
| `KeyboardDisconnected` | liaison perdue | basse, transitoire | Ouvrir | `link` |
| `KeyboardReconnected` | liaison rétablie | basse, transitoire | Ouvrir | `link` |
| `KeyboardOff` | clavier éteint (annoncé avant la coupure) | basse, transitoire | Ouvrir | `link` |
| `KeyboardUnreachable` | clavier appairé qui ne répond plus | normale | Ouvrir | `link` |
| `RepairNeeded` | ré-appairage nécessaire | critique, persistante | Réparer…, Ouvrir | `repair` |
| `KeyboardRemoved` | clavier supprimé du poste depuis l'extérieur (Oublier de Plasma, `bluetoothctl remove`) (#252) | normale, 12 s | Réparer…, Ouvrir | `repair` |
| `BatteryEstimate` | l'unique rappel « estimation selon vos piles » quand PowerDevil alerte déjà (#254) | normale, 12 s | Ouvrir | `battery` |
| `FirmwareUpdate` | firmware plus récent connu | normale | Ouvrir | `firmware` |
| `BatteryReminder` | rappel « changez les piles » | normale, 15 s | Ouvrir, Ignorer ce rappel | `battery` |
| `BatteryReplaced` | piles neuves détectées | normale | Ouvrir | `battery` |
| `Error` | échec d'une opération | critique | aucun | `error` |

* Urgence critique : `expire_timeout = 0` (ne disparaît pas seule). Basse : 6 s,
  non gardée dans l'historique (`transient`).
* Un clic sur le corps de la notification (`default`) fait « Ouvrir ».
* Remplacement : le démon garde l'identifiant renvoyé par le serveur pour chaque
  emplacement et le renvoie dans `replaces_id` : une alerte de piles ne s'empile
  pas, la reconnexion remplace la déconnexion. Une notification fermée par
  l'utilisateur (`NotificationClosed`) est oubliée.
* `firmware_update` et `battery_reminder` sont prêts (`notify.rs`) mais aucun
  déclencheur du démon ne les appelle encore : l'événement existe dans Plasma
  (réglable), l'émission reste à brancher.

## Boutons

| Bouton | Effet |
|---|---|
| Ouvrir | `org.freedesktop.Application.Activate` sur `com.agenceapi.AppleKbMonitor` (activation D-Bus, instance unique, jeton d'activation Wayland fourni par Plasma via `ActivationToken` s'il existe), sinon `apihub-app` |
| Réparer… | `akmctl repair` dans un terminal (`konsole --hold -e`, puis les autres terminaux connus) |
| Ignorer ce rappel | coupe `BatteryReminder` jusqu'à la détection de piles neuves |

Le signal `ActionInvoked` est écouté par un fil dédié (`kb-notify-actions`), sur sa
propre connexion : ni l'acquisition ni l'envoi n'attendent. Chaque action tourne
dans son propre fil. Garde-fous : le signal doit venir du propriétaire actuel du
nom `org.freedesktop.Notifications` (un autre client ne peut pas déclencher
« Réparer »), et le bouton doit être proposé par cette notification. 64
notifications en attente au plus.

## Langue

Français si `LC_ALL`, `LC_MESSAGES` ou `LANG` (le premier non vide) commence par
`fr`, anglais sinon. Les noms d'événements sont traduits dans le notifyrc
(`Name[fr]`, `Comment[fr]`) et dans le `.desktop`.

## Configuration

`config.toml` inchangé : `[alerts] enabled`, `[notifications] connection` /
`battery_replaced` et `--no-notify` coupent l'émission côté démon, comme avant.
Le reste (popup, son, historique, ne pas déranger) se règle dans Plasma, par
événement.

## Tester

```sh
# Le notifyrc est installé et lisible
ls /usr/share/knotifications6/apple-kb-monitor.notifyrc

# Tests automatiques (bus privé + faux serveur, rien n'arrive au bureau)
cargo test -p apple-kb-monitord --test notify_kde --test notify_mute
```

Sur le bureau : Paramètres système → Notifications → « Moniteur de clavier
Apple » doit lister les événements ; décocher « Afficher les fenêtres
contextuelles » pour `BatteryLow` supprime la bulle sans toucher au démon.
`dbus-monitor "interface=org.freedesktop.Notifications"` montre `desktop-entry`,
`x-kde-eventId` et les actions de chaque `Notify`.
