#!/usr/bin/env python3
"""Minimal HID report descriptor parser (read-only, no device access).

Usage:
    hid_rdesc.py <descriptor.hex|descriptor.bin|/sys/.../report_descriptor>

Prints, per report ID and type (input/output/feature), the bit layout and
the 32-bit usages (page << 16 | id) of every field. Used by
test_entrees_modeles.py to check what each Apple keyboard family exposes
on the input side, without hardware. Only reads files: never opens hidraw.
"""
import sys
from collections import defaultdict
from pathlib import Path

MAIN_INPUT, MAIN_OUTPUT, MAIN_FEATURE = 0x8, 0x9, 0xB
KIND = {MAIN_INPUT: "input", MAIN_OUTPUT: "output", MAIN_FEATURE: "feature"}


class Field:
    """One main item: `count` x `size` bits, with its usages."""

    def __init__(self, kind, report_id, bit_offset, size, count, flags, usages):
        self.kind = kind
        self.report_id = report_id
        self.bit_offset = bit_offset
        self.size = size
        self.count = count
        self.flags = flags
        self.usages = usages

    @property
    def constant(self):
        return bool(self.flags & 0x01)

    @property
    def variable(self):
        return bool(self.flags & 0x02)

    def __repr__(self):
        u = ",".join(f"{x:08x}" for x in self.usages)
        return (f"<{self.kind} id=0x{self.report_id:02x} bit={self.bit_offset} "
                f"{self.count}x{self.size} {'const' if self.constant else 'data'} [{u}]>")


def load(path):
    """Descriptor bytes from a .hex (whitespace tolerated) or binary file."""
    raw = Path(path).read_bytes()
    try:
        txt = raw.decode("ascii")
        clean = "".join(txt.split())
        if clean and all(c in "0123456789abcdefABCDEF" for c in clean) and len(clean) % 2 == 0:
            return bytes.fromhex(clean)
    except UnicodeDecodeError:
        pass
    return raw


def parse(desc):
    """Return the list of Field of a report descriptor (short items only)."""
    fields = []
    g = {"page": 0, "size": 0, "count": 0, "rid": 0}
    stack = []
    local = {"usages": [], "min": None, "max": None}
    offsets = defaultdict(int)  # (kind, rid) -> next bit
    i = 0
    while i < len(desc):
        b = desc[i]
        if b == 0xFE:  # long item: skip
            i += 3 + desc[i + 1]
            continue
        size = (0, 1, 2, 4)[b & 0x03]
        typ = (b >> 2) & 0x03
        tag = b >> 4
        data = int.from_bytes(desc[i + 1:i + 1 + size], "little")
        i += 1 + size
        if typ == 0:  # main
            if tag in KIND:
                usages = list(local["usages"])
                if local["min"] is not None and local["max"] is not None:
                    usages += list(range(local["min"], local["max"] + 1))
                kind = KIND[tag]
                key = (kind, g["rid"])
                fields.append(Field(kind, g["rid"], offsets[key], g["size"], g["count"], data, usages))
                offsets[key] += g["size"] * g["count"]
            local = {"usages": [], "min": None, "max": None}
        elif typ == 1:  # global
            if tag == 0x0:
                g["page"] = data
            elif tag == 0x7:
                g["size"] = data
            elif tag == 0x8:
                g["rid"] = data
            elif tag == 0x9:
                g["count"] = data
            elif tag == 0xA:
                stack.append(dict(g))
            elif tag == 0xB and stack:
                g = stack.pop()
        elif typ == 2:  # local: a 4-byte usage carries its own page
            full = data if size == 4 else (g["page"] << 16) | data
            if tag == 0x0:
                local["usages"].append(full)
            elif tag == 0x1:
                local["min"] = full
            elif tag == 0x2:
                local["max"] = full
    return fields


def reports(fields):
    """{(kind, report_id): total_bits}"""
    out = defaultdict(int)
    for f in fields:
        out[(f.kind, f.report_id)] += f.size * f.count
    return dict(out)


def usage_bits(fields, kind, report_id):
    """{usage: bit_offset} of the 1-bit variable data fields of one report."""
    out = {}
    for f in fields:
        if f.kind != kind or f.report_id != report_id or f.constant or not f.variable:
            continue
        for n in range(f.count):
            if n < len(f.usages):
                out[f.usages[n]] = f.bit_offset + n * f.size
            elif f.usages:  # HID rule: the last usage repeats
                out.setdefault(f.usages[-1], f.bit_offset + n * f.size)
    return out


def has_usage(fields, usage, kind=None):
    return any(usage in f.usages and not f.constant and (kind is None or f.kind == kind)
               for f in fields)


def main(argv):
    if len(argv) != 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    desc = load(argv[1])
    fields = parse(desc)
    print(f"{len(desc)} bytes, {len(fields)} main items")
    for (kind, rid), bits in sorted(reports(fields).items(), key=lambda x: (x[0][1], x[0][0])):
        print(f"report 0x{rid:02x} {kind:7s} {bits:4d} bits ({(bits + 7) // 8} bytes)")
        for f in fields:
            if f.kind == kind and f.report_id == rid and not f.constant:
                shown = f.usages[:8]
                more = "…" if len(f.usages) > 8 else ""
                print(f"    bit {f.bit_offset:3d} {f.count}x{f.size} "
                      + ",".join(f"{u:08x}" for u in shown) + more)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
