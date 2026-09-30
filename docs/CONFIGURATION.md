# Configuration

File: `~/.config/apple-kb-monitor/config.toml`, fallback `/etc/apple-kb-monitor/config.toml` (template: `config.toml.example`). Read by `ddc.rs` (bus), `main.rs` (MQTT/brightness) and `mqtt-bridge.py`.

| Section | Key | Default | Meaning |
|---|---|---|---|
| `[ddc]` | `bus` | `/dev/i2c-6` | I2C bus. Priority: config > auto-detection (probes `/dev/i2c-*` for VCP 0xDF) > fallback `/dev/i2c-6` |
| `[mqtt]` | `broker` | empty | Broker address; empty disables MQTT |
| `[mqtt]` | `port` | 1883 | |
| `[mqtt]` | `user`, `password` | empty | Stored in clear text: keep the file `chmod 600` |
| `[mqtt]` | `topic_prefix` | `homeassistant` | Discovery/topic prefix |
| `[monitor]` | `model` | `lg_34gn850` | Slug used in MQTT topic paths |
| `[brightness]` | `min` / `max` | 2 / 70 | DDC brightness bounds (%) used for lamp sync and MQTT |
| `[brightness]` | `lamp_entity` | `light.bureau` | Home Assistant entity synchronised with brightness |

Notes:
- The parser is hand-written (no `toml` crate): use `key = value` lines, one per line. Tracked in issue #12.
- Saving from the MQTT tab rewrites the whole file and resets `topic_prefix` to `homeassistant` and `model` to `lg_34gn850` (issue #12).
- Profiles: `~/.config/apple-kb-monitor/profiles.json`; App Presets file next to it; battery history: `~/.local/share/apple-kb-monitor/history.jsonl`.
- The circadian curve (30 % before 06:00, ramp to 70 % at 09:00, 70 % until 17:00, ramp to 30 % at 21:00) is hard-coded (issue #18).
