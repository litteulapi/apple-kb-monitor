#!/usr/bin/env python3
"""Statistiques de liaison BR/EDR HID à partir d'une capture btmon (lecture seule).

Entrée : sortie texte de `btmon -r capture.snoop -T --no-pager -C 200` (ou un .snoop, décodé ici).
Sortie : JSON sur stdout — compteurs par canal L2CAP, transactions HID (type/paramètre),
cycles sniff/actif, intervalles de sniff, rafales de rapports d'entrée, demandeurs MGMT.

Confidentialité : les octets des rapports d'entrée clavier (0xA1 0x01 …) ne sont JAMAIS
recopiés ; seuls leur nombre et leur horodatage servent aux statistiques.

Usage :
    link_btmon_stats.py capture.snoop|capture.txt [--handle 256]
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
    """Découpe en paquets : (dir, titre, ts, corps)."""
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
        print(json.dumps({"error": "capture vide"}))
        return 1
    t0, t1 = pkts[0][2], pkts[-1][2]
    res: dict = {"capture": path.rsplit("/", 1)[-1], "debut": str(t0), "fin": str(t1),
                 "duree_s": round((t1 - t0).total_seconds(), 1)}

    chan = collections.Counter()
    hid = collections.Counter()
    hs = collections.Counter()
    getrep_ids = collections.Counter()
    mgmt = collections.Counter()
    hci_cmd = collections.Counter()
    events = collections.Counter()
    modes = []          # (ts, mode, interval_ms)
    input_ts = []       # horodatage des rapports d'entrée (interruption)
    ctrl_out_ts = []    # requêtes hôte -> clavier sur le canal de contrôle
    exits = []          # commandes HCI Exit Sniff Mode (hôte)

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
                if d == ">" and p == 1:  # rapport d'entrée
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

    res["paquets_acl_par_canal"] = dict(chan)
    res["hidp_transactions"] = dict(hid.most_common())
    res["get_report_par_id"] = dict(getrep_ids.most_common())
    res["handshakes"] = dict(hs)
    res["hci_commandes"] = dict(hci_cmd.most_common())
    res["hci_evenements"] = dict(events.most_common())
    res["mgmt"] = dict(mgmt.most_common(10))

    # Sniff : intervalles négociés, durée passée en actif, durée de chaque séjour en sniff.
    ivs = collections.Counter(f"{m[2]:.3f} ms" for m in modes if m[1] == "Sniff")
    active_durs, sniff_durs = [], []
    for a, b in zip(modes, modes[1:]):
        dur = (b[0] - a[0]).total_seconds()
        (active_durs if a[1] == "Active" else sniff_durs).append(dur)
    def summ(xs):
        if not xs:
            return None
        xs = sorted(xs)
        return {"n": len(xs), "min_s": round(xs[0], 3), "mediane_s": round(statistics.median(xs), 3),
                "p90_s": round(xs[int(0.9 * (len(xs) - 1))], 3), "max_s": round(xs[-1], 3),
                "total_s": round(sum(xs), 1)}
    res["sniff"] = {"intervalles": dict(ivs), "sejours_actif": summ(active_durs),
                    "sejours_sniff": summ(sniff_durs),
                    "part_actif_pct": round(100 * sum(active_durs) / max(1e-9, sum(active_durs) + sum(sniff_durs)), 2)}

    # Délai de réveil : Exit Sniff Mode -> Mode Change (Active), selon l'inactivité qui précède
    # (temps depuis l'entrée en sniff ou le dernier rapport d'entrée, le plus récent des deux).
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
    res["reveil_exit_sniff_par_inactivite"] = {k: summ(v) for k, v in buckets.items()}

    # Rafales de rapports d'entrée (frappes) : écart entre rapports, sessions séparées de > 2 s.
    gaps = [(b - a).total_seconds() for a, b in zip(input_ts, input_ts[1:])]
    sessions = 1 + sum(1 for g in gaps if g > 2) if input_ts else 0
    res["rapports_entree"] = {"n": len(input_ts), "sessions_gt2s": sessions,
                              "ecart_mediane_ms": round(1000 * statistics.median(gaps), 1) if gaps else None,
                              "ecart_min_ms": round(1000 * min(gaps), 1) if gaps else None}

    # Délai entre la dernière activité (entrée ou requête) et le retour en sniff.
    acts = sorted(input_ts + ctrl_out_ts)
    back = []
    for ts, mo, _ in modes:
        if mo != "Sniff":
            continue
        prev = [a for a in acts if a <= ts]
        if prev:
            back.append((ts - prev[-1]).total_seconds())
    res["retour_sniff_apres_derniere_activite"] = summ(back)
    # Silences du canal d'interruption (keepalive ?) : plus longs écarts sans aucun paquet ACL.
    acl_ts = [p[2] for p in pkts if p[1].startswith("ACL:")]
    sil = sorted(((b - a).total_seconds() for a, b in zip(acl_ts, acl_ts[1:])), reverse=True)[:5]
    res["plus_longs_silences_acl_s"] = [round(s, 1) for s in sil]
    print(json.dumps(res, ensure_ascii=False, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
