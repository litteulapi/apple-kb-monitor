# udev - acces hidraw aux claviers Apple (#155)

`70-apple-kb-hidraw.rules` pose le tag `uaccess` (ACL logind rw pour l'utilisateur de la session active) sur les `/dev/hidraw*` des **claviers Bluetooth** Apple uniquement : la liste de product ids est celle de `APPLE_MODELS` (`apihub-app/akm-core/src/model.rs`). Souris, trackpads (Magic Mouse/Trackpad) et peripheriques USB filaires ne sont plus touches.

## Ou est la regle, et ce qui la masque (#296)

Le paquet l'installe dans `/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules`. udev lit les regles par **nom de fichier** : un fichier de meme nom dans `/etc/udev/rules.d/` (ou `/run/udev/rules.d/`) **remplace entierement** celui du paquet, sans avertissement. Constate le 2026-10-03 : un `/etc/udev/rules.d/70-apple-kb-hidraw.rules` d'une ancienne installation (aucun paquet proprietaire) donnait l'ACL a **tout** peripherique HID Apple (`KERNELS=="0005:05AC:*|0005:004C:*|0003:05AC:*"` : souris, trackpads, filaires), et toute correction de la regle packagee restait sans effet ; ni `pacman -Qkk` ni la CI ne le voyaient.

* Le scriptlet du paquet le signale a l'installation et a chaque mise a jour quand ce fichier existe et differe du fichier package ; il ne le supprime pas (ce peut etre une surcharge voulue, par exemple la variante « groupe dedie » ci-dessous).
* Verifier quelle regle s'applique : `udevadm cat 70-apple-kb-hidraw.rules` (premiere ligne = chemin du fichier retenu ; verifie le 2026-10-03 : `# /usr/lib/udev/rules.d/70-apple-kb-hidraw.rules` une fois l'orphelin retire) ; `pacman -Qo /etc/udev/rules.d/70-apple-kb-hidraw.rules` (« aucun paquet » = reste d'une installation a la main).
* Retirer une surcharge non voulue : `sudo rm /etc/udev/rules.d/70-apple-kb-hidraw.rules && sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=hidraw`.
* Surcharger volontairement : copier le fichier du paquet dans `/etc/udev/rules.d/` sous le meme nom et l'adapter ; le message du scriptlet le rappellera a chaque mise a jour (la copie ne suit plus le paquet).

## Compromis accepte : risque keylogger

Un `read()` sur hidraw rend les rapports d'entree bruts du clavier, donc les frappes. Avec `uaccess`, **tout processus tournant sous l'utilisateur de la session active** peut lire les frappes de ce clavier, y compris hors du modele d'isolation Wayland. L'ACL ouvre aussi l'ecriture (rapports de sortie/feature : LED, etc.).

Raison du choix : le demon `apple-kb-monitord` est un service **utilisateur** (il lit la batterie en GET_REPORT et pilote les LED) ; il lui faut donc l'acces dans la session sans groupe ni privilege. Le groupe `input` donnerait la meme capacite a une surface plus large.

## Poste multi-utilisateur : descripteur ouvert qui survit au changement de session (#204)

logind recalcule l'ACL `uaccess` quand la session active change, mais **ne revoque pas les descripteurs deja ouverts** (`HIDIOCREVOKE` ne s'applique qu'aux peripheriques pris par `TakeDevice`). Un compte A qui a ouvert `/dev/hidrawN` (`cat /dev/hidrawN`) puis cede la place a un compte B (changement rapide d'utilisateur) **continue de recevoir les rapports d'entree de B**, donc ses frappes et ses mots de passe. Niveau de preuve : analyse du code et des binaires, pas de demonstration en direct (ni second compte ni materiel pendant l'audit).

Mitigations :

* **Aucune mitigation cote udev n'est sure** : une regle udev ne peut ni fermer le descripteur d'un autre processus ni empecher l'`open()` tant que A est actif. `uaccess` reste donc un compromis valable pour un poste **mono-utilisateur** (cas cible).
* Poste partage : preferer la variante « groupe dedie » ci-dessous, ou deconnecter le clavier / fermer la session de A (`loginctl terminate-session`, ou `KillUserProcesses=yes` dans `logind.conf`) avant de ceder la place. Le changement rapide d'utilisateur avec un clavier Bluetooth Apple partage n'est pas supporte.
* Cote demon : il garde le noeud ouvert en ecoute passive uniquement tant que le clavier est connecte et ne lit jamais le contenu des frappes (horodatage seulement). Fermer le noeud a la perte de `login1.Session.Active` est une piste (suivi #204), pas un correctif du processus de A.

Alternative durcie (non fournie, a la charge de l'administrateur) : remplacer `TAG+="uaccess"` par `GROUP="akm", MODE="0660"` et executer le demon sous un compte du groupe `akm` (unite systemd systeme avec `SupplementaryGroups=akm`). Cela perd l'acces de l'application graphique lancee par l'utilisateur.

Verification : `udevadm verify udev/70-apple-kb-hidraw.rules` ; `getfacl /dev/hidrawN` sur un clavier connecte (ACL presente) et sur une souris Apple (ACL absente).

## hwdb du mapping des touches (#247)

Aucun fichier hwdb n'est fourni par le paquet (défaut = mapping du noyau). `akmctl keymap apply`
fait écrire `/etc/udev/hwdb.d/90-apple-kb-monitor.hwdb` par `akm-keymap-helper` (entrées
`evdev:input:b0005v05ACpPPPP*` + `KEYBOARD_KEY_<usage HID>=<nom>`, liste blanche), puis
`systemd-hwdb update` et `udevadm trigger --subsystem-match=input --action=change`.
`akmctl keymap reset` le retire. Détails : docs/TOUCHES.md.
