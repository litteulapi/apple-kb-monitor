#!/usr/bin/env python3
"""Annote, dans le pseudo-C LOCAL produit par ghidra_dump.py, les appels virtuels sur `this`
(`(**(code **)(*param_1 + 0xa88))(…)`) avec le nom de la méthode lu dans vtables.tsv.

Le fichier .c est réécrit en place dans le dossier de travail (jamais commité).

  python annotate_vcalls.py <sortie_dump> <kext_id> [classe_par_défaut]

La classe est déduite du préfixe du nom de fichier (`Classe_méthode@adresse.c`).
"""
import glob
import os
import re
import sys

OUT, KEXT = sys.argv[1], sys.argv[2]
DEFAULT = sys.argv[3] if len(sys.argv) > 3 else None

vt = {}
for line in open(os.path.join(OUT, "vtables.tsv")):
    sym, off, name = line.rstrip("\n").split("\t")
    cls = re.sub(r"^__ZTV\d+", "", sym)
    vt.setdefault(cls, {})[int(off, 16)] = name

# this = param_1 (méthodes) ; motifs `*param_1 + 0x..` et `*(long *)param_1 + 0x..`
PAT = re.compile(r"\*(?:\(long \*\))?param_1 \+ (0x[0-9a-f]+)\)\)")
n = 0
for path in glob.glob(os.path.join(OUT, "decomp", KEXT, "*.c")):
    base = os.path.basename(path)
    cls = DEFAULT or (base.split("_")[0] if "_" in base else None)
    if cls not in vt:
        cls = DEFAULT
    if cls not in vt:
        continue
    table = vt[cls]
    src = open(path).read()

    def sub(m):
        off = int(m.group(1), 16)
        name = table.get(off)
        return m.group(0) + (f"/*{name}*/" if name else "")

    new = PAT.sub(sub, src)
    if new != src:
        open(path, "w").write(new)
        n += 1
print(f"{n} fichiers annotés")
