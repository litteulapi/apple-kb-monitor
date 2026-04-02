#!/usr/bin/env python3
"""MQTT ↔ DDC bridge for LG 34GN850 monitor control.

Subscribes to HA command topics, writes DDC via ddc-tool.
Publishes current state back to HA after each write.
"""

import subprocess
import sys
import paho.mqtt.client as mqtt

BROKER = "192.168.8.3"
PORT = 1883
MQTT_USER = "adminapi"
MQTT_PASS = "12b27g02"
TOPIC_CMD = "homeassistant/number/lg_34gn850/brightness/set"
TOPIC_STATE = "homeassistant/number/lg_34gn850/brightness/state"


def ddc_write_brightness(value: int) -> bool:
    value = max(2, min(70, value))
    try:
        r = subprocess.run(
            ["ddc-tool", "write", "6", "0x10", str(value)],
            capture_output=True, text=True, timeout=5,
        )
        if r.returncode == 0:
            print(f"[ddc] brightness → {value}", file=sys.stderr)
            return True
        print(f"[ddc] write error: {r.stderr}", file=sys.stderr)
    except Exception as e:
        print(f"[ddc] exception: {e}", file=sys.stderr)
    return False


def ddc_read_brightness() -> int:
    try:
        r = subprocess.run(
            ["ddc-tool", "read", "6", "0x10"],
            capture_output=True, text=True, timeout=5,
        )
        if r.returncode == 0:
            return int(r.stdout.strip().split()[0])
    except Exception:
        pass
    return -1


def on_connect(client, _userdata, _flags, rc):
    if rc == 0:
        client.subscribe(TOPIC_CMD)
        print(f"[mqtt] connected, subscribed to {TOPIC_CMD}", file=sys.stderr)
        bri = ddc_read_brightness()
        if bri >= 0:
            client.publish(TOPIC_STATE, str(bri), retain=True)
    else:
        print(f"[mqtt] connect failed: rc={rc}", file=sys.stderr)


def on_message(client, _userdata, msg):
    try:
        value = int(float(msg.payload.decode()))
    except (ValueError, UnicodeDecodeError):
        print(f"[mqtt] bad payload: {msg.payload}", file=sys.stderr)
        return

    value = max(2, min(70, value))
    if ddc_write_brightness(value):
        client.publish(TOPIC_STATE, str(value), retain=True)


def main():
    client = mqtt.Client(client_id="lg-ddc-bridge")
    client.username_pw_set(MQTT_USER, MQTT_PASS)
    client.on_connect = on_connect
    client.on_message = on_message
    client.connect(BROKER, PORT, 60)
    print("[mqtt-bridge] starting...", file=sys.stderr)
    client.loop_forever()


if __name__ == "__main__":
    main()
