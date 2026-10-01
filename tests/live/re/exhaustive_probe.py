#!/usr/bin/env python3
"""Exhaustive READ-ONLY GET_REPORT probe of the A1314 (BCM2042) — RE HID 2026-10-01.

For one report type per pass (feature | input | output), every report id
0x00-0xFF and every buffer length of LENGTHS, issues the matching hidraw
GET ioctl (HIDIOCGFEATURE / HIDIOCGINPUT / HIDIOCGOUTPUT) on a node opened
O_RDONLY. Never SET_REPORT, never write(), never an LED or link change.

Safety rules (enforced, not advisory):
  * `bluetoothctl info MAC` must say "Connected: yes" before the pass;
  * a refusal of an unimplemented id is a FAST EIO (HIDP HANDSHAKE error,
    a few ms). A SLOW error (> --slow s, i.e. BlueZ REPORT_REQ_TIMEOUT 3 s /
    uhid 5 s) or any errno other than EIO/EINVAL means the link is failing:
    the pass aborts immediately and the abort is recorded;
  * --gap seconds between two requests (no tight loop);
  * a lock file refuses a second pass less than 300 s after the previous one.

Report 0x4C (paired host + key material) is never stored in clear: only its
length, first two bytes and a truncated SHA-256 of the returned bytes.

usage: exhaustive_probe.py --type feature --out pass.jsonl [--dev /dev/hidraw7]
       exhaustive_probe.py --summarize a.jsonl b.jsonl ... > summary.json
"""
import argparse, errno, fcntl, hashlib, json, os, subprocess, sys, time

MAC = "04:DB:56:CA:42:EE"
LENGTHS = [1, 2, 3, 4, 8, 9, 16, 20, 32, 64, 256]
NR = {"feature": 0x07, "input": 0x0A, "output": 0x0C}   # HIDIOCG* numbers
SENSITIVE = {0x4C}
LOCK = os.path.expanduser("~/.cache/apple-kb-monitor-re-lastpass")


def ioc(nr, length):
    # _IOC(_IOC_READ|_IOC_WRITE, 'H', nr, length)
    return (3 << 30) | (length << 16) | (ord("H") << 8) | nr


def connected():
    out = subprocess.run(["bluetoothctl", "info", MAC], capture_output=True,
                         text=True, timeout=10).stdout
    return "Connected: yes" in out


def record(rid, data, rtype=None):
    if rtype == "input" and rid == 0x01 and len(data) > 1:
        # live boot-keyboard state = what the user is typing: never stored.
        # Only the shape is kept (reserved byte, number of pressed keys).
        return {"len": len(data), "keystate": True,
                "mod_nonzero": data[1] != 0,
                "reserved": data[2] if len(data) > 2 else None,
                "nkeys": sum(1 for b in data[3:] if b)}
    if rid in SENSITIVE:
        r = {"len": len(data), "first": data[:2].hex(), "masked": True}
        # a hash of a short prefix (e.g. 9 bytes = 8 known + 1 secret) would be
        # brute-forceable: only the complete 20-byte report is fingerprinted
        if len(data) >= 20:
            r["sha256_8"] = hashlib.sha256(data).hexdigest()[:16]
        return r
    return {"len": len(data), "hex": data.hex()}


def run_pass(args):
    try:
        last = os.path.getmtime(LOCK)
        if time.time() - last < 300:
            sys.exit(f"refused: previous pass {time.time()-last:.0f}s ago (< 300 s)")
    except FileNotFoundError:
        pass
    if not connected():
        sys.exit("refused: keyboard not connected")
    node = os.path.basename(args.dev)
    uev = open(f"/sys/class/hidraw/{node}/device/uevent").read().lower()
    if f"hid_uniq={MAC.lower()}" not in uev:
        sys.exit(f"refused: {args.dev} is not {MAC}")
    os.makedirs(os.path.dirname(LOCK), exist_ok=True)
    open(LOCK, "w").close()
    nr = NR[args.type]
    fd = os.open(args.dev, os.O_RDONLY | os.O_CLOEXEC)
    out = open(args.out, "a")
    t_start = time.time()
    n = 0
    abort = None
    try:
        for rid in range(256):
            for length in LENGTHS:
                buf = bytearray(length)
                buf[0] = rid
                t0 = time.monotonic()
                try:
                    ret = fcntl.ioctl(fd, ioc(nr, length), buf, True)
                    err = None
                except OSError as e:
                    ret, err = None, e.errno
                dt = time.monotonic() - t0
                n += 1
                row = {"type": args.type, "id": rid, "buflen": length,
                       "lat_ms": round(dt * 1000, 1), "ts": round(time.time(), 3)}
                if err is None:
                    row["ret"] = ret
                    row.update(record(rid, bytes(buf[:ret]), args.type))
                else:
                    row["errno"] = errno.errorcode.get(err, err)
                out.write(json.dumps(row) + "\n")
                if err is not None and (err not in (errno.EIO, errno.EINVAL)
                                        or dt > args.slow):
                    abort = f"id 0x{rid:02x} len {length}: {row['errno']} after {dt:.2f}s"
                    break
                time.sleep(args.gap)
            if abort:
                break
    finally:
        os.close(fd)
        meta = {"_pass": args.type, "requests": n, "start": round(t_start, 3),
                "dur_s": round(time.time() - t_start, 1), "abort": abort,
                "connected_after": connected()}
        out.write(json.dumps(meta) + "\n")
        out.close()
        print(json.dumps(meta))


def summarize(paths):
    cells, passes = {}, []
    for p in paths:
        for line in open(p):
            r = json.loads(line)
            if r.get("id") in SENSITIVE and r.get("len", 99) < 20:
                r.pop("sha256_8", None)          # see record()
            if "_pass" in r:
                passes.append(r)
                continue
            key = f"{r['type']}:{r['id']:02x}"
            c = cells.setdefault(key, {"by_len": {}, "lat_ms": []})
            c["lat_ms"].append(r["lat_ms"])
            v = r.get("errno") or {k: r[k] for k in ("ret", "hex", "first", "sha256_8",
                                                     "keystate", "mod_nonzero",
                                                     "reserved", "nkeys")
                                   if k in r}
            c["by_len"][str(r["buflen"])] = v
    res = {"passes": passes, "answering": {}, "refused": {}}
    for key, c in sorted(cells.items()):
        lat = sorted(c["lat_ms"])
        ok = {k: v for k, v in c["by_len"].items() if isinstance(v, dict)}
        if ok:
            res["answering"][key] = {"by_len": c["by_len"],
                                     "lat_ms_median": lat[len(lat) // 2]}
        else:
            errs = sorted(set(c["by_len"].values()))
            res["refused"].setdefault("/".join(errs), []).append(key)
    for k in res["refused"]:
        res["refused"][k] = {"count": len(res["refused"][k]), "keys": res["refused"][k]}
    json.dump(res, sys.stdout, indent=1, sort_keys=True)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--type", choices=NR)
    ap.add_argument("--dev", default="/dev/hidraw7")
    ap.add_argument("--out")
    ap.add_argument("--gap", type=float, default=0.03)
    ap.add_argument("--slow", type=float, default=1.0)
    ap.add_argument("--summarize", nargs="+")
    a = ap.parse_args()
    if a.summarize:
        summarize(a.summarize)
    elif a.type and a.out:
        run_pass(a)
    else:
        ap.error("--type and --out, or --summarize")
