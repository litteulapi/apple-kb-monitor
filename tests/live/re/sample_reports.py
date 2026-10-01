#!/usr/bin/env python3
"""Échantillonnage LECTURE SEULE des rapports Feature d'un clavier Apple BT (A1314).

Uniquement des GET_REPORT (ioctl HIDIOCGFEATURE) : aucun SET_REPORT/SET_FEATURE,
aucune écriture sur le clavier. Le périphérique est ouvert en O_RDONLY.

Chaque échantillon = une ligne JSON (JSONL) :
  - ts, rapports {id_hex: {len, hex}} (longueur réellement rendue par l'ioctl),
  - batterie noyau (power_supply capacity/status), BlueZ Battery1.Percentage,
  - décodages provisoires (tension, seuils) pour faciliter la corrélation.

Le rapport 0x4C (clé d'identité, SENSIBLE) n'est JAMAIS écrit en clair :
seuls sa longueur, son premier octet et une empreinte SHA-256 tronquée (8 o)
sont enregistrés — suffisant pour détecter un changement, pas pour la reconstruire.

Garde-fou radio : intervalle minimum 60 s entre deux échantillons.

Exemples :
  sample_reports.py --once
  sample_reports.py --interval 300 --count 13 --out samples.jsonl
  sample_reports.py --scan-once          # balaye 0x00-0xFF une fois (GET seul)
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
DEFAULT_IDS = [0x09, 0x46, 0x47, 0x49, 0x4A, 0x4B, 0x4C, 0x4F, 0x51, 0x52, 0x53,
               0x54, 0x5A, 0x5B, 0x5C, 0x5D, 0x60, 0xD1, 0xD8, 0xEA, 0xEB, 0xF4,
               0xF5, 0xF6, 0xF7, 0xFE, 0xFF]
SENSITIVE = {0x4C}
BUF = 64


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
        return {"len": r["len"], "first": raw[:2].hex(),
                "sha256_8": hashlib.sha256(raw).hexdigest()[:16], "masked": True}
    return {"len": r["len"], "hex": raw.hex()}


def be16(b, o):
    return (b[o] << 8) | b[o + 1]


def decode(reps):
    """Décodages provisoires (voir docs/HARDWARE-RAPPORTS-HID.md)."""
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
    """Résumé d'un JSONL : valeurs distinctes par rapport et plages décodées."""
    allrows = [json.loads(l) for l in open(path) if l.strip()]
    rows = [r for r in allrows if "reports" in r]
    absent = len(allrows) - len(rows)
    if not rows:
        sys.exit(f"aucun échantillon exploitable ({absent} absents)")
    print(f"{len(rows)} échantillons (+{absent} clavier absent), "
          f"{rows[0]['ts']} -> {rows[-1]['ts']}")
    ids = sorted({k for r in rows for k in r["reports"]})
    for k in ids:
        vals = [r["reports"].get(k, {}) for r in rows]
        keyed = [v.get("hex") or v.get("sha256_8") or f"err{v.get('err')}" for v in vals]
        uniq = list(dict.fromkeys(keyed))
        tag = "CONSTANT" if len(uniq) == 1 else f"VARIE ({len(uniq)} valeurs)"
        print(f"  0x{k.upper()} {tag}: {', '.join(uniq[:6])}{' …' if len(uniq) > 6 else ''}")
    keys = sorted({k for r in rows for k in r.get("decoded", {})})
    for k in keys:
        v = [r["decoded"][k] for r in rows if k in r.get("decoded", {})]
        if v and isinstance(v[0], int):
            print(f"  {k}: min {min(v)} max {max(v)} premier {v[0]} dernier {v[-1]}")
    hp = [r["host"].get("ps_capacity") for r in rows]
    print(f"  noyau capacity: {list(dict.fromkeys(hp))}")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mac", default="04:DB:56:CA:42:EE")
    ap.add_argument("--dev", help="/dev/hidrawN (sinon trouvé par HID_UNIQ)")
    ap.add_argument("--once", action="store_true")
    ap.add_argument("--scan-once", action="store_true",
                    help="GET_REPORT sur les 256 IDs, une seule fois")
    ap.add_argument("--interval", type=int, default=300)
    ap.add_argument("--count", type=int, default=13)
    ap.add_argument("--out", help="fichier JSONL (ajout) ; défaut stdout")
    ap.add_argument("--analyze", metavar="JSONL",
                    help="résumer un fichier d'échantillons (aucun accès matériel)")
    a = ap.parse_args()

    if a.analyze:
        analyze(a.analyze)
        return

    if a.interval < MIN_INTERVAL:
        sys.exit(f"intervalle < {MIN_INTERVAL} s refusé (garde-fou radio)")

    ids = list(range(256)) if a.scan_once else DEFAULT_IDS
    n = 1 if (a.once or a.scan_once) else a.count
    out = open(a.out, "a") if a.out else sys.stdout
    for i in range(n):
        # Le clavier se déconnecte quand il dort : on re-cherche le nœud à
        # chaque tour et on journalise l'absence au lieu de le réveiller.
        dev = a.dev or find_hidraw(a.mac)
        s = None
        if dev:
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
        if i + 1 < n:
            time.sleep(a.interval)


if __name__ == "__main__":
    main()
