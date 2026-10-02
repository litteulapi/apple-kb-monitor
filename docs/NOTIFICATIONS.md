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
| `BatteryLow` | seuil 30 / 15 % franchi | normale, 12 s | Ouvrir, Me rappeler demain | `battery` |
| `BatteryCritical` | seuil 5 % franchi | critique, persistante | Ouvrir | `battery` |
| `KeyboardAlert` | alerte du clavier (Input `0x30`) | normale ou critique | Ouvrir | `battery` |
| `KeyboardDisconnected` | liaison perdue | basse, transitoire | Ouvrir | `link` |
| `KeyboardReconnected` | liaison rétablie | basse, transitoire | Ouvrir | `link` |
| `KeyboardOff` | clavier éteint (annoncé avant la coupure) | basse, transitoire | Ouvrir | `link` |
| `KeyboardUnreachable` | clavier appairé qui ne répond plus | normale | Ouvrir | `link` |
| `RepairNeeded` | ré-appairage nécessaire | critique, persistante | Réparer…, Ouvrir | `repair` |
| `KeyboardRemoved` | clavier supprimé du poste depuis l'extérieur (Oublier de Plasma, `bluetoothctl remove`) (#252) | normale, 12 s | Réparer…, Ouvrir | `repair` |
| `BatteryEstimate` | l'unique rappel « estimation selon vos piles » quand PowerDevil alerte déjà (#254) | normale, 12 s | Ouvrir, Me rappeler demain | `battery` |
| `FirmwareUpdate` | firmware plus récent connu (table embarquée) : une fois par version, mémorisé | normale | Ouvrir, Me rappeler demain | `firmware` |
| `BatteryReminder` | « changez les piles » : tension lissée `0x49` sous le seuil Bas puis Critique **du clavier** (`0x60` = `0x5A`, 2506 / 2404 mV sur A1314), avec l'indication du clavier en % | Bas : normale, 15 s ; Critique : critique, persistante | Ouvrir, Me rappeler demain, Ignorer ce rappel | `battery` |
| `BatteryReplaced` | piles neuves détectées | normale | Ouvrir | `battery` |
| `Error` | échec d'une opération | critique | aucun | `error` |

* Urgence critique : `expire_timeout = 0` (ne disparaît pas seule). Basse : 6 s,
  non gardée dans l'historique (`transient`).
* Un clic sur le corps de la notification (`default`) fait « Ouvrir ».
* Remplacement : le démon garde l'identifiant renvoyé par le serveur pour chaque
  emplacement et le renvoie dans `replaces_id` : une alerte de piles ne s'empile
  pas, la reconnexion remplace la déconnexion. Une notification fermée par
  l'utilisateur (`NotificationClosed`) est oubliée.
* Déclenchement de `BatteryReminder` et `FirmwareUpdate` (`akm-core::reminder`,
  appelé par l'acteur après chaque lecture) : **une fois par franchissement**.
  Un niveau de piles n'est réarmé que si la tension remonte de 50 mV au-dessus
  de son seuil (bruit de l'ADC ignoré), ou par des piles neuves ; le firmware
  est réannoncé pour une version plus récente, ou après être redevenu à jour.
  L'état est gardé dans `$XDG_STATE_HOME/apple-kb-monitor/notices.json` (écrit
  seulement quand l'historique l'est) : un redémarrage du démon ne répète pas
  un rappel déjà vu. Sans seuils lus (lecture noyau seule), pas de rappel de
  tension. `[alerts] enabled = false` coupe aussi le rappel de piles.

## Boutons

| Bouton | Effet |
|---|---|
| Ouvrir | `org.freedesktop.Application.Activate` sur `com.agenceapi.AppleKbMonitor` (activation D-Bus, instance unique, jeton d'activation Wayland fourni par Plasma via `ActivationToken` s'il existe), sinon `apihub-app` |
| Réparer… | `akmctl repair` dans un terminal (`konsole --hold -e`, puis les autres terminaux connus) |
| Me rappeler demain | la notification est mise de côté et réémise à l'identique 24 h plus tard (#110), y compris après un redémarrage du démon (`deferred-notifications.json`) ; si l'échéance tombe dans une plage « ne pas déranger », elle attend la fin de la plage ; des piles neuves, ou « Ignorer ce rappel », annulent un rappel de piles en attente. Proposé par `BatteryLow`, `BatteryEstimate`, `BatteryReminder` et `FirmwareUpdate`, jamais par ce qui doit être traité tout de suite (`BatteryCritical`, `RepairNeeded`) |
| Ignorer ce rappel | coupe le rappel du seuil Bas jusqu'à la détection de piles neuves ; le rappel Critique reste émis |

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

`[alerts] enabled`, `[notifications] connection` / `battery_replaced` et
`--no-notify` coupent l'émission côté démon. Le reste (popup, son, historique,
ne pas déranger) se règle dans Plasma, par événement.

### Plages horaires « ne pas déranger » (#91)

`[notifications] quiet_hours = "22:00-07:00"` (heure locale ; plusieurs plages
séparées par des virgules ; une plage peut traverser minuit ; `""` = aucune,
valeur par défaut). Dans une plage :

* une notification **non critique** n'est pas envoyée : elle est journalisée
  (`notification held until 07:00 (quiet hours 22:00-07:00): [BatteryLow] …`)
  et gardée, une par emplacement de remplacement (la dernière), puis émise à la
  fin de la plage (`quiet hours over: showing […]`) ;
* une notification **critique** (piles critiques, ré-appairage nécessaire,
  erreur) part toujours tout de suite.

Ce qui attend est gardé dans
`$XDG_STATE_HOME/apple-kb-monitor/deferred-notifications.json` (32 entrées au
plus) : un redémarrage du démon ne perd rien. Le mode « Ne pas déranger » de
Plasma reste appliqué par Plasma lui-même (`ShowPopupsInDndMode` pour les
événements critiques du notifyrc) : les deux se cumulent. Logique :
`akm-core::quiet` (plages), `akm-core::deferred` (file d'attente),
`apple-kb-monitord::notify_policy` (décision, horloge injectable).

## Tester

```sh
# Le notifyrc est installé et lisible
ls /usr/share/knotifications6/apple-kb-monitor.notifyrc

# Tests automatiques (bus privé + faux serveur, rien n'arrive au bureau)
cargo test -p apple-kb-monitord --test notify_kde --test notify_mute --test notify_quiet_remind
```

Sur le bureau : Paramètres système → Notifications → « Moniteur de clavier
Apple » doit lister les événements ; décocher « Afficher les fenêtres
contextuelles » pour `BatteryLow` supprime la bulle sans toucher au démon.
`dbus-monitor "interface=org.freedesktop.Notifications"` montre `desktop-entry`,
`x-kde-eventId` et les actions de chaque `Notify`.
