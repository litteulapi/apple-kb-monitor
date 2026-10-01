#!/usr/bin/env python3
"""Read-only sampler of the BCM2042 vendor Feature Reports (audit RE, 2026-10-01).

Only HIDIOCGFEATURE (GET_REPORT, type Feature) on the 22 report ids the
keyboard already answers; never SET_REPORT / SET_FEATURE / write().
Records the ioctl return value (= real payload length, which the
`--dump` of apple-kb-monitor hides by trimming trailing zeros) and
the payload, N rounds spaced by --period seconds, --gap seconds
between two requests (the keyboard sleeps; every request wakes the radio).

Report 0x4C is redacted after its 7th byte (it holds the paired host
address followed by key material) unless --raw-4c is given.

usage: sample_reports.py /dev/hidrawN [--rounds 4] [--period 90] [--gap 0.4]
       [--ids ea,47,...] > samples.jsonl
"""
import argparse, fcntl, json, os, sys, time

IDS = [0x09, 0x46, 0x47, 0x49, 0x4A, 0x4B, 0x4C, 0x4F, 0x51, 0x52, 0x53,
       0x5A, 0x5B, 0x60, 0xEA, 0xEB, 0xF4, 0xF5, 0xF6, 0xF7, 0xFE, 0xFF]


def hidiocgfeature(length):
    # _IOC(_IOC_READ|_IOC_WRITE, 'H', 0x07, length)
    return (3 << 30) | (length << 16) | (ord('H') << 8) | 0x07


def get_feature(fd, rid, size=64):
    buf = bytearray(size)
    buf[0] = rid
    t0 = time.monotonic()
    try:
        n = fcntl.ioctl(fd, hidiocgfeature(size), buf, True)
        err = None
    except OSError as e:
        n, err = -1, e.errno
    return n, bytes(buf[:max(n, 0)]), err, round((time.monotonic() - t0) * 1000, 1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dev")
    ap.add_argument("--rounds", type=int, default=4)
    ap.add_argument("--period", type=float, default=90)
    ap.add_argument("--gap", type=float, default=0.4)
    ap.add_argument("--ids", default="")
    ap.add_argument("--raw-4c", action="store_true")
    a = ap.parse_args()
    ids = [int(x, 16) for x in a.ids.split(",")] if a.ids else IDS
    fd = os.open(a.dev, os.O_RDWR | os.O_CLOEXEC)
    try:
        for r in range(a.rounds):
            for rid in ids:
                n, data, err, ms = get_feature(fd, rid)
                hexd = data.hex()
                if rid == 0x4C and not a.raw_4c and len(data) > 7:
                    hexd = data[:7].hex() + "<redacted %d bytes>" % (len(data) - 7)
                print(json.dumps({"t": round(time.time(), 1), "round": r,
                                  "id": "0x%02X" % rid, "ret": n,
                                  "errno": err, "ms": ms, "hex": hexd}),
                      flush=True)
                time.sleep(a.gap)
            if r + 1 < a.rounds:
                time.sleep(a.period)
    finally:
        os.close(fd)


if __name__ == "__main__":
    sys.exit(main())
