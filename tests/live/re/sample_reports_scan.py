#!/usr/bin/env python3
"""READ-ONLY sampling of the Feature reports of an Apple BT keyboard (A1314).

Only GET_REPORT (HIDIOCGFEATURE ioctl): no SET_REPORT/SET_FEATURE,
no write to the keyboard. The device is opened with O_RDONLY.

Each sample = one JSON line (JSONL):
  - ts, reports {id_hex: {len, hex}} (length actually returned by the ioctl),
  - kernel battery (power_supply capacity/status), BlueZ Battery1.Percentage,
  - provisional decodings (voltage, thresholds) to ease correlation.

Report 0x4C (identity key, SENSITIVE) is NEVER written in clear:
only its length, its first two bytes and the paired host address are
recorded; a truncated SHA-256 fingerprint (8 B) only with
AKM_RE_FINGERPRINT=1 — enough to detect a change, not to rebuild it.

0x4C and 0xFE (froze the firmware, right after 0xF7) are never read
(docs/FEATURES.md), --scan-once included, unless --allow-never-read is given.

Safeguards: minimum interval 60 s (5 min recommended); read only if
`bluetoothctl info` shows "Connected: yes"; final stop (code 2) at the
first absent keyboard or the first I/O error, without retry.

Examples:
  sample_reports_scan.py --once
  sample_reports_scan.py --interval 300 --count 13 --out samples.jsonl
  sample_reports_scan.py --scan-once          # sweeps 0x00-0xFF once (GET only)
"""

import argparse
import fcntl
import glob
import hashlib
import json
import os
import subprocess
import sys
import time

MIN_INTERVAL = 60
DEFAULT_IDS = [0x09, 0x46, 0x47, 0x49, 0x4A, 0x4B, 0x4F, 0x51, 0x52, 0x53,
               0x54, 0x5A, 0x5B, 0x5C, 0x5D, 0x60, 0xD1, 0xD8, 0xEA, 0xEB, 0xF4,
               0xF5, 0xF6, 0xF7, 0xFF]
NEVER_READ = {0x4C, 0xFE}
SENSITIVE = {0x4C}
BUF = 64


def ids_for(scan_once, allow_never_read):
    ids = list(range(256)) if scan_once else DEFAULT_IDS
    if allow_never_read:
        return sorted(set(ids) | NEVER_READ)
    return [i for i in ids if i not in NEVER_READ]


def HIDIOCGFEATURE(n):
    return 0xC0004807 | (n << 16)


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


def get_feature(fd, rid):
    buf = bytearray(BUF)
    buf[0] = rid
    try:
        n = fcntl.ioctl(fd, HIDIOCGFEATURE(BUF), buf)
    except OSError as e:
        return {"err": e.errno}
    return {"len": n, "raw": bytes(buf[:n])}


def mask(rid, r):
    if "err" in r:
        return r
    raw = r["raw"]
    if rid in SENSITIVE:
        # 0x4C = type (1 B) + BD_ADDR of the paired host (6 B, little-endian)
        # + 12 secret bytes. Only the host address (public) is kept.
        host = ":".join(f"{x:02x}" for x in raw[2:8][::-1]) if len(raw) >= 8 else None
        out = {"len": r["len"], "first": raw[:2].hex(), "bonded_host": host, "masked": True}
        if os.environ.get("AKM_RE_FINGERPRINT"):  # opt-in, never committed
            out["sha256_8"] = hashlib.sha256(raw).hexdigest()[:16]
        return out
    return {"len": r["len"], "hex": raw.hex()}


def be16(b, o):
    return (b[o] << 8) | b[o + 1]


def decode(reps):
    """Provisional decodings (see docs/HARDWARE-HID-REPORTS.md)."""
    out = {}
    def raw(rid):
        v = reps.get(f"{rid:02x}")
        return bytes.fromhex(v["hex"]) if v and "hex" in v else None
    b = raw(0x47)
    if b and len(b) >= 2:
        out["pct_0x47"] = b[1]
    b = raw(0xEA)
    if b and len(b) >= 2:
        out["pct_0xEA"] = b[1]
    b = raw(0x46)
    if b and len(b) >= 3:
        out["mv_0x46_le"] = b[1] | (b[2] << 8)
    b = raw(0x49)
    if b and len(b) >= 3:
        out["mv_0x49_le"] = b[1] | (b[2] << 8)
    b = raw(0xFF)
    if b and len(b) >= 4:
        out["mv_0xFF_be"] = be16(b, 1)
        out["flag_0xFF"] = b[3]
    b = raw(0xF5)
    if b and len(b) >= 3:
        out["f5_be16"] = be16(b, 1)
    b = raw(0xF4)
    if b and len(b) >= 3:
        out["f4_be16"] = be16(b, 1)
    b = raw(0x5A)
    if b and len(b) >= 9:
        out["thr_0x5A"] = [be16(b, 1 + 2 * i) for i in range(4)]
    return out


def host_battery(mac):
    res = {}
    for ps in glob.glob(f"/sys/class/power_supply/hid-{mac.lower()}-battery*"):
        for k in ("capacity", "status"):
            try:
                with open(os.path.join(ps, k)) as f:
                    res[f"ps_{k}"] = f.read().strip()
            except OSError:
                pass
    path = "/org/bluez/hci0/dev_" + mac.upper().replace(":", "_")
    try:
        o = subprocess.run(["busctl", "get-property", "org.bluez", path,
                            "org.bluez.Battery1", "Percentage"],
                           capture_output=True, text=True, timeout=5).stdout.split()
        if len(o) == 2:
            res["bluez_pct"] = int(o[1])
    except (OSError, subprocess.SubprocessError, ValueError):
        pass
    return res


def bluez_connected(mac):
    try:
        o = subprocess.run(["bluetoothctl", "info", mac], capture_output=True,
                           text=True, timeout=10).stdout
    except (OSError, subprocess.SubprocessError):
        return False
    return "\tConnected: yes" in o


def sample(dev, mac, ids):
    fd = os.open(dev, os.O_RDONLY)
    try:
        reps = {}
        for rid in ids:
            reps[f"{rid:02x}"] = mask(rid, get_feature(fd, rid))
    finally:
        os.close(fd)
    return {"ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "epoch": int(time.time()),
            "dev": dev, "reports": reps, "host": host_battery(mac),
            "decoded": decode(reps)}


def analyze(path):
    """Summary of a JSONL: distinct values per report and decoded ranges."""
    allrows = [json.loads(l) for l in open(path) if l.strip()]
    rows = [r for r in allrows if "reports" in r]
    absent = len(allrows) - len(rows)
    if not rows:
        sys.exit(f"no usable sample ({absent} absent)")
    print(f"{len(rows)} samples (+{absent} keyboard absent), "
          f"{rows[0]['ts']} -> {rows[-1]['ts']}")
    ids = sorted({k for r in rows for k in r["reports"]})
    for k in ids:
        vals = [r["reports"].get(k, {}) for r in rows]
        keyed = [v.get("hex") or v.get("sha256_8") or f"err{v.get('err')}" for v in vals]
        uniq = list(dict.fromkeys(keyed))
        tag = "CONSTANT" if len(uniq) == 1 else f"VARIES ({len(uniq)} values)"
        print(f"  0x{k.upper()} {tag}: {', '.join(uniq[:6])}{' …' if len(uniq) > 6 else ''}")
    keys = sorted({k for r in rows for k in r.get("decoded", {})})
    for k in keys:
        v = [r["decoded"][k] for r in rows if k in r.get("decoded", {})]
        if v and isinstance(v[0], int):
            print(f"  {k}: min {min(v)} max {max(v)} first {v[0]} last {v[-1]}")
    hp = [r["host"].get("ps_capacity") for r in rows]
    print(f"  kernel capacity: {list(dict.fromkeys(hp))}")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mac", default="AA:BB:CC:DD:EE:F1")
    ap.add_argument("--dev", help="/dev/hidrawN (otherwise found by HID_UNIQ)")
    ap.add_argument("--once", action="store_true")
    ap.add_argument("--scan-once", action="store_true",
                    help="GET_REPORT on the 256 IDs but 0x4C and 0xFE, once only")
    ap.add_argument("--allow-never-read", action="store_true",
                    help="also read 0x4C and 0xFE (0xFE can freeze the firmware)")
    ap.add_argument("--interval", type=int, default=300)
    ap.add_argument("--count", type=int, default=13)
    ap.add_argument("--out", help="JSONL file (append); default stdout")
    ap.add_argument("--analyze", metavar="JSONL",
                    help="summarize a sample file (no hardware access)")
    a = ap.parse_args()

    if a.analyze:
        analyze(a.analyze)
        return

    if a.interval < MIN_INTERVAL:
        sys.exit(f"interval < {MIN_INTERVAL} s refused (radio safeguard)")

    ids = ids_for(a.scan_once, a.allow_never_read)
    n = 1 if (a.once or a.scan_once) else a.count
    out = open(a.out, "a") if a.out else sys.stdout
    for i in range(n):
        # Reads ONLY if BlueZ sees the keyboard connected; otherwise logs
        # the absence and stops (never a retry loop on a dead link).
        dev = a.dev or find_hidraw(a.mac)
        s = None
        if dev and bluez_connected(a.mac):
            try:
                s = sample(dev, a.mac, ids)
            except OSError as e:
                s = {"error": e.errno}
        if s is None or "error" in s:
            s = {"ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "epoch": int(time.time()),
                 "absent": True, "host": host_battery(a.mac), **(s or {})}
        elif a.scan_once:
            s["reports"] = {k: v for k, v in s["reports"].items() if "err" not in v}
        out.write(json.dumps(s, sort_keys=True) + "\n")
        out.flush()
        io_err = s.get("absent") or (not a.scan_once and any(
            "err" in v for v in s["reports"].values()))
        if io_err:
            print("stop: keyboard absent or I/O error (no retry)", file=sys.stderr)
            sys.exit(2)
        if i + 1 < n:
            time.sleep(a.interval)


if __name__ == "__main__":
    main()
