#!/usr/bin/env python3
"""A1314 (BCM2042) battery time series — READ ONLY (BATTERY-CHECK, 2026-10-01).

Each burst (at most one every --period s, 300 s by default), light mode:
  1. reads the UPower percentage;
  2. HIDIOCGFEATURE (GET_REPORT Feature) on the 4 battery reports 0x46, 0x49, 0x47, 0xEA,
     timing each read (ms);
  3. rereads the UPower percentage.
No SET_REPORT, no write: the node is opened with O_RDONLY.
Immediate stop at the first I/O error (EIO, ENODEV, ETIMEDOUT...) or if the node disappears.

usage: verif_battery_series.py AA:BB:CC:DD:EE:F1|/dev/hidraw7 --rounds 40 --out s.jsonl
       verif_battery_series.py --analyze s.jsonl
"""
import argparse, errno, fcntl, json, os, sys, time

MAC = ""
# Light mode enforced after the drop-out of 01/10 12:13: 4 reports, never 0xFE nor a sweep.
IDS = [0x46, 0x49, 0x47, 0xEA]


def bt_connected(mac):
    """BlueZ state (bluetoothd cache, no radio request)."""
    import subprocess
    try:
        out = subprocess.run(["bluetoothctl", "info", mac], capture_output=True, text=True, timeout=10).stdout
    except Exception:
        return False
    return "\tConnected: yes" in out


def upower_pct(mac):
    """Percentage held by UPower (already polled by the system): no extra request."""
    import subprocess
    try:
        out = subprocess.run(["upower", "-d"], capture_output=True, text=True, timeout=10).stdout
    except Exception:
        return None
    blk = [b for b in out.split("\n\n") if mac.lower().replace(":", "_") in b.lower() or mac.lower() in b.lower()]
    for line in (blk[0].splitlines() if blk else []):
        if "percentage:" in line:
            return line.split(":", 1)[1].strip()
    return None


def hidiocgfeature(length):
    return (3 << 30) | (length << 16) | (ord('H') << 8) | 0x07


def get_feature(fd, rid, size=64):
    buf = bytearray(size)
    buf[0] = rid
    t = time.monotonic()
    n = fcntl.ioctl(fd, hidiocgfeature(size), buf, True)
    return bytes(buf[:n]), round((time.monotonic() - t) * 1000, 1)


def find_hidraw(mac):
    """hidraw node whose HID_UNIQ = mac (the number changes from one reconnection to the next)."""
    import glob
    for ue in glob.glob("/sys/class/hidraw/hidraw*/device/uevent"):
        try:
            if ("hid_uniq=" + mac.lower()) in open(ue).read().lower():
                return "/dev/" + ue.split("/")[4]
        except OSError:
            pass
    return None


def one_round(dev, gap):
    rec = {"t": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "epoch": round(time.time(), 1)}
    rec["upower_before"] = upower_pct(MAC)
    fd = os.open(dev, os.O_RDONLY | os.O_CLOEXEC)
    try:
        rep, lat = {}, {}
        for rid in IDS:
            data, ms = get_feature(fd, rid)
            rep["%02x" % rid] = data.hex()
            lat["%02x" % rid] = ms
            time.sleep(gap)
    finally:
        os.close(fd)
    rec["rep"], rec["lat_ms"] = rep, lat
    rec["upower_after"] = upower_pct(MAC)
    return rec


def decode(rep):
    b = {k: bytes.fromhex(v) for k, v in rep.items()}
    d = {}
    if len(b.get("46", b"")) >= 3: d["v46"] = int.from_bytes(b["46"][1:3], "little")
    if len(b.get("ff", b"")) >= 4: d["vff"] = int.from_bytes(b["ff"][1:3], "big"); d["ff3"] = b["ff"][3]
    if len(b.get("49", b"")) >= 3: d["v49"] = int.from_bytes(b["49"][1:3], "little")
    if len(b.get("47", b"")) >= 2: d["p47"] = b["47"][1]
    if len(b.get("ea", b"")) >= 2: d["pea"] = b["ea"][1]
    return d


def analyze(path):
    rows = [json.loads(l) for l in open(path) if l.strip()]
    rows = [r for r in rows if "rep" in r]
    print("t | v46 vff v49 | p47 pea | upower_before upower_after | ff3")
    for r in rows:
        d = decode(r["rep"])
        print(r["t"][11:19], d.get("v46"), d.get("vff"), d.get("v49"), "|", d.get("p47"), d.get("pea"),
              "|", r.get("upower_before", r.get("cap_before")), r.get("upower_after", r.get("cap_after")), "|", d.get("ff3"))
    # registers that change
    keys = rows[0]["rep"].keys() if rows else []
    for k in keys:
        vals = sorted({r["rep"].get(k) for r in rows})
        if len(vals) > 1:
            print("VARIES %s: %d values %s" % (k, len(vals), vals[:8]))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dev", nargs="?", help="keyboard address or hidraw node")
    ap.add_argument("--rounds", type=int, default=40)
    ap.add_argument("--period", type=float, default=300)
    ap.add_argument("--gap", type=float, default=0.4)
    ap.add_argument("--out")
    ap.add_argument("--analyze")
    ap.add_argument("--wait-node", action="store_true", help="wait passively for the node to appear")
    a = ap.parse_args()
    if a.analyze:
        return analyze(a.analyze)
    if not a.dev or not a.out:
        ap.error("dev and --out are required (or --analyze)")
    if a.period < 300:
        sys.exit("period < 300 s refused (rule: at most one full read per 5 min)")
    # Passive wait for the node (no I/O to the keyboard): the keyboard sleeps after inactivity,
    # we only read once it has reconnected by itself (key press).
    mac = a.dev if ":" in a.dev else None
    resolve = (lambda: find_hidraw(mac)) if mac else (lambda: a.dev if os.path.exists(a.dev) else None)
    while a.wait_node and not (resolve() and (not mac or bt_connected(mac))):
        time.sleep(10)
    # Cadence kept between two runs: last burst of the file is >= period old.
    if os.path.exists(a.out):
        last = [json.loads(l) for l in open(a.out) if l.strip()]
        last = [r["epoch"] for r in last if "epoch" in r]
        if last and time.time() - last[-1] < a.period:
            time.sleep(a.period - (time.time() - last[-1]))
    global MAC
    MAC = mac or ""
    out = open(a.out, "a", buffering=1)
    for i in range(a.rounds):
        dev = resolve()
        if not dev or (mac and not bt_connected(mac)):
            out.write(json.dumps({"t": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "stop": "node missing"}) + "\n")
            return 2
        try:
            rec = one_round(dev, a.gap)
            rec["dev"] = dev
        except OSError as e:
            out.write(json.dumps({"t": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "stop": "OSError",
                                  "errno": e.errno, "err": errno.errorcode.get(e.errno, str(e.errno))}) + "\n")
            return 1
        rec["round"] = i
        out.write(json.dumps(rec) + "\n")
        if i + 1 < a.rounds:
            time.sleep(a.period)
    return 0


if __name__ == "__main__":
    sys.exit(main() or 0)
