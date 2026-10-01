# Fixtures modèles (sans matériel)

Utilisées par `tests/live/re/test_entrees_modeles.py`. Voir `docs/HARDWARE-ENTREES-MODELES.md`.

| Fichier | Nature | Source |
|---|---|---|
| `kernel_selftest_0005_05ac_0256.hex` | publié | `report_descriptor` de la classe `AppleKeyboard` dans [`tools/testing/selftests/hid/tests/test_apple_keyboard.py`](https://github.com/torvalds/linux/blob/master/tools/testing/selftests/hid/tests/test_apple_keyboard.py) (noyau Linux, branche master, 2026-10-01), `input_info=(BUS_BLUETOOTH, 0x05AC, 0x0256)`. **Identique octet pour octet** (224 o) au descripteur capturé de l'A1314 ISO réel (`../a1314_iso/report_descriptor.hex`). |
| `mk2021_0005_004c_029c_bt.hex` | publié (tiers) | Magic Keyboard 2021 `004C:029C`, version `0x0206`, 189 o, dump `esp_hidh_dev_dump` d'un hôte ESP32 : [gist xloc/9f1ecca90ca29a9039c2a2468af70763](https://gist.github.com/xloc/9f1ecca90ca29a9039c2a2468af70763). Non vérifié sur notre matériel. |
| `hid-ids-apple.txt` | extrait | lignes Apple de [`drivers/hid/hid-ids.h`](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-ids.h) (master, 2026-10-01). |
| `families.json` | **synthétique** | un cas par PID/famille : `HID_ID`, Modalias BlueZ, famille attendue, descripteur de référence (`null` = non publié, à capturer). Les Modalias autres que celui de l'A1314 sont construits selon le format BlueZ (`bluetooth:v004CpXXXXdYYYY`), version `d` arbitraire. |

Descripteurs **non publiés / à capturer** (aucune source fiable trouvée) : A1255, A1314 2009, A1314 JIS,
Magic Keyboard 2015 (A1644/A1843) en BT et en USB, Touch ID 2021/2024 (A2449/A2520), 2024 USB-C.
Le `hid-apple.c` décrit seulement un fragment du descripteur USB du Magic Keyboard 2015 (correctif
`APPLE_RDESC_BATTERY`, 83 o, usage vendor `FF00:000B` remplacé par `Generic Desktop/Keyboard`).
