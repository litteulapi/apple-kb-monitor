#!/usr/bin/env python3
"""Low-rate READ-ONLY time series of the 26 answering Feature reports (0xFE excluded) (RE HID 2026-10-01).

Every --period seconds (>= 300), if and only if `bluetoothctl info` says the
keyboard is connected, reads the 26 Feature report ids once each
(HIDIOCGFEATURE, buffer 64, node opened O_RDONLY, --gap s between requests).
The latency of the FIRST request of a round is kept: it measures how deep the
radio was asleep (sniff/park exit) before the round.

Between rounds, passive observation only:
  * BlueZ link state every 15 s (`bluetoothctl info`, cached by bluetoothd, no
    radio traffic) -> disconnect / reconnect timestamps;
  * input reports read from the hidraw node: ONLY the timestamp and the report
    id are kept (never the key codes), to correlate typing with the values.

A round stops at the first slow error (> 1 s) or any errno other than EIO.
0x4C is never stored: length, first byte and SHA-256/8 of the full report only.

usage: exhaustive_timeseries.py --out ts.jsonl --mac XX:XX:XX:XX:XX:XX [--period 300] [--duration 3600]
"""
import argparse, errno, fcntl, glob, hashlib, json, os, select, subprocess, sys, time

MAC = None  # --mac or $KB_MAC
IDS = [0x09, 0x46, 0x47, 0x49, 0x4A, 0x4B, 0x4C, 0x4F, 0x51, 0x52, 0x53, 0x54,
       0x5A, 0x5B, 0x5C, 0x5D, 0x60, 0xD1, 0xD8, 0xEA, 0xEB, 0xF4, 0xF5, 0xF6,
       0xF7, 0xFF]   # 0xFE never read


def ioc_gfeature(length):
    return (3 << 30) | (length << 16) | (ord("H") << 8) | 0x07


def bluez():
    out = subprocess.run(["bluetoothctl", "info", MAC], capture_output=True,
                         text=True, timeout=10).stdout
    pct = None
    for line in out.splitlines():
        if "Battery Percentage" in line:
            pct = int(line.split("(")[-1].rstrip(")"))
    return "Connected: yes" in out, pct


def find_node():
    for d in glob.glob("/sys/class/hidraw/hidraw*/device/uevent"):
        if f"hid_uniq={MAC.lower()}" in open(d).read().lower():
            return "/dev/" + d.split("/")[4]
    return None


def round_reads(dev, gap, slow):
    fd = os.open(dev, os.O_RDONLY | os.O_CLOEXEC)
    res, abort = {}, None
    try:
        for i, rid in enumerate(IDS):
            buf = bytearray(64)
            buf[0] = rid
            t0 = time.monotonic()
            try:
                n = fcntl.ioctl(fd, ioc_gfeature(64), buf, True)
                err = None
            except OSError as e:
                n, err = -1, e.errno
            ms = round((time.monotonic() - t0) * 1000, 1)
            key = f"{rid:02x}"
            if err is not None:
                res[key] = {"errno": errno.errorcode.get(err, err), "ms": ms}
                if err != errno.EIO or ms > slow * 1000:
                    abort = f"0x{key}: {res[key]}"
                    break
            elif rid == 0x4C:
                d = bytes(buf[:n])
                res[key] = {"len": n, "first": d[:2].hex(), "ms": ms,
                            "sha256_8": hashlib.sha256(d).hexdigest()[:16]
                            if n >= 20 and os.environ.get("AKM_RE_FINGERPRINT") else None}  # opt-in
            else:
                res[key] = {"hex": bytes(buf[:n]).hex(), "ms": ms}
            time.sleep(gap)
    finally:
        os.close(fd)
    return res, abort


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--mac", default=os.environ.get("KB_MAC"),
                    help="keyboard address (default: $KB_MAC)")
    ap.add_argument("--period", type=float, default=300)
    ap.add_argument("--duration", type=float, default=3600)
    ap.add_argument("--gap", type=float, default=0.3)
    ap.add_argument("--slow", type=float, default=1.0)
    a = ap.parse_args()
    if not a.mac:
        ap.error("--mac (or $KB_MAC) is required")
    global MAC
    MAC = a.mac
    if a.period < 300:
        sys.exit("period < 300 s refused")
    out = open(a.out, "a")

    def emit(obj):
        obj["t"] = round(time.time(), 2)
        out.write(json.dumps(obj) + "\n")
        out.flush()

    end = time.time() + a.duration
    next_round = time.time()
    last_link, last_bz = None, 0
    in_fd, in_node, n_in = None, None, 0
    while time.time() < end:
        now = time.time()
        if now - last_bz >= 15:
            last_bz = now
            link, pct = bluez()
            if link != last_link:
                emit({"ev": "link", "connected": link, "bluez_pct": pct})
                last_link = link
                if in_fd is not None:
                    os.close(in_fd)
                    in_fd = None
            if link and in_fd is None:
                in_node = find_node()
                if in_node:
                    in_fd = os.open(in_node, os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC)
        if now >= next_round:
            next_round = now + a.period
            if last_link and in_node:
                res, abort = round_reads(in_node, a.gap, a.slow)
                emit({"ev": "round", "node": in_node, "bluez_pct": bluez()[1],
                      "inputs_since_last": n_in, "first_ms": next(iter(res.values()))["ms"]
                      if res else None, "abort": abort, "r": res})
                n_in = 0
            else:
                emit({"ev": "round_skipped", "reason": "not connected"})
        if in_fd is not None:
            r, _, _ = select.select([in_fd], [], [], 1.0)
            if r:
                try:
                    data = os.read(in_fd, 64)
                    emit({"ev": "input", "rid": f"{data[0]:02x}" if data else None})
                    n_in += 1
                except OSError as e:
                    emit({"ev": "input_err", "errno": errno.errorcode.get(e.errno, e.errno)})
                    os.close(in_fd)
                    in_fd = None
        else:
            time.sleep(1.0)
    out.close()


if __name__ == "__main__":
    main()
