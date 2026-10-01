#!/usr/bin/env python3
"""Décompile (sortie LOCALE, jamais commitée) les fonctions d'un projet Ghidra dont le nom complet
correspond à une regex, et liste leurs appelants. Complète ghidra_dump.py pour les fonctions hors kexts visés.

  python ghidra_decomp.py <kc.macho> <dossier_projet> <nom_projet> <sortie> <regex>
"""
import os
import re
import sys

import pyghidra

KC, PDIR, PNAME, OUT, RX = sys.argv[1], os.path.abspath(sys.argv[2]), sys.argv[3], sys.argv[4], re.compile(sys.argv[5])

launcher = pyghidra.HeadlessPyGhidraLauncher(install_dir=os.environ.get("GHIDRA_INSTALL_DIR", "/opt/ghidra"))
launcher.add_vmargs(os.environ.get("GHIDRA_XMX", "-Xmx14g"))
launcher.start()

from ghidra.base.project import GhidraProject  # noqa: E402
from ghidra.app.decompiler import DecompInterface  # noqa: E402
from ghidra.util.task import ConsoleTaskMonitor  # noqa: E402

mon = ConsoleTaskMonitor()
proj = GhidraProject.openProject(PDIR, PNAME, True)
prog = proj.openProgram("/", os.path.basename(KC), True)
deci = DecompInterface()
deci.openProgram(prog)
os.makedirs(OUT, exist_ok=True)
with open(os.path.join(OUT, "callers.tsv"), "a") as ct:
    for f in prog.getFunctionManager().getFunctions(True):
        name = f.getName(True)
        if not RX.search(name):
            continue
        callers = sorted({c.getName(True) for c in f.getCallingFunctions(mon)})
        ct.write(f"{name}\t{f.getEntryPoint()}\t{'; '.join(callers)}\n")
        res = deci.decompileFunction(f, 90, mon)
        if res.decompileCompleted():
            safe = re.sub(r"[^A-Za-z0-9_.~-]+", "_", name)[:150]
            with open(os.path.join(OUT, f"{safe}@{f.getEntryPoint()}.c"), "w") as fh:
                fh.write(res.getDecompiledFunction().getC())
        print(name, f.getEntryPoint(), len(callers), flush=True)
proj.close()
