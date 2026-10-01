#!/usr/bin/env python3
"""Mesure du délai d'inactivité avant mise en veille d'un clavier Apple BT (LECTURE SEULE).

Ouvre /dev/hidrawN en O_RDONLY et ne garde QUE l'horodatage des rapports
d'entrée (jamais leur contenu : pas de keylogging). Quand le nœud disparaît
(clavier endormi/déconnecté), écrit une ligne JSON :
  connect, last_input, disconnect, idle_s = disconnect - last_input.
Aucune requête n'est envoyée au clavier (ni GET ni SET).

Sert à tester l'hypothèse « 0xF5 = 0x0384 = 900 s = délai de veille ».

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
                    del data  # contenu jamais conservé
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
