# RE — Pilotes et updaters Apple de l'époque (2009-2012) pour l'A1314 (BCM2042, firmware 0x0050)

Analyse **statique, en lecture seule**, de logiciels Apple **publiquement distribués** par les serveurs
d'Apple, à des fins d'interopérabilité avec le clavier du gérant (A1314 ISO, `05AC:0256` = 598,
bcdDevice `0x0050`). Complète `RE-PILOTE-MACOS.md` (macOS 26.5), qui n'envoie presque rien de spécifique.

* Aucun binaire Apple, aucun code décompilé, aucune clé n'est commité : seulement des faits courts
  (IDs, tailles, formats, noms de symboles et de clés).
* Aucun accès au clavier réel, aucune écriture HID. Aucun outil de flash n'est écrit.

Niveaux de preuve : **[plist]**, **[désassemblage]**, **[chaîne]**, **[déduction]**, **[mesuré]** (renvoie
à nos mesures : `RE-HID-EXHAUSTIF.md`, `HARDWARE-RAPPORTS-HID.md`).

*(document rédigé au fil de l'eau ; sections complétées par commits successifs)*

## 1. Sources téléchargées (serveurs Apple publics) et intégrité

| Paquet | URL officielle (Apple) | Taille | Empreinte |
|---|---|---|---|
| `AlKybdSoftwareUpdate.pkg` (« Wireless Keyboard Update 2.0 », produit `041-85086`) | `http://swcdn.apple.com/content/downloads/25/51/041-85086-A_0O8GEX5RKH/c20e4b3bz7iq0rp0rgdreyuy4rr1yzpix8/AlKybdSoftwareUpdate.pkg` | 11 332 239 o (= taille du catalogue) | sha256 `efb9618a…4abd` ; TOC xar SHA-1 vérifiée ; signature RSA, 3 certificats |
| `BluetoothFirmwareUpdate2.0.1.pkg` (produit `041-85074`) | `http://swcdn.apple.com/content/downloads/35/36/041-85074-A_MOUDL4NHT5/x3thzouodq1tckv99fa2albrqjf0k2ns4y/BluetoothFirmwareUpdate2.0.1.pkg` | 1 671 448 o (= catalogue) | sha256 `9f064473…7428` ; TOC SHA-1 OK ; RSA |
| `WirelessKybdFirmwareUpdate.dmg` (« 2009 Aluminum Keyboard Firmware Update 1.0 », support.apple.com/en-us/106755) | `https://updates.cdn-apple.com/2019/cert/041-87809-20191017-6ee067c4-5369-4075-bc88-f1cfb623f02b/WirelessKybdFirmwareUpdate.dmg` | 886 799 o | sha256 `449d819b…905d` ; pkg interne : TOC SHA-1 OK ; RSA |
| `MacOSXUpdCombo10.7.5.pkg` (produit `041-98155`) | `http://swcdn.apple.com/content/downloads/30/25/041-98155-A_E2HC6UPBOQ/y8k2fq4q0u2jo4ci8txpxkw96l9dzfsi19/MacOSXUpdCombo10.7.5.pkg` | 2 041 831 704 o | lu en flux (`Payload` 2 035 347 918 o, SHA-1 du flux `e8b7b6b5…` ), TOC SHA-1 OK ; seuls les composants Bluetooth/HID ont été extraits, rien n'a été stocké |

Les URL proviennent des catalogues publics de Software Update
(`https://swscan.apple.com/content/catalogs/others/index-lion-snowleopard-leopard.merged-1.sucatalog`).
Apple ne publie pas d'empreinte pour ces paquets : l'intégrité est établie par la taille annoncée dans le
catalogue, la somme SHA-1 de la table des matières xar et les sommes de chaque fichier de l'archive
(toutes vérifiées), plus la présence d'une signature RSA du paquet (chaîne de 3 certificats Apple).
Le Combo 10.6.8 (`041-98179`) a été écarté : il est découpé en 13 parties et la partie contenant les
répertoires des kexts Bluetooth/HID ne contient que des liens de signature, pas les binaires.

Outils (sans sudo) : lecteur xar en Python (vérification TOC + sommes), `gzip`/`cpio`, `7z` (DMG),
`llvm-objdump`/`llvm-nm`/`llvm-lipo`, capstone 5 (désassemblage i386/x86_64).
