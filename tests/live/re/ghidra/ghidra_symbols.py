#!/usr/bin/env python3
"""Applique au projet Ghidra les tables de symboles LC_SYMTAB de TOUTES les entrées d'un KC fileset
(le chargeur Mach-O de Ghidra 12.1.2 n'en applique aucune pour un MH_FILESET), démangle les noms C++
(`__ZN…` → `Classe::méthode`) et crée les fonctions manquantes aux symboles de code.

  python ghidra_symbols.py <kc.macho> <dossier_projet> <nom_projet>
"""
import os
import struct
import sys

import pyghidra

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from kc_extract import LC_FILESET_ENTRY, symbols  # noqa: E402

KC, PDIR, PNAME = sys.argv[1], os.path.abspath(sys.argv[2]), sys.argv[3]

launcher = pyghidra.HeadlessPyGhidraLauncher(install_dir=os.environ.get("GHIDRA_INSTALL_DIR", "/opt/ghidra"))
launcher.add_vmargs(os.environ.get("GHIDRA_XMX", "-Xmx14g"))
launcher.start()

from ghidra.base.project import GhidraProject  # noqa: E402
from ghidra.program.model.symbol import SourceType  # noqa: E402
from ghidra.app.cmd.label import DemanglerCmd  # noqa: E402
from ghidra.app.cmd.function import CreateFunctionCmd  # noqa: E402
from ghidra.util.task import ConsoleTaskMonitor  # noqa: E402

mon = ConsoleTaskMonitor()
proj = GhidraProject.openProject(PDIR, PNAME, False)
prog = proj.openProgram("/", os.path.basename(KC), False)
space = prog.getAddressFactory().getDefaultAddressSpace()
st, fm, mem = prog.getSymbolTable(), prog.getFunctionManager(), prog.getMemory()

d = open(KC, "rb").read()
names = []
ncmds = struct.unpack_from("<I", d, 16)[0]
off = 32
for _ in range(ncmds):
    cmd, sz = struct.unpack_from("<II", d, off)
    if cmd == LC_FILESET_ENTRY:
        so = struct.unpack_from("<I", d, off + 24)[0]
        names.append(d[off + so : d.index(b"\0", off + so)].decode())
    off += sz
del d

tx = prog.startTransaction("symboles fileset")
nl = nd = nf = 0
for k in names:
    for a, n in symbols(KC, k):
        addr = space.getAddress(f"{a:#x}")
        if not mem.contains(addr):
            continue
        st.createLabel(addr, n, SourceType.IMPORTED)
        nl += 1
        blk = mem.getBlock(addr)
        if blk.isExecute() and fm.getFunctionAt(addr) is None and not n.startswith(("__ZZ", "ltmp", "l_")):
            if CreateFunctionCmd(addr).applyTo(prog):
                nf += 1
        if n.startswith("__Z"):
            if DemanglerCmd(addr, n[1:]).applyTo(prog, mon):
                nd += 1
prog.endTransaction(tx, True)
print(f"{len(names)} entrées, {nl} symboles posés, {nd} démanglés, {nf} fonctions créées", flush=True)
proj.save(prog)
proj.close()
