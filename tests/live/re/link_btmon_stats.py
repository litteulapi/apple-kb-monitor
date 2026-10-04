#!/usr/bin/env python3
"""BR/EDR HID link statistics from a btmon capture (read only).

Input: text output of `btmon -r capture.snoop -T --no-pager -C 200` (or a .snoop, decoded here).
Output: JSON on stdout — counters per L2CAP channel, HID transactions (type/parameter),
sniff/active cycles, sniff intervals, input report bursts, MGMT requesters.

Privacy: the bytes of the keyboard input reports (0xA1 0x01 …) are NEVER
copied; only their count and timestamps feed the statistics.

Usage:
    link_btmon_stats.py capture.snoop|capture.txt
"""
from __future__ import annotations

import collections
import datetime as dt
import json
import re
import statistics
import subprocess
import sys

HDR = re.compile(r"^([<>@=]) (.+?)\s{2,}.*?(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{6})\s*$")
CHAN = re.compile(r"Channel: (\d+) len (\d+)")
HEX = re.compile(r"^\s{8}((?:[0-9a-f]{2} )+)")
MODE = re.compile(r"Mode: (\w+)")
INTERVAL = re.compile(r"Interval: ([\d.]+) msec")

HIDP_TYPES = {
    0x0: "HANDSHAKE", 0x1: "HID_CONTROL", 0x4: "GET_REPORT", 0x5: "SET_REPORT",
    0x6: "GET_PROTOCOL", 0x7: "SET_PROTOCOL", 0x8: "GET_IDLE", 0x9: "SET_IDLE",
    0xA: "DATA", 0xB: "DATC",
}
HANDSHAKE = {0: "SUCCESSFUL", 1: "NOT_READY", 2: "ERR_INVALID_REPORT_ID",
             3: "ERR_UNSUPPORTED_REQUEST", 4: "ERR_INVALID_PARAMETER", 0xE: "ERR_UNKNOWN",
             0xF: "ERR_FATAL"}


def load(path: str) -> list[str]:
    if path.endswith(".txt"):
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read().splitlines()
    out = subprocess.run(["btmon", "-r", path, "-T", "--no-pager", "-C", "200"],
                         capture_output=True, text=True, check=False)
    return out.stdout.splitlines()


def parse(lines: list[str]):
    """Splits into packets: (dir, title, ts, body)."""
    pkts, cur = [], None
    for ln in lines:
        m = HDR.match(ln)
        if m:
            if cur:
                pkts.append(cur)
            ts = dt.datetime.strptime(m.group(3), "%Y-%m-%d %H:%M:%S.%f")
            cur = [m.group(1), m.group(2).strip(), ts, []]
        elif cur:
            cur[3].append(ln)
    if cur:
        pkts.append(cur)
    return pkts


def first_bytes(body: list[str]) -> list[int]:
    for ln in body:
        m = HEX.match(ln)
        if m:
            return [int(x, 16) for x in m.group(1).split()]
    return []


def main() -> int:
    path = sys.argv[1]
    pkts = parse(load(path))
    if not pkts:
        print(json.dumps({"error": "empty capture"}))
        return 1
    t0, t1 = pkts[0][2], pkts[-1][2]
    res: dict = {"capture": path.rsplit("/", 1)[-1], "start": str(t0), "end": str(t1),
                 "duration_s": round((t1 - t0).total_seconds(), 1)}

    chan = collections.Counter()
    hid = collections.Counter()
    hs = collections.Counter()
    getrep_ids = collections.Counter()
    mgmt = collections.Counter()
    hci_cmd = collections.Counter()
    events = collections.Counter()
    modes = []          # (ts, mode, interval_ms)
    input_ts = []       # timestamps of the input reports (interrupt)
    ctrl_out_ts = []    # host -> keyboard requests on the control channel
    exits = []          # HCI Exit Sniff Mode commands (host)

    for d, title, ts, body in pkts:
        if title.startswith("ACL:"):
            cm = next((CHAN.search(b) for b in body if CHAN.search(b)), None)
            if not cm:
                continue
            cid = int(cm.group(1))
            b = first_bytes(body)
            chan[f"{d} cid {cid}"] += 1
            if not b:
                continue
            t, p = b[0] >> 4, b[0] & 0x0F
            name = HIDP_TYPES.get(t, f"0x{t:X}")
            key = f"{d} cid {cid} {name}"
            if name in ("DATA",):
                key += f" type={p} id=0x{b[1]:02X}" if len(b) > 1 else ""
                if d == ">" and p == 1:  # input report
                    input_ts.append(ts)
            elif name == "GET_REPORT":
                rid = b[1] if len(b) > 1 else None
                getrep_ids[f"type={p & 3} id=0x{rid:02X}" if rid is not None else f"type={p & 3}"] += 1
                ctrl_out_ts.append(ts)
            elif name in ("SET_REPORT", "SET_PROTOCOL", "SET_IDLE", "HID_CONTROL"):
                key += f" param={p}"
                ctrl_out_ts.append(ts)
            elif name == "HANDSHAKE":
                hs[HANDSHAKE.get(p, str(p))] += 1
            hid[key] += 1
        elif title.startswith("HCI Event: Mode Change"):
            mo = next((MODE.search(x).group(1) for x in body if MODE.search(x)), "?")
            iv = next((float(INTERVAL.search(x).group(1)) for x in body if INTERVAL.search(x)), 0.0)
            modes.append((ts, mo, iv))
            events["Mode Change"] += 1
        elif title.startswith("HCI Event:"):
            events[title.split(" (")[0][11:]] += 1
        elif title.startswith("HCI Command:"):
            hci_cmd[title.split(" (")[0][13:]] += 1
            if title.startswith("HCI Command: Exit Sniff Mode"):
                exits.append(ts)
        elif title.startswith("MGMT Command:") or title.startswith("MGMT Open:"):
            mgmt[title.split(" (")[0]] += 1

    res["acl_packets_by_channel"] = dict(chan)
    res["hidp_transactions"] = dict(hid.most_common())
    res["get_report_by_id"] = dict(getrep_ids.most_common())
    res["handshakes"] = dict(hs)
    res["hci_commands"] = dict(hci_cmd.most_common())
    res["hci_events"] = dict(events.most_common())
    res["mgmt"] = dict(mgmt.most_common(10))

    # Sniff: negotiated intervals, time spent in active, duration of each stay in sniff.
    ivs = collections.Counter(f"{m[2]:.3f} ms" for m in modes if m[1] == "Sniff")
    active_durs, sniff_durs = [], []
    for a, b in zip(modes, modes[1:]):
        dur = (b[0] - a[0]).total_seconds()
        (active_durs if a[1] == "Active" else sniff_durs).append(dur)
    def summ(xs):
        if not xs:
            return None
        xs = sorted(xs)
        return {"n": len(xs), "min_s": round(xs[0], 3), "median_s": round(statistics.median(xs), 3),
                "p90_s": round(xs[int(0.9 * (len(xs) - 1))], 3), "max_s": round(xs[-1], 3),
                "total_s": round(sum(xs), 1)}
    res["sniff"] = {"intervals": dict(ivs), "active_stays": summ(active_durs),
                    "sniff_stays": summ(sniff_durs),
                    "active_share_pct": round(100 * sum(active_durs) / max(1e-9, sum(active_durs) + sum(sniff_durs)), 2)}

    # Wake-up delay: Exit Sniff Mode -> Mode Change (Active), depending on the preceding inactivity
    # (time since entering sniff or since the last input report, whichever is more recent).
    buckets = {"<0.5s": [], "0.5-25s": [], ">=25s": []}
    for ex in exits:
        nxt = next((m for m in modes if m[0] >= ex), None)
        sn = [m[0] for m in modes if m[0] < ex and m[1] == "Sniff"]
        if not nxt or nxt[1] != "Active" or not sn:
            continue
        ref = max([sn[-1]] + [t for t in input_ts if t < ex][-1:])
        idle = (ex - ref).total_seconds()
        k = "<0.5s" if idle < 0.5 else "0.5-25s" if idle < 25 else ">=25s"
        buckets[k].append((nxt[0] - ex).total_seconds())
    res["exit_sniff_wakeup_by_idle"] = {k: summ(v) for k, v in buckets.items()}

    # Input report bursts (keystrokes): gap between reports, sessions separated by > 2 s.
    gaps = [(b - a).total_seconds() for a, b in zip(input_ts, input_ts[1:])]
    sessions = 1 + sum(1 for g in gaps if g > 2) if input_ts else 0
    res["input_reports"] = {"n": len(input_ts), "sessions_gt2s": sessions,
                              "gap_median_ms": round(1000 * statistics.median(gaps), 1) if gaps else None,
                              "gap_min_ms": round(1000 * min(gaps), 1) if gaps else None}

    # Delay between the last activity (input or request) and the return to sniff.
    acts = sorted(input_ts + ctrl_out_ts)
    back = []
    for ts, mo, _ in modes:
        if mo != "Sniff":
            continue
        prev = [a for a in acts if a <= ts]
        if prev:
            back.append((ts - prev[-1]).total_seconds())
    res["back_to_sniff_after_last_activity"] = summ(back)
    # Silences of the interrupt channel (keepalive?): longest gaps without any ACL packet.
    acl_ts = [p[2] for p in pkts if p[1].startswith("ACL:")]
    sil = sorted(((b - a).total_seconds() for a, b in zip(acl_ts, acl_ts[1:])), reverse=True)[:5]
    res["longest_acl_silences_s"] = [round(s, 1) for s in sil]
    print(json.dumps(res, ensure_ascii=False, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
