#!/usr/bin/env python3
"""Measures the idle delay before an Apple BT keyboard goes to sleep (READ ONLY).

Opens /dev/hidrawN with O_RDONLY and keeps ONLY the timestamp of the input
reports (never their content: no keylogging). When the node disappears
(keyboard asleep/disconnected), writes a JSON line:
  connect, last_input, disconnect, idle_s = disconnect - last_input.
No request is sent to the keyboard (neither GET nor SET).

Used to test the hypothesis "0xF5 = 0x0384 = 900 s = sleep delay".

  idle_timeout.py --out idle.jsonl --duration 3600
"""

import argparse
import glob
import json
import os
import select
import sys
import time


def find_hidraw(mac):
    mac = mac.lower()
    for h in sorted(glob.glob("/sys/class/hidraw/hidraw*")):
        try:
            with open(os.path.join(h, "device", "uevent")) as f:
                if f"hid_uniq={mac}" in f.read().lower():
                    return "/dev/" + os.path.basename(h)
        except OSError:
            pass
    return None


def iso(t):
    return time.strftime("%Y-%m-%dT%H:%M:%S%z", time.localtime(t)) if t else None


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mac", default="AA:BB:CC:DD:EE:F1")
    ap.add_argument("--duration", type=int, default=3600)
    ap.add_argument("--out")
    a = ap.parse_args()
    out = open(a.out, "a") if a.out else sys.stdout
    end = time.time() + a.duration
    while time.time() < end:
        dev = find_hidraw(a.mac)
        if not dev:
            time.sleep(5)
            continue
        try:
            fd = os.open(dev, os.O_RDONLY | os.O_NONBLOCK)
        except OSError:
            time.sleep(5)
            continue
        connect, last, n = time.time(), None, 0
        try:
            while time.time() < end:
                r, _, _ = select.select([fd], [], [], 5)
                if r:
                    try:
                        data = os.read(fd, 64)
                    except BlockingIOError:
                        continue
                    if not data:
                        break
                    del data  # content never kept
                    last, n = time.time(), n + 1
                elif not os.path.exists(dev) or find_hidraw(a.mac) != dev:
                    break
        except OSError:
            pass
        finally:
            os.close(fd)
        gone = time.time()
        rec = {"connect": iso(connect), "last_input": iso(last), "disconnect": iso(gone),
               "reports": n, "idle_s": round(gone - last, 1) if last else None,
               "connected_s": round(gone - connect, 1),
               "ended_by": "duration" if gone >= end else "device_gone"}
        out.write(json.dumps(rec) + "\n")
        out.flush()


if __name__ == "__main__":
    main()
