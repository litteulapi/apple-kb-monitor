# Model fixtures (no hardware)

Used by `tests/live/re/test_model_inputs.py`. See `docs/HARDWARE-INPUTS-MODELS.md`.

| File | Kind | Source |
|---|---|---|
| `kernel_selftest_0005_05ac_0256.hex` | published | `report_descriptor` of the `AppleKeyboard` class in [`tools/testing/selftests/hid/tests/test_apple_keyboard.py`](https://github.com/torvalds/linux/blob/master/tools/testing/selftests/hid/tests/test_apple_keyboard.py) (Linux kernel, master branch, 2026-10-01), `input_info=(BUS_BLUETOOTH, 0x05AC, 0x0256)`. **Byte for byte identical** (224 B) to the descriptor captured from the real A1314 ISO (`../a1314_iso/report_descriptor.hex`). |
| `mk2021_0005_004c_029c_bt.hex` | published (third party) | Magic Keyboard 2021 `004C:029C`, version `0x0206`, 189 B, `esp_hidh_dev_dump` dump of an ESP32 host: [gist xloc/9f1ecca90ca29a9039c2a2468af70763](https://gist.github.com/xloc/9f1ecca90ca29a9039c2a2468af70763). Not checked on our hardware. |
| `hid-ids-apple.txt` | extract | Apple lines of [`drivers/hid/hid-ids.h`](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-ids.h) (master, 2026-10-01). |
| `families.json` | **synthetic** | one case per PID/family: `HID_ID`, BlueZ Modalias, expected family, reference descriptor (`null` = not published, to be captured). The Modalias other than the A1314's are built after the BlueZ format (`bluetooth:v004CpXXXXdYYYY`), arbitrary `d` version. |

Descriptors **not published / to be captured** (no reliable source found): A1255, A1314 2009, A1314 JIS,
Magic Keyboard 2015 (A1644/A1843) over BT and USB, Touch ID 2021/2024 (A2449/A2520), 2024 USB-C.
`hid-apple.c` only describes a fragment of the USB descriptor of the Magic Keyboard 2015 (the
`APPLE_RDESC_BATTERY` fix, 83 B, vendor usage `FF00:000B` replaced by `Generic Desktop/Keyboard`).
