# LG 34GN850 — Reverse Engineering Notes

## Scaler Chip
- Reports: Myson Century (DDC Controller ID=6), likely Realtek RTD2795/2796
- Firmware: 3.0, VCP Version: 2.1

## I2C Bus (i2c-6)
- 0x30: E-DDC Segment Pointer (empty)
- 0x37: DDC/CI Scaler (raw register reads blocked, returns 0x6E echo)
- 0x50: EDID EEPROM (256 bytes)

## OSD Control via DDC/CI
| VCP | Function | Notes |
|-----|----------|-------|
| 0x62 | Volume | ONLY VCP that triggers native OSD slider |
| 0xCA | OSD Lock | 1=locked, 2=unlocked |
| 0xCC | OSD Language | 0=EN, 2=FR, 3=DE |
| 0x08 | Restore Color | Triggers CAUTION message OSD |
| 0x10 | Brightness | Changes backlight silently, NO OSD |

## Mirror Registers (0xE8-0xEF = copies of standard VCPs)
0xE8=Brightness, 0xE9=Contrast, 0xEA=Color Preset, 0xEB=Picture Mode
0xEC=Red, 0xED=Green, 0xEE=Blue, 0xEF=Input Source

## Undocumented VCPs
| VCP | Value | Likely Function |
|-----|-------|-----------------|
| 0x69 | 6500 | Color Temperature (Kelvin direct) |
| 0xC1 | 44/300 | Backlight PWM (wider range) |
| 0x87 | 50/100 | Sharpness |
| 0x6C/6E/70 | 128 | Video Black Level RGB |
| 0xDE | 44/100 | Scratch Pad (brightness mirror) |

## LG Vendor VCPs
0xF4=Response Time, 0xF5=Super Resolution, 0xF6=DAS Mode
0xF7=FreeSync, 0xF8=HDR, 0xF9=Gamma, 0xFA=Color Gamut, 0xFE=Black Level

## Conclusion
Brightness OSD not triggerable via DDC/CI. Firmware hardcoded.
Volume OSD trick: write same value to 0x62 triggers OSD without audio change.
256/256 VCPs scanned, 48 with data, 23 undocumented, all decoded.
