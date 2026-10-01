#!/usr/bin/env python3
"""Décode un kernelcache Apple (IMG4/IM4P, LZFSE/BVX2 ou LZSS) en Mach-O et liste ses entrées fileset.

Analyse statique en lecture seule, pour l'interopérabilité (RE-GHIDRA-KEXT.md).
Ne contient ni ne produit aucun binaire Apple dans le dépôt : sortie dans un dossier de travail.

Dépendances (venv) : pyimg4, pyliblzfse.

  kc_extract.py decode <kernelcache> <sortie.macho>
  kc_extract.py entries <kc.macho> [filtre]
"""
import struct
import sys

LC_SEGMENT_64 = 0x19
LC_FILESET_ENTRY = 0x80000035
CPU = {0x01000007: "x86_64", 0x0100000C: "arm64"}


def decode(src: str, dst: str) -> None:
    raw = open(src, "rb").read()
    import pyimg4

    try:
        im4p = pyimg4.IMG4(raw).im4p
    except Exception:
        im4p = pyimg4.IM4P(raw)
    payload = im4p.payload
    if payload.compression != pyimg4.Compression.NONE:
        payload.decompress()
    data = bytes(payload.output().data)
    magic = struct.unpack_from("<I", data)[0]
    if magic != 0xFEEDFACF:
        sys.exit(f"payload inattendu, magic {magic:#x}")
    open(dst, "wb").write(data)
    cpu, sub, ftype = struct.unpack_from("<iII", data, 4)
    print(f"{dst}: {len(data)} o, cpu {CPU.get(cpu & 0xFFFFFFFF, hex(cpu))} sub {sub & 0xFF:#x}, filetype {ftype}")


def entries(path: str, flt: str = "") -> None:
    d = open(path, "rb").read()
    ncmds = struct.unpack_from("<I", d, 16)[0]
    off = 32
    for _ in range(ncmds):
        cmd, sz = struct.unpack_from("<II", d, off)
        if cmd == LC_FILESET_ENTRY:
            vmaddr, fileoff, stroff = struct.unpack_from("<QQI", d, off + 8)
            name = d[off + stroff : d.index(b"\0", off + stroff)].decode()
            if flt.lower() in name.lower():
                print(f"{vmaddr:#018x} {fileoff:#010x} {name}")
        off += sz


def fileset_ranges(path: str, names) -> dict:
    """{kext: [(segment, vmaddr, vmsize), ...]} hors __LINKEDIT, pour les entrées demandées."""
    d = open(path, "rb").read()
    out = {}
    ncmds = struct.unpack_from("<I", d, 16)[0]
    off = 32
    for _ in range(ncmds):
        cmd, sz = struct.unpack_from("<II", d, off)
        if cmd == LC_FILESET_ENTRY:
            vmaddr, fileoff, stroff = struct.unpack_from("<QQI", d, off + 8)
            name = d[off + stroff : d.index(b"\0", off + stroff)].decode()
            if name in names:
                segs = []
                n2 = struct.unpack_from("<I", d, fileoff + 16)[0]
                o = fileoff + 32
                for _ in range(n2):
                    c, s = struct.unpack_from("<II", d, o)
                    if c == LC_SEGMENT_64:
                        seg = d[o + 8 : o + 24].split(b"\0")[0].decode()
                        a, n = struct.unpack_from("<QQ", d, o + 24)
                        if seg != "__LINKEDIT" and n:
                            segs.append((seg, a, n))
                    o += s
                out[name] = segs
        off += sz
    return out


def symbols(path: str, kext: str):
    """Symboles définis (nlist_64, N_SECT) d'une entrée fileset : [(adresse, nom mangé)]."""
    d = open(path, "rb").read()
    ncmds = struct.unpack_from("<I", d, 16)[0]
    off = 32
    for _ in range(ncmds):
        cmd, sz = struct.unpack_from("<II", d, off)
        if cmd == LC_FILESET_ENTRY:
            vmaddr, fileoff, stroff = struct.unpack_from("<QQI", d, off + 8)
            if d[off + stroff : d.index(b"\0", off + stroff)].decode() == kext:
                n2 = struct.unpack_from("<I", d, fileoff + 16)[0]
                o = fileoff + 32
                for _ in range(n2):
                    c, s = struct.unpack_from("<II", d, o)
                    if c == 0x2:  # LC_SYMTAB : offsets relatifs au fichier KC entier
                        symoff, nsyms, stroff2, _strsize = struct.unpack_from("<IIII", d, o + 8)
                        out = []
                        for i in range(nsyms):
                            nstrx, ntype, _sect, _desc, value = struct.unpack_from("<IBBHQ", d, symoff + 16 * i)
                            if ntype & 0x0E == 0x0E:
                                name = d[stroff2 + nstrx : d.index(b"\0", stroff2 + nstrx)].decode(errors="replace")
                                out.append((value, name))
                        return sorted(out)
                    o += s
        off += sz
    return []


if __name__ == "__main__":
    if len(sys.argv) >= 4 and sys.argv[1] == "symbols":
        for a, n in symbols(sys.argv[2], sys.argv[3]):
            print(f"{a:#018x} {n}")
        sys.exit(0)
    if len(sys.argv) >= 4 and sys.argv[1] == "decode":
        decode(sys.argv[2], sys.argv[3])
    elif len(sys.argv) >= 3 and sys.argv[1] == "entries":
        entries(sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else "")
    else:
        sys.exit(__doc__)
