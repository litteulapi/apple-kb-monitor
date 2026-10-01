# udev - acces hidraw aux claviers Apple (#155)

`70-apple-kb-hidraw.rules` pose le tag `uaccess` (ACL logind rw pour l'utilisateur de la session active) sur les `/dev/hidraw*` des **claviers Bluetooth** Apple uniquement : la liste de product ids est celle de `APPLE_MODELS` (`apihub-app/akm-core/src/model.rs`). Souris, trackpads (Magic Mouse/Trackpad) et peripheriques USB filaires ne sont plus touches.

## Compromis accepte : risque keylogger

Un `read()` sur hidraw rend les rapports d'entree bruts du clavier, donc les frappes. Avec `uaccess`, **tout processus tournant sous l'utilisateur de la session active** peut lire les frappes de ce clavier, y compris hors du modele d'isolation Wayland. L'ACL ouvre aussi l'ecriture (rapports de sortie/feature : LED, etc.).

Raison du choix : le demon `apple-kb-monitord` est un service **utilisateur** (il lit la batterie en GET_REPORT et pilote les LED) ; il lui faut donc l'acces dans la session sans groupe ni privilege. Le groupe `input` donnerait la meme capacite a une surface plus large.

Alternative durcie (non fournie, a la charge de l'administrateur) : remplacer `TAG+="uaccess"` par `GROUP="akm", MODE="0660"` et executer le demon sous un compte du groupe `akm` (unite systemd systeme avec `SupplementaryGroups=akm`). Cela perd l'acces de l'application graphique lancee par l'utilisateur.

Verification : `udevadm verify udev/70-apple-kb-hidraw.rules` ; `getfacl /dev/hidrawN` sur un clavier connecte (ACL presente) et sur une souris Apple (ACL absente).
