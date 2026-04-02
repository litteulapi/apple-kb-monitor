# LG 34GN850 — Reverse Engineering Notes

## Scaler Chip
- Reports: Myson Century (DDC Controller ID=6), likely Realtek RTD2795/2796
- Firmware: 3.0 (VCP 0xC9 = 768 = 0x0300), VCP Version: 2.1 (VCP 0xDF = 513 = 0x0201)
- Usage: 33909 hours (VCP 0xC0)

## I2C Bus (i2c-6)
| Address | Purpose | Notes |
|---------|---------|-------|
| 0x30 | E-DDC Segment Pointer | Empty (no extended EDID) |
| 0x37 | DDC/CI Scaler | Main VCP channel |
| 0x50 | EDID EEPROM | 256 bytes, also LG sidechannel via header 0x50 |

**I2C Bus Quirk**: NVIDIA adapter has pipeline aliasing — rapid sequential reads return data from the PREVIOUS request. Must double-read (flush + real read) for accuracy. Can't use I2C_RDWR for writes (NVIDIA), must use I2C_SLAVE + libc::write.

## VCP Register Map — 85 VCPs Scanned

### Standard MCCS (20 VCPs, fully decoded)

| VCP | Name | Current | Max | RW | Notes |
|-----|------|---------|-----|-----|-------|
| 0x02 | New Control Value | 2 | 2 | RO | Flag: 2 = no pending changes |
| 0x04 | Factory Reset | 0 | 255 | WO | **DANGEROUS** — write any value to reset all settings |
| 0x05 | Restore Luminance/Contrast | 0 | 1 | WO | Resets brightness + contrast to factory |
| 0x08 | Restore Color | 0 | 255 | WO | Resets color settings, triggers CAUTION OSD |
| 0x10 | Brightness | 65 | 100 | **RW** | Backlight PWM, no OSD |
| 0x12 | Contrast | 51 | 100 | **RW** | |
| 0x14 | Color Preset | 9 | 65535 | **RW** | 5=6500K, 8=9300K, 0xB=User |
| 0x15 | Picture Mode | 45 | 255 | **RW** | 16 modes (see table below) |
| 0x16 | Red Gain | 50 | 100 | **RW** | |
| 0x18 | Green Gain | 50 | 100 | **RW** | |
| 0x1A | Blue Gain | 50 | 100 | **RW** | |
| 0x60 | Input Source | 15 | 18 | **RW** | 0x0F=DP, 0x11=HDMI1, 0x12=HDMI2 |
| 0x62 | Volume | 52 | 100 | **RW** | **Only VCP that triggers native OSD** |
| 0x69 | Color Temp (Kelvin) | 9300 | 65535 | RO | Direct Kelvin readback |
| 0x87 | Sharpness | 50 | 100 | **RW** | |
| 0x8D | Audio Mute | 2 | 100 | **RW** | 1=muted, 2=unmuted |
| 0xCA | OSD Lock | 2 | 2 | RO | 1=locked, 2=unlocked (type=TABLE, read-only via DDC) |
| 0xCC | Language | 2 | 16 | **RW** | 0=EN, 2=FR, 3=DE, 4=ES, 5=IT, 6=KO, 7=ZH, 8=JA, 9=PT |
| 0xD6 | Power Mode | 1 | 5 | **RW** | 1=On, 2=Standby, 3=Suspend, 4=Off(soft), 5=Off(hard) |
| 0xDF | VCP Version | 513 | 255 | RO | 0x0201 = MCCS 2.1 |

### Monitor Info (8 VCPs, read-only)

| VCP | Name | Current | Max | Notes |
|-----|------|---------|-----|-------|
| 0xAC | Horizontal Frequency | 21592 | 3 | kHz × 100 (?) |
| 0xAE | Vertical Frequency | 21820 | 0 | Hz × 100 → ~218.2 Hz (?) |
| 0xB2 | Subpixel Layout | 0 | 65535 | 0 = undefined |
| 0xB6 | Display Technology | 3 | 5 | 3 = IPS |
| 0xC0 | Usage Hours | 33909 | 65535 | Total panel hours |
| 0xC1 | Backlight PWM | 65 | 300 | Wider range than brightness (0-300) |
| 0xC8 | Controller Type | 6 | 255 | 6 = Myson Century |
| 0xC9 | Firmware Version | 768 | 65535 | 0x0300 = 3.0 |

### VGA Legacy (5 VCPs, irrelevant on DisplayPort)

| VCP | Name | Current | Max | Notes |
|-----|------|---------|-----|-------|
| 0x0E | Image Lock Coarse | 50 | 100 | Pixel clock (VGA only) |
| 0x1E | Auto Setup | 0 | 2 | Auto-adjust (VGA only) |
| 0x20 | Horizontal Position | 0 | 100 | Always 0 on DP |
| 0x30 | Vertical Position | 0 | 100 | Always 0 on DP |
| 0x3E | Clock Phase | 50 | 100 | VGA fine tune |

### Video Black Level (3 VCPs)

| VCP | Name | Current | Max | Notes |
|-----|------|---------|-----|-------|
| 0x6C | Black Level Red | 128 | 100 | Mid-point, cur > max is normal (scaler uses 0-255 internal) |
| 0x6E | Black Level Green | 128 | 100 | |
| 0x70 | Black Level Blue | 128 | 100 | |

### LG Vendor — Decoded (10 VCPs)

| VCP | Name | Current | Max | RW | Notes |
|-----|------|---------|-----|-----|-------|
| 0xF5 | Aspect Ratio | 1 | 255 | **RW** | 0=Full Wide, 1=Original, 2=Just Scan, 3=Cinema1 |
| 0xF6 | Smart Energy Saving | 0 | 255 | **RW** | 0=Off, 1=Low, 2=High |
| 0xF7 | Response Time | 1 | 255 | **RW** | 0=Off, 1=Fast, 2=Normal, 3=Slow, 4=Faster |
| 0xF8 | FreeSync | 1 | 255 | **RW** | 0=Off, 1=Basic, 2=Extended |
| 0xF9 | Black Stabilizer | 25 | 255 | **RW** | 0-100 (UI range) |
| 0xFE | Gamma | 3 | 255 | **RO** | 0=2.2, 1=2.4, 2=2.0, 3=1.8. **Read-only: writes accepted but ignored by firmware** |
| 0xFD | Power LED | 0 | 65535 | RO | 0=Off, 1=On (type=TABLE, read-only via DDC) |
| 0xD7 | Split / PBP | 1 | 255 | RO | 0=Off, 1=PBP (type=TABLE, read-only via DDC) |
| 0xDE | Scratch Pad | 0 | 65535 | ?? | Brightness mirror (undocumented purpose) |
| 0x52 | Active Control | 0 | 255 | RO | Flags for pending setting changes |

### Mirror Bank 1 (0xE8-0xEF) — Stale Copies

These registers mirror standard VCPs but values are **stale/delayed** — not real-time copies. Likely snapshot at boot or last OSD interaction.

| VCP | Mirrors | Notes |
|-----|---------|-------|
| 0xE8 | Brightness (0x10) | Often reads 0 instead of current value |
| 0xE9 | Contrast (0x12) | Reads stale value |
| 0xEA | Color Preset (0x14) | |
| 0xEB | Picture Mode (0x15) | Max=1 (boolean?) instead of 255 |
| 0xEC | Red Gain (0x16) | Unreliable |
| 0xED | Green Gain (0x18) | Unreliable |
| 0xEE | Blue Gain (0x1A) | Unreliable |

### Mirror Bank 2 (0xE3-0xF3, even VCPs) — Stale Copies

Second set of mirrors, same stale behavior.

| VCP | Mirrors | Notes |
|-----|---------|-------|
| 0xE3 | Brightness | |
| 0xE4 | Contrast | |
| 0xE5 | Color Preset | max=179 (unusual) |
| 0xE6 | Picture Mode | |
| 0xE7 | Red Gain | |
| 0xF0 | Green Gain | |
| 0xF1 | Blue Gain | |
| 0xF3 | Input Source | |
| 0xFB | Volume | max=15 (not 100!) |
| 0xFC | Color Temp Kelvin | Second copy |

### Newly Decoded (session 2025-04-02)

| VCP | Current | Decoded Name | Evidence |
|-----|---------|-------------|----------|
| 0x72 | 30720 | **MCCS Gamma (WRITABLE)** | High-byte = (gamma-1.0)×100. 0x7800=2.2. Write confirmed. |
| 0x4D | 32770 | Capability Flags | 0x8002, static bit field, hardware capability register |
| 0x4E | 0 | Status Register | Always 0 = normal operation |
| 0x4F | 7042 | Panel Identifier | 0x1B82, static, panel family + revision code |
| 0xAF | volatile | MCU Tick Counter | Increments ~768/read (~268/sec), 16-bit heartbeat, wraps every ~244s |
| 0xCF | 526 | DDC/CI Sub-version | 0x020E = LG DDC implementation v2.14 |
| 0xEF | 22624 | Panel Timing Metadata | 0x5860, static factory calibration constant |
| 0xFA | 255 | Color Gamut | 255 = Wide/Native, locked by picture mode |
| 0xFF | 0 | Vendor Command Register | 0 = idle, factory command/status register |
| 0xC6 | 104 | Feature Identifier | **Read-only**, NOT an auth key. LG OnScreen reads it to fingerprint model capabilities |

### Still Unknown (11 VCPs)

| VCP | Current | Max | Type | Notes |
|-----|---------|-----|------|-------|
| 0x0B | 0 | 24028 | TABLE | MCCS "Color Temp Increment" — value 24028 unexplained |
| 0x0C | 63 | 100 | TABLE | MCCS "Color Temp Request" |
| 0x50 | 2 | 15 | TABLE | Unknown — "bottom_corner" in some MCCS implementations |
| 0x55 | 1 | 1 | TABLE | Unknown toggle — always 1 |
| 0x6A | 271 | 65535 | TABLE | MCCS "Color Temp Increment" (alternate) |
| 0x7A | 240 | 65535 | TABLE | MCCS "Adjust Focal Plane" |
| 0xD8 | 1 | 255 | TABLE | MCCS "Display Mode" or LG vendor |
| 0xDD | 0 | 65535 | TABLE | Unknown |
| 0xE0 | 0 | 65535 | TABLE | LG vendor — purpose unknown |
| 0xE1 | 0 | 65535 | TABLE | LG vendor — purpose unknown |
| 0xE2 | 1 | 65535 | TABLE | LG vendor — purpose unknown |

### VCP 0x72: MCCS Standard Gamma — DECODED AND WRITABLE

Value: 30720 = 0x7800. Encoding: **high byte = (gamma - 1.0) × 100**.

| Value | High Byte | Gamma |
|-------|-----------|-------|
| 0x5000 (20480) | 0x50 (80) | 1.8 |
| 0x6400 (25600) | 0x64 (100) | 2.0 |
| 0x7800 (30720) | 0x78 (120) | **2.2** (current) |
| 0x8C00 (35840) | 0x8C (140) | 2.4 |

**WRITABLE** — unlike 0xFE which is read-only, VCP 0x72 accepts writes and the gamma actually changes on the panel. This bypasses the picture mode gamma lock on 0xFE.

## Picture Modes (VCP 0x15) — 16 modes from capabilities

| Value | Mode | Gamma locked? |
|-------|------|--------------|
| 0x01 (1) | Reader | Yes |
| 0x06 (6) | Color Weakness / Gamer 2 | Yes |
| 0x11 (17) | Custom | Configurable via OSD only |
| 0x13 (19) | RTS | Yes |
| 0x14 (20) | Vivid | Yes |
| 0x15 (21) | sRGB | Yes (2.2) |
| 0x18 (24) | sRGB/SMPTE-C | Yes |
| 0x19 (25) | EBU | Yes |
| 0x20 (32) | Photo | Yes |
| 0x22 (34) | ? | Unknown |
| 0x23 (35) | ? | Unknown |
| 0x24 (36) | ? | Unknown |
| 0x28 (40) | FPS Game 1 | Yes |
| 0x29 (41) | FPS Game 2 | Yes |
| 0x32 (50) | ? | Unknown |
| 0x48 (72) | Cinema | Yes |

**Note**: Current picture mode is 45 (0x2D) which is NOT in the capabilities string. The monitor accepts it, suggesting undocumented modes exist.

## OSD Control

| VCP | Function | Triggers OSD? |
|-----|----------|:------------:|
| 0x62 | Volume | **YES** — ONLY VCP that triggers native OSD slider |
| 0x08 | Restore Color | YES — triggers CAUTION message |
| 0x10 | Brightness | NO — changes backlight silently |
| All others | — | NO |

Volume OSD trick: writing the SAME value to 0x62 triggers the OSD without changing audio.

## RAM Map (via sidechannel VCP 0xD1)

### Mapped Regions
| Region | Size | Purpose | Status |
|--------|------|---------|--------|
| 0x0F00-0x0FFF | 256B | OSD/Settings (dynamic) | **Partially decoded** |
| 0x1020-0x11BF | 416B | VCP response cache (shadow) | **Decoded** — ring buffer |
| 0x6660-0x67BF | 352B | VCP response cache (shadow) | **Decoded** — ring buffer |
| 0x82D0-0x846F | 416B | VCP response cache (shadow) | **Decoded** — ring buffer |
| 0xBC10-0xBDAF | 416B | VCP response cache (shadow) | **Decoded** — ring buffer |
| 0xF5B0-0xF6BF | 272B | VCP response cache (shadow) | **Decoded** — ring buffer |

All 5 "unknown" regions contain the same data structure: **multi-buffered shadow copies of the scaler's VCP register file**. These are FIFO/ring buffers used by the DDC/CI response generation engine. Values are volatile (shift offsets between reads) and every non-zero value maps to a known VCP setting (brightness=65, contrast=51, h_freq=21592, etc.).

### OSD Region (0x0F00-0x0FFF) Diff Results
| Action | Bytes Changed | OSD Displayed |
|--------|--------------|---------------|
| Brightness 44→80 | 23 | NO |
| Volume 50→80 | 11 | YES |
| Language FR→EN | 38 | NO |
| Picture Mode 45→1 | 31 | NO |

### Volume-Exclusive RAM Changes (OSD trigger analysis)
- 0x0F1E: 88→04 (flag set when OSD displays)
- 0x0F1F: 88→04 (flag set when OSD displays)
- These are OUTPUT flags (consequence of OSD), not INPUT triggers

### Conclusion
VCP dispatch table is in SPI flash firmware, not in RAM. Cannot patch OSD behavior without physical access to SPI flash.

## Sidechannel Commands (DDC2AB, header byte 0x50)

| VCP | Function | Tested | Result |
|-----|----------|--------|--------|
| 0xC9 | Firmware version | Yes | Returns raw bytes |
| 0xCA | Model string | Yes | Returns "34GN850" |
| 0xCC | OSD Language | Yes | 0=EN, 2=FR, 3=DE (writable!) |
| 0xD1 | RAM read | Yes | Works — full scaler RAM access |
| 0xD5 | RAM write | Yes | Accepted but no visible effect on OSD |
| 0xF4 | Input switch (LG) | **NOT TESTED** | Risk of locking input |

## Service Menu

### Physical Access (NOT tested, PROCEED WITH CAUTION)
The LG 34GN850 has a factory service menu accessible via the Realtek RTD2795 scaler:

1. **Method 1**: Turn off monitor → Press and hold joystick button (click inward) → Power on while holding → Release after 5 seconds
2. **Method 2**: On some LG models, rapidly press joystick: Menu, Menu, Menu, Mute pattern
3. **Method 3**: LG Calibration Studio / True Color Pro software (Windows, uses DDC)
4. **Method 4**: InStart (ISP) tool used by LG service centers (requires specific cable)

### Service Menu Contents (from documentation of similar Realtek scaler models)
- Panel hours / usage counter
- ADC calibration / white balance
- Factory reset (deeper than VCP 0x04)
- Test patterns (color bars, gradient, pixel test)
- EEPROM dump / raw register access
- Model / serial number
- Gamma curve per picture mode
- Panel voltage settings
- Backlight current limits

### DDC Access to Service Features
- **VCP 0x04**: Factory reset (tested on other VCPs, works but resets ALL settings)
- **VCP 0xC6**: Application Enable Key (value=104) — used by LG OnScreen Control for authentication. Could unlock additional features.
- **VCP 0x4D-0x4F**: LG internal registers (0x8002, 0, 0x1B82) — purpose unknown, possibly service-related
- **No known DDC command enters the full service menu** — the OSD state machine is in SPI flash firmware

## LG OnScreen Control SDK

- App: .NET WPF (decompilable with dnSpy/ILSpy)
- DLL: LGMonitorDDCCISDK.dll (in System32)
- Functions: Get/SetPropertyWithoutOpcodeVerification
- Protocol: Standard DDC/CI VCP Set/Get (no secret commands)
- No OSD trigger command exists in SDK
- Uses VCP 0xC6 for application key authentication

## Confirmed Impossibilities

| Feature | Status | Why |
|---------|--------|-----|
| Brightness OSD via DDC | **IMPOSSIBLE** | OSD trigger is firmware-only (SPI flash) |
| Gamma change via DDC | **SOLVED** | VCP 0xFE is read-only, but **VCP 0x72 works** (MCCS standard encoding) |
| Service menu via DDC | **NOT FOUND** | Would require undocumented VCP sequence |
| RAM patching for OSD | **IMPOSSIBLE** | VCP dispatch table in SPI flash, not RAM |

## Summary

- **85 VCPs scanned** across standard MCCS, LG vendor, and mirror banks
- **43 fully decoded** (20 standard + 10 LG vendor + 3 black level + 10 newly decoded)
- **16 mirror registers** in 2 banks (stale data, not real-time)
- **5 VGA legacy** (dead on DisplayPort)
- **11 still unknown** (0x0B, 0x0C, 0x50, 0x55, 0x6A, 0x7A, 0xD8, 0xDD, 0xE0-E2) — all TABLE type, read-only, likely MCCS informational
- **5 RAM regions decoded** — all VCP response cache ring buffers, no distinct data
- **Service menu**: physical access documented, NOT accessible via DDC
- **Key discovery**: VCP 0x72 (MCCS Gamma) is **writable** with high-byte encoding, bypasses 0xFE picture mode lock
