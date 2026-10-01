#!/usr/bin/env python3
"""Qui, dans tout le cache dyld, référence un sélecteur ou une classe ObjC donné(e) ?

Le cache arm64e coalesce les chaînes de sélecteurs : `strings` ne dit pas quelle image les utilise.
Ce script (Python pur, sans Ghidra) :
  1. lit les mappings de chaque fichier du cache (en-tête dyld_cache_header) ;
  2. trouve l'adresse virtuelle de chaque nom (chaîne terminée par NUL, précédée d'un NUL) ;
  3. cherche, dans les mappings de données, les pointeurs 64 bits qui y mènent. Format arm64e
     (slide info v5) : décalage depuis la base du cache dans les 34 bits bas, sans bit d'authentification ;
     on accepte aussi l'adresse brute ;
  4. rattache chaque pointeur à l'image qui le contient grâce au fichier `.map` du cache.

  06_dsc_selref_scan.py CACHE_BASE_PATH nom1 nom2 ...   (ex. .../dyld_shared_cache_arm64e sendSCOLinkActive)
Sortie : nom -> liste d'images (avec le segment) contenant un pointeur vers ce nom.
Limite : une référence de sélecteur via `objc_msgSend$sel` (stubs) passe aussi par une selref, donc couverte.
"""
import glob
import mmap
import os
import re
import struct
import sys


def mappings(path):
    with open(path, "rb") as f:
        h = f.read(0x200)
    off, cnt = struct.unpack_from("<II", h, 0x10)
    with open(path, "rb") as f:
        f.seek(off)
        raw = f.read(cnt * 32)
    return [struct.unpack_from("<QQQII", raw, i * 32) for i in range(cnt)]


def load_map(mapfile):
    """Renvoie [(start, end, image, segment)] à partir du .map (lignes « __SEG 0xSTART -> 0xEND »)."""
    out, img = [], None
    rx = re.compile(r"^\s*(__\w+)\s+0x([0-9A-Fa-f]+)\s*->\s*0x([0-9A-Fa-f]+)")
    for line in open(mapfile, errors="replace"):
        if line.startswith("/"):
            img = line.strip()
            continue
        m = rx.match(line)
        if m and img:
            out.append((int(m.group(2), 16), int(m.group(3), 16), img, m.group(1)))
    out.sort()
    return out


def main():
    base_path, names = sys.argv[1], sys.argv[2:]
    files = sorted(p for p in glob.glob(base_path + "*") if not p.endswith((".map", ".atlas")))
    regions = []  # (vmaddr, size, fileoff, mm)
    cache_base = None
    for p in files:
        fd = open(p, "rb")
        mm = mmap.mmap(fd.fileno(), 0, access=mmap.ACCESS_READ)
        for addr, size, foff, maxp, initp in mappings(p):
            if cache_base is None or addr < cache_base:
                cache_base = addr
            regions.append((addr, size, foff, mm, initp, os.path.basename(p)))
    imap = load_map(base_path + ".map")

    def img_of(va):
        lo, hi = 0, len(imap) - 1
        while lo <= hi:
            mid = (lo + hi) // 2
            s, e, im, seg = imap[mid]
            if va < s:
                hi = mid - 1
            elif va >= e:
                lo = mid + 1
            else:
                return f"{im} {seg}"
        return "?"

    # 2. adresses des chaînes
    targets = {}
    for n in names:
        pat = b"\x00" + n.encode() + b"\x00"
        for addr, size, foff, mm, initp, fn in regions:
            if initp & 2:  # chaînes en lecture seule seulement
                continue
            i = mm.find(pat, foff, foff + size)
            while i != -1:
                va = addr + (i + 1 - foff)
                targets.setdefault(va, n)
                i = mm.find(pat, i + 1, foff + size)
    print(f"base du cache {cache_base:#x}, {len(targets)} chaînes trouvées", flush=True)
    want = {}
    for va, n in targets.items():
        want[va] = n
        want[va - cache_base] = n
    # 3. balayage des pointeurs (données seulement)
    hits = {}
    for addr, size, foff, mm, initp, fn in regions:
        if initp & 4:  # exécutable : ignorer
            continue
        buf = memoryview(mm)[foff: foff + size]
        n8 = size // 8
        arr = struct.unpack_from(f"<{n8}Q", buf) if n8 < 60_000_000 else ()
        for k, v in enumerate(arr):
            if v == 0 or (v >> 63):
                continue
            t = v & 0x3FFFFFFFF
            n = want.get(t) or want.get(v)
            if n:
                hits.setdefault(n, set()).add(img_of(addr + k * 8))
        del buf
    for n in names:
        print(f"{n}: " + "; ".join(sorted(hits.get(n, ()))))


if __name__ == "__main__":
    main()
