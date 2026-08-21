#!/usr/bin/env python3
"""MQTT ↔ DDC bridge for LG 34GN850 monitor control.

Subscribes to HA command topics, writes DDC via ddc-tool.
Publishes current state back to HA after each write.

Configuration: ~/.config/apple-kb-monitor/config.toml
Fallback:      /etc/apple-kb-monitor/config.toml
"""

import json
import pathlib
import subprocess
import sys
import tomllib

import paho.mqtt.client as mqtt

CONFIG_PATHS = [
    pathlib.Path.home() / ".config" / "apple-kb-monitor" / "config.toml",
    pathlib.Path("/etc/apple-kb-monitor/config.toml"),
]

def _build_topics(cfg: dict) -> tuple[str, str, str]:
    """Build MQTT command/state/discovery topics from config."""
    prefix = cfg.get("mqtt", {}).get("topic_prefix", "homeassistant")
    model = cfg.get("monitor", {}).get("model", "lg_34gn850")
    topic_cmd = f"{prefix}/number/{model}/brightness/set"
    topic_state = f"{prefix}/number/{model}/brightness/state"
    topic_config = f"{prefix}/number/{model}/brightness/config"
    return topic_cmd, topic_state, topic_config


def _build_discovery_payload(cfg: dict, topic_cmd: str, topic_state: str) -> dict:
    """Build the retained HA MQTT discovery payload for the brightness entity.

    Republished on every (re)connect so the entity survives a broker
    restart or a loss of retained messages without manual intervention
    (root cause of the 2026-08 outage: this discovery config was never
    republished after being lost, leaving the entity 'restored'/unavailable
    even though state publishes kept arriving on an unwatched topic).
    """
    bri_cfg = cfg.get("brightness", {})
    model = cfg.get("monitor", {}).get("model", "lg_34gn850")
    return {
        "name": "LG Monitor Brightness",
        "unique_id": "lg_34gn850_brightness_ctrl",
        "command_topic": topic_cmd,
        "state_topic": topic_state,
        "min": bri_cfg.get("min", 2),
        "max": bri_cfg.get("max", 70),
        "step": 1,
        "mode": "auto",
        "icon": "mdi:monitor-shimmer",
        "unit_of_measurement": "%",
        "device": {
            "identifiers": [model],
            "manufacturer": "LG Electronics",
            "model": "34GN850",
            "name": "LG 34GN850",
        },
    }


def load_config() -> dict:
    """Load the first existing TOML config file, or exit with an error."""
    for path in CONFIG_PATHS:
        if path.is_file():
            with open(path, "rb") as fh:
                cfg = tomllib.load(fh)
            print(f"[config] loaded {path}", file=sys.stderr)
            return cfg

    searched = "\n  ".join(str(p) for p in CONFIG_PATHS)
    print(
        f"[config] fatal: no config file found. Searched:\n  {searched}\n"
        f"Copy config.toml.example to one of these paths and fill in credentials.",
        file=sys.stderr,
    )
    sys.exit(1)


def _bus_number(cfg: dict) -> str:
    """Extract the bus number from a /dev/i2c-N path or plain integer."""
    raw = str(cfg.get("ddc", {}).get("bus", "/dev/i2c-6"))
    return raw.rsplit("-", 1)[-1] if raw.startswith("/dev/") else raw


def ddc_write_brightness(bus: str, bri_min: int, bri_max: int, value: int) -> bool:
    value = max(bri_min, min(bri_max, value))
    try:
        r = subprocess.run(
            ["ddc-tool", "write", bus, "0x10", str(value)],
            capture_output=True, text=True, timeout=5,
        )
        if r.returncode == 0:
            print(f"[ddc] brightness → {value}", file=sys.stderr)
            return True
        print(f"[ddc] write error: {r.stderr}", file=sys.stderr)
    except Exception as e:
        print(f"[ddc] exception: {e}", file=sys.stderr)
    return False


def ddc_read_brightness(bus: str) -> int:
    try:
        r = subprocess.run(
            ["ddc-tool", "read", bus, "0x10"],
            capture_output=True, text=True, timeout=5,
        )
        if r.returncode == 0:
            return int(r.stdout.strip().split()[0])
    except Exception:
        pass
    return -1


def on_connect(client, userdata, _flags, rc):
    if rc == 0:
        topic_cmd = userdata["topic_cmd"]
        topic_config = userdata["topic_config"]
        # Re-publish (retained) HA discovery on every reconnect: makes the
        # entity self-healing across broker restarts / retained-message loss.
        client.publish(
            topic_config, json.dumps(userdata["discovery_payload"]), retain=True
        )
        client.subscribe(topic_cmd)
        print(f"[mqtt] connected, discovery published, subscribed to {topic_cmd}", file=sys.stderr)
        bri = ddc_read_brightness(userdata["bus"])
        if bri >= 0:
            client.publish(userdata["topic_state"], str(bri), retain=True)
    else:
        print(f"[mqtt] connect failed: rc={rc}", file=sys.stderr)


def on_message(client, userdata, msg):
    try:
        value = int(float(msg.payload.decode()))
    except (ValueError, UnicodeDecodeError):
        print(f"[mqtt] bad payload: {msg.payload}", file=sys.stderr)
        return

    bri_min = userdata["bri_min"]
    bri_max = userdata["bri_max"]
    bus = userdata["bus"]

    value = max(bri_min, min(bri_max, value))
    if ddc_write_brightness(bus, bri_min, bri_max, value):
        client.publish(userdata["topic_state"], str(value), retain=True)


def main():
    cfg = load_config()

    mqtt_cfg = cfg.get("mqtt", {})
    broker = mqtt_cfg.get("broker", "")
    port = mqtt_cfg.get("port", 1883)
    user = mqtt_cfg.get("user", "")
    password = mqtt_cfg.get("password", "")

    if not broker:
        print("[config] fatal: mqtt.broker is empty", file=sys.stderr)
        sys.exit(1)

    bri_cfg = cfg.get("brightness", {})
    bri_min = bri_cfg.get("min", 2)
    bri_max = bri_cfg.get("max", 70)
    bus = _bus_number(cfg)

    topic_cmd, topic_state, topic_config = _build_topics(cfg)

    userdata = {
        "bus": bus,
        "bri_min": bri_min,
        "bri_max": bri_max,
        "topic_cmd": topic_cmd,
        "topic_state": topic_state,
        "topic_config": topic_config,
        "discovery_payload": _build_discovery_payload(cfg, topic_cmd, topic_state),
    }

    client = mqtt.Client(client_id="lg-ddc-bridge", userdata=userdata)
    if user:
        client.username_pw_set(user, password)
    client.on_connect = on_connect
    client.on_message = on_message
    client.connect(broker, port, 60)
    print("[mqtt-bridge] starting...", file=sys.stderr)
    client.loop_forever()


if __name__ == "__main__":
    main()
