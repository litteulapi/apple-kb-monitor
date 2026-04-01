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


## RAM Map (via sidechannel 0xD1)

### Active Regions
| Region | Size | Purpose |
|--------|------|---------|
| 0x0F00-0x0FFF | 256B | OSD/Settings (dynamic) |
| 0x1020-0x11BF | 416B | Unknown data |
| 0x6660-0x67BF | 352B | Unknown data |
| 0x82D0-0x846F | 416B | Unknown data |
| 0xBC10-0xBDAF | 416B | Unknown data |
| 0xF5B0-0xF6BF | 272B | Unknown data |

### OSD Region (0x0F00-0x0FFF) Diff Results
| Action | Bytes Changed | OSD Displayed |
|--------|--------------|---------------|
| Brightness 44→80 | 23 | NO |
| Volume 50→80 | 11 | YES |
| Language FR→EN | 38 | NO |
| Picture Mode 45→1 | 31 | NO |

### Volume-Exclusive RAM Changes (OSD trigger candidates)
- 0x0F1E: 88→04 (flag set when OSD displays)
- 0x0F1F: 88→04 (flag set when OSD displays)
- These are OUTPUT flags (consequence of OSD), not INPUT triggers

### Sidechannel Commands (address 0x50)
| VCP | Function | Tested |
|-----|----------|--------|
| 0xC9 | Firmware version | Returns raw bytes |
| 0xCA | Model string | Returns "34GN850" |
| 0xCC | OSD Language | 0=EN, 2=FR, 3=DE (writable!) |
| 0xD1 | RAM read | Works — full RAM access |
| 0xD5 | RAM write | Accepted but no visible effect on OSD |
| 0xF4 | Input switch (LG proprietary) | Not tested (risk) |

### Conclusion
VCP dispatch table is in SPI flash firmware, not in RAM.
Cannot patch OSD behavior without physical access to SPI flash.
OSD brightness via DDC/CI: CONFIRMED IMPOSSIBLE on LG 34GN850.


## Capabilities String (parsed from DDC)

Model: GN850, MCCS 2.1

### Picture Modes (VCP 0x15) — 16 modes from capabilities
| Value | Mode |
|-------|------|
| 0x01 (1) | Reader |
| 0x06 (6) | Color Weakness / Gamer 2 |
| 0x11 (17) | Custom |
| 0x13 (19) | RTS |
| 0x14 (20) | Vivid |
| 0x15 (21) | sRGB |
| 0x18 (24) | sRGB/SMPTE-C |
| 0x19 (25) | EBU |
| 0x20 (32) | Photo |
| 0x22 (34) | ? |
| 0x23 (35) | ? |
| 0x24 (36) | ? |
| 0x28 (40) | FPS Game 1 |
| 0x29 (41) | FPS Game 2 |
| 0x32 (50) | ? |
| 0x48 (72) | Cinema |

### All Controllable Features
| VCP | Feature | Current | Range |
|-----|---------|---------|-------|
| 0x10 | Brightness | 50 | 0-100 |
| 0x12 | Contrast | 50 | 0-100 |
| 0x14 | Color Preset | User(11) | 5=6500K,8=9300K,B=User |
| 0x15 | Picture Mode | Custom(45) | 16 modes |
| 0x16 | Red Gain | 50 | 0-100 |
| 0x18 | Green Gain | 50 | 0-100 |
| 0x1A | Blue Gain | 49 | 0-100 |
| 0x60 | Input Source | DP-1(15) | DP1/DP2/HDMI1/HDMI2 |
| 0x62 | Volume | 57 | 0-100 (triggers OSD) |
| 0x87 | Sharpness | 50 | 0-100 |
| 0x8D | Audio Mute | Off(2) | 1=mute,2=unmute |
| 0xCA | OSD Lock | Off(2) | 1=lock,2=unlock |
| 0xCC | Language | FR(2) | 0=EN,2=FR,3=DE... |
| 0xD6 | Power | On(1) | 1=on,4=off |
| 0xF5 | Aspect Ratio | 1 | 1=Wide,2=Original,3=Cinema1,4=Cinema2 |
| 0xF6 | Smart Energy | Low(1) | 0=Off,1=Low,2=High |
| 0xF7 | Response Time | Middle(2) | 0=Off,1=Fast,2=Normal,3=Slow |
| 0xF8 | FreeSync | On(1) | 0=Off,1=On |
| 0xF9 | Black Stabilizer | 50 | 0-100 |
| 0xFD | Power LED | Off(0) | 0=Off,1=On |
| 0xFE | Gamma | Mode2(3) | 0=Off,1=Mode0,2=Mode1 |

### LG OnScreen Control SDK Analysis
- App: .NET WPF (decompilable with dnSpy/ILSpy)
- DLL: LGMonitorDDCCISDK.dll (in System32)
- Functions: Get/SetPropertyWithoutOpcodeVerification
- Protocol: Standard DDC/CI VCP Set/Get (no secret commands)
- No OSD trigger command exists in SDK
