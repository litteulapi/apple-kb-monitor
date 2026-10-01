#!/usr/bin/env python3
"""Recommended BlueZ / UPower settings for apple-kb-monitor (#142, #143, #146).

Dry run by default: prints the unified diff and changes nothing.
`--apply` (as root, run by the administrator) writes the file after a
timestamped backup next to it. Comments and ordering are preserved; a key that
sits in the WRONG section (e.g. `ReconnectAttempts` under `[AdvMon]`, which
bluetoothd ignores with "Unknown key ... for group AdvMon") is moved.

  bluetooth/akm-conf.py bluez                  # diff for /etc/bluetooth/main.conf
  sudo bluetooth/akm-conf.py bluez --apply     # then: sudo systemctl restart bluetooth
  bluetooth/akm-conf.py upower                 # optional: stop UPower's 30 s HID polling
  sudo bluetooth/akm-conf.py upower --apply    # then: sudo systemctl restart upower

Revert: copy the `.akm-bak-*` backup back over the file and restart the service.
See docs/RECONNEXION-PAIRAGE.md for the reason behind every key.
"""

import argparse
import datetime
import difflib
import os
import re
import shutil
import sys

PROFILES = {
    "bluez": (
        "/etc/bluetooth/main.conf",
        [
            # Interlaced page scan: the host answers the keyboard's page train
            # faster after a key press (the keyboard pages only briefly).
            ("General", "FastConnectable", "true"),
            # Host-side reconnection after a link-loss timeout, HID included.
            ("Policy", "ReconnectUUIDs", "00001124-0000-1000-8000-00805f9b34fb"),
            ("Policy", "ReconnectAttempts", "7"),
            ("Policy", "ReconnectIntervals", "1,2,4,8,16,32,64"),
        ],
    ),
    "upower": (
        "/etc/UPower/UPower.conf",
        [
            # UPower otherwise reads every battery each 30 s; for this HID
            # keyboard each read is two GET_REPORT over the air (#146).
            ("UPower", "NoPollBatteries", "true"),
        ],
    ),
}

SECTION = re.compile(r"^\s*\[([^\]]+)\]\s*$")
KEY = re.compile(r"^\s*([A-Za-z][A-Za-z0-9_]*)\s*=")


def transform(lines, wanted):
    """Return new lines with every (section, key, value) of `wanted` set."""
    out = list(lines)
    for section, key, value in wanted:
        # 1. drop active occurrences of the key outside its section
        cur, keep = None, []
        for ln in out:
            m = SECTION.match(ln)
            if m:
                cur = m.group(1).strip()
            k = KEY.match(ln)
            if k and k.group(1) == key and cur != section:
                continue
            keep.append(ln)
        out = keep
        # 2. set it inside its section (replace, or insert after the header)
        cur, done, header_at = None, False, None
        for i, ln in enumerate(out):
            m = SECTION.match(ln)
            if m:
                cur = m.group(1).strip()
                if cur == section:
                    header_at = i
                continue
            k = KEY.match(ln)
            if cur == section and k and k.group(1) == key:
                if ln.split("=", 1)[1].strip() != value:
                    out[i] = f"{key} = {value}\n"
                done = True
                break
        if not done:
            if header_at is None:
                if out and not out[-1].endswith("\n"):
                    out[-1] += "\n"
                out += ["\n", f"[{section}]\n", f"{key} = {value}\n"]
            else:
                out.insert(header_at + 1, f"{key} = {value}\n")
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("profile", choices=sorted(PROFILES))
    ap.add_argument("--file", help="file to edit (default: the system path)")
    ap.add_argument("--apply", action="store_true", help="write (needs root)")
    a = ap.parse_args()
    path, wanted = PROFILES[a.profile]
    path = a.file or path
    try:
        with open(path, encoding="utf-8") as fh:
            old = fh.readlines()
    except FileNotFoundError:
        old = []
    new = transform(old, wanted)
    diff = list(difflib.unified_diff(old, new, path, path + " (akm)"))
    if not diff:
        print(f"{path}: already compliant, nothing to do")
        return 0
    sys.stdout.writelines(diff)
    if not a.apply:
        print("\n(dry run — rerun with sudo and --apply to write)")
        return 0
    if os.path.exists(path):
        stamp = datetime.datetime.now().strftime("%Y%m%d%H%M%S")
        bak = f"{path}.akm-bak-{stamp}"
        shutil.copy2(path, bak)
        print(f"backup: {bak}")
    tmp = path + ".akm-tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.writelines(new)
    if os.path.exists(path):
        shutil.copymode(path, tmp)
    os.replace(tmp, path)
    print(f"written: {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
