#!/usr/bin/env python3
"""Série temporelle batterie A1314 (BCM2042) — LECTURE SEULE (VERIF-BATTERIE, 2026-10-01).

Chaque salve (au plus une toutes les --period s, 300 s par défaut) :
  1. lit sysfs power_supply/capacity en chronométrant la lecture (latence µs = cache
     noyau, latence ms = le noyau a émis un GET_REPORT au clavier) ;
  2. HIDIOCGFEATURE (GET_REPORT Feature) sur les 26 IDs connus, 0x4C EXCLU (secret) ;
     les IDs batterie d'abord pour qu'ils soient quasi simultanés ;
  3. relit sysfs capacity.
Aucun SET_REPORT, aucune écriture : le nœud est ouvert en O_RDONLY.
Arrêt immédiat à la première erreur d'E/S (EIO, ENODEV, ETIMEDOUT...) ou disparition du nœud.

usage: verif_battery_series.py 04:DB:56:CA:42:EE|/dev/hidraw7 /sys/class/power_supply/<psy> --rounds 40 --out s.jsonl
       verif_battery_series.py --analyze s.jsonl
"""
import argparse, errno, fcntl, json, os, sys, time

MAC = ""
BATT_FIRST = [0x46, 0xFF, 0x49, 0x47, 0xEA]
OTHERS = [0x09, 0x4A, 0x4B, 0x4F, 0x51, 0x52, 0x53, 0x54, 0x5A, 0x5B, 0x5C, 0x5D,
          0x60, 0xD1, 0xD8, 0xEB, 0xF4, 0xF5, 0xF6, 0xF7, 0xFE]
IDS_FULL = BATT_FIRST + OTHERS  # 26 ids ; 0x4C volontairement absent (non utilisé : consigne du 01/10 12:20)
# Mode léger imposé après le décrochage du 01/10 12:13 : 4 rapports, jamais 0xFE ni balayage.
IDS = [0x46, 0x49, 0x47, 0xEA]


def bt_connected(mac):
    """État BlueZ (cache de bluetoothd, aucune requête radio)."""
    import subprocess
    try:
        out = subprocess.run(["bluetoothctl", "info", mac], capture_output=True, text=True, timeout=10).stdout
    except Exception:
        return False
    return "\tConnected: yes" in out


def upower_pct(mac):
    """Pourcentage tenu par UPower (déjà interrogé par le système) : aucune requête supplémentaire."""
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


def read_capacity(psy):
    import glob  # le suffixe -battery-NN change à chaque reconnexion : psy peut être un motif
    m = sorted(glob.glob(psy))
    if not m:
        raise OSError(errno.ENODEV, "power_supply absent")
    psy = m[-1]
    t = time.monotonic()
    with open(os.path.join(psy, "capacity")) as f:
        v = f.read().strip()
    return int(v), round((time.monotonic() - t) * 1e6)  # µs


def find_hidraw(mac):
    """Nœud hidraw dont HID_UNIQ = mac (le numéro change d'une reconnexion à l'autre)."""
    import glob
    for ue in glob.glob("/sys/class/hidraw/hidraw*/device/uevent"):
        try:
            if ("hid_uniq=" + mac.lower()) in open(ue).read().lower():
                return "/dev/" + ue.split("/")[4]
        except OSError:
            pass
    return None


def one_round(dev, psy, gap):
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
    print("t | v46 vff v49 | p47 pea | cap_av cap_ap (lat_us) | ff3")
    for r in rows:
        d = decode(r["rep"])
        print(r["t"][11:19], d.get("v46"), d.get("vff"), d.get("v49"), "|", d.get("p47"), d.get("pea"),
              "|", r.get("upower_before", r.get("cap_before")), r.get("upower_after", r.get("cap_after")), "|", d.get("ff3"))
    # registres qui bougent
    keys = rows[0]["rep"].keys() if rows else []
    for k in keys:
        vals = sorted({r["rep"].get(k) for r in rows})
        if len(vals) > 1:
            print("VARIE %s : %d valeurs %s" % (k, len(vals), vals[:8]))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dev", nargs="?")
    ap.add_argument("psy", nargs="?")
    ap.add_argument("--rounds", type=int, default=40)
    ap.add_argument("--period", type=float, default=300)
    ap.add_argument("--gap", type=float, default=0.4)
    ap.add_argument("--out")
    ap.add_argument("--analyze")
    ap.add_argument("--wait-node", action="store_true", help="attendre passivement l'apparition du nœud")
    a = ap.parse_args()
    if a.analyze:
        return analyze(a.analyze)
    if a.period < 300:
        sys.exit("période < 300 s refusée (consigne : une lecture complète par 5 min au plus)")
    # Attente passive du nœud (aucune E/S vers le clavier) : le clavier dort après inactivité,
    # on ne lit qu'une fois qu'il s'est reconnecté de lui-même (appui de touche).
    mac = a.dev if ":" in a.dev else None
    resolve = (lambda: find_hidraw(mac)) if mac else (lambda: a.dev if os.path.exists(a.dev) else None)
    while a.wait_node and not (resolve() and (not mac or bt_connected(mac))):
        time.sleep(10)
    # Respect de la cadence entre deux exécutions : dernière salve du fichier il y a >= period.
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
            out.write(json.dumps({"t": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "stop": "noeud absent"}) + "\n")
            return 2
        try:
            rec = one_round(dev, a.psy, a.gap)
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
