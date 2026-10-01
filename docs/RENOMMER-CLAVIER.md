# Renommer le clavier (#141)

Deux noms existent. Seul le premier est implémenté.

| | (a) Alias côté poste | (b) Nom stocké dans le clavier |
|---|---|---|
| Où | BlueZ, `org.bluez.Device1.Alias`, persisté dans `/var/lib/bluetooth/<adaptateur>/<MAC>/info` (`Alias=`) | Firmware du clavier (BCM2042), rapports Feature `0x51-0x54` (32 o ASCII) |
| Valeur actuelle | `Clavier de maria #1` | `Clavier de maria #1` (identique à `HID_NAME` et au nom distant BlueZ) [mesuré, docs/AUDIT-DECODAGE-HID.md] |
| Visible par | ce poste uniquement (KDE Bluetooth, `bluetoothctl`, tray, widget, `akmctl`) | tout appareil qui s'appaire au clavier |
| Risque | nul : propriété BlueZ réversible, ni déconnexion ni réappairage | voir §3 |
| État | **implémenté** | **recherche seulement, aucune écriture** |

## 1. (a) Alias : ce qui est livré

* `akmctl rename <nom>` / `akmctl rename --reset` (`--mac` pour cibler un clavier). Codes retour : 0 OK, 1 erreur (nom refusé, BlueZ), 2 démon absent.
* D-Bus session : `com.agenceapi.AppleKbMonitor1.SetAlias(s mac, s nom) -> s` sur l'objet racine, `Device.SetAlias(s nom) -> s` sur l'objet clavier ; propriété `Name` (alias, sinon nom propre) sur les deux. Nom vide = retour au nom d'origine (comportement BlueZ).
* Validation (`akm-core::alias`, partagée par tous les clients) : espaces de bord retirés, max 64 caractères et 248 octets UTF-8 (limite HCI), refus des caractères de contrôle (Cc), séparateurs de ligne/paragraphe, caractères invisibles et surcharges bidirectionnelles (U+200B-200F, 2028-202E, 2060-2064, 2066-2069, FEFF). Le démon n'écrit que sur un appareil BlueZ dont le `Modalias` est Apple.
* Tray : « Renommer le clavier… » (boîte `kdialog`, sinon `zenity`, sinon ouverture de la fenêtre). Fenêtre egui : champ + Renommer / Réinitialiser. Widget Plasma : champ + Renommer / Réinitialiser.
* Affichage : infobulle du tray, en-tête du menu, `akmctl status [--json]` (`name`, `alias`), `apple-kb-monitord --json` (`name`, `keyboard.device.alias`), JSON du snapshot, widget.
* Un renommage fait ailleurs (`bluetoothctl`, Paramètres KDE) est repris via `PropertiesChanged.Alias`.
* Limite : `HID_NAME` du noyau (`/sys/.../uevent`) garde l'ancien nom jusqu'à la prochaine reconnexion ; l'application affiche l'alias en priorité.

## 2. (b) Ce que les sources établissent

* **hid-apple (noyau)** : aucune référence aux rapports `0x51-0x53` ni à un nom. Seules écritures : rétroéclairage (`0xB0`, Output) et lecture `0xBF`. Le noyau ne sait donc ni lire ni écrire ce nom ; il utilise `hdev->name`, fourni par BlueZ à la création du périphérique `uhid`.
* **Descripteur HID de l'A1314** : seul `0x09` est déclaré en Feature ; `0x51-0x54` répondent à `GET_FEATURE` mais ne sont pas déclarés (docs/HARDWARE-RAPPORTS-HID.md). Leur sémantique d'écriture est inconnue.
* **Recherche web** : le seul document trouvé sur ces rapports est le dépôt `litteulapi/apple-kb-monitor` (https://github.com/litteulapi/apple-kb-monitor) qui les décrit comme lus depuis la ROM de la puce ; aucune source publique ne décrit une écriture du nom (ni outil Apple, ni projet libre). Le nom du BCM2042 est en général fixé dans la configuration du firmware ; rien ne prouve qu'il soit en flash modifiable.
* **Comment le nom est relu** [connaissance BlueZ, non re-vérifiée ici] : BlueZ obtient le nom distant par EIR ou HCI *Remote Name Request* à l'appairage/découverte, le met en cache (`Name=` dans le fichier `info`) et le republie ; l'`Alias` est un champ distinct, prioritaire à l'affichage. Une modification du nom dans le clavier ne serait visible qu'après un nouveau *Remote Name Request*, souvent jamais tant que le cache existe ; il faudrait vider le cache (donc risquer le pairage) ou attendre une redécouverte. Le nom noyau (`HID_NAME`) vient de ce cache.

## 3. (b) Risques

1. **Brique / état firmware** : écrire un rapport vendeur non documenté sur un firmware ancien et sans mode de récupération connu. L'effet de `SET_FEATURE` sur `0x51-0x54` est inconnu ; un rapport voisin (calibration, `0x5A/0x60/0xEB`, ou le mode de mise à jour) pourrait être touché par une erreur de numéro. Irréversible sans outil Apple.
2. **Perte de pairage** : si le nom fait partie des données couvertes par la clé de lien ou si le clavier redémarre, le pairage peut sauter ; un clavier Bluetooth sans pairage ne peut pas se re-pairer sans autre clavier.
3. **Nom écrasé** : si le nom est reconstruit depuis la ROM à la mise sous tension, l'écriture disparaît au changement de piles.
4. **Gain faible** : l'alias (a) donne déjà le même résultat sur ce poste, sans risque.

## 4. Recommandation

Rester sur (a). Ne pas écrire dans le clavier : bénéfice cosmétique, perte potentielle d'un clavier de 2009 non remplaçable à distance. Si le gérant veut quand même explorer (b), procéder par étapes avec OK explicite à chacune :

1. Lecture seule : sauvegarder les rapports `0x51-0x54` et la liste complète des rapports lisibles (déjà fait en partie, docs/HARDWARE-RAPPORTS-HID.md).
2. Avoir un second clavier filaire/Bluetooth appairé et fonctionnel, et la photo du clavier de secours.
3. Un seul `SET_FEATURE` sur `0x51` avec la valeur **actuelle** (écriture neutre) pour observer si le firmware l'accepte (code d'erreur, relecture identique) ; arrêt à la première anomalie.
4. Seulement ensuite, modifier un octet, relire, redémarrer le clavier (piles), relire.
5. Vérifier la relecture côté BlueZ (`bluetoothctl info`, `Name` vs `Alias`) avant d'espérer un effet.

Aucune de ces étapes n'a été exécutée.
