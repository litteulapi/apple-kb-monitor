#!/usr/bin/env python3
"""Importe une collection de noyau Apple (Mach-O MH_FILESET, arm64e ou x86_64) dans un projet Ghidra,
puis lance l'analyse automatique **limitée** aux kexts demandés (le reste du KC sert à résoudre
les appels inter-kexts et les symboles du noyau).

Lecture seule, interopérabilité (docs/RE-GHIDRA-KEXT.md). Projet Ghidra et binaires : dossier de travail,
jamais dans le dépôt.

  GHIDRA_INSTALL_DIR=/opt/ghidra JAVA_HOME=/usr/lib/jvm/java-21-openjdk \
  python ghidra_import.py <kc.macho> <dossier_projet> <nom_projet> <kext_id> [<kext_id>...]
"""
import os
import sys
import time

import pyghidra

KC, PDIR, PNAME, KEXTS = sys.argv[1], os.path.abspath(sys.argv[2]), sys.argv[3], sys.argv[4:]

launcher = pyghidra.HeadlessPyGhidraLauncher(install_dir=os.environ.get("GHIDRA_INSTALL_DIR", "/opt/ghidra"))
launcher.add_vmargs(os.environ.get("GHIDRA_XMX", "-Xmx14g"))
launcher.start()

from java.io import File  # noqa: E402
from ghidra.base.project import GhidraProject  # noqa: E402
from ghidra.app.util.importer import ProgramLoader  # noqa: E402
from ghidra.util.task import ConsoleTaskMonitor  # noqa: E402
from ghidra.program.model.address import AddressSet  # noqa: E402
from ghidra.app.plugin.core.analysis import AutoAnalysisManager  # noqa: E402
from ghidra.program.util import GhidraProgramUtilities  # noqa: E402

mon = ConsoleTaskMonitor()
os.makedirs(PDIR, exist_ok=True)
t0 = time.time()
if os.path.exists(os.path.join(PDIR, PNAME + ".gpr")):
    # Reprise : l'import (1 à 2 min) est déjà sauvegardé, on ne relance que l'analyse.
    proj = GhidraProject.openProject(PDIR, PNAME, False)
    prog = proj.openProgram("/", os.path.basename(KC), False)
else:
    proj = GhidraProject.createProject(PDIR, PNAME, False)
    res = ProgramLoader.builder().source(File(KC)).project(proj.getProject()).projectFolderPath("/").monitor(mon).load()
    res.save(mon)
    prog = res.getPrimaryDomainObject(proj)
print(f"import: {time.time() - t0:.0f} s, {prog.getMemory().getNumAddresses()} adresses", flush=True)

# Plages des kexts demandés : segments (hors __LINKEDIT) de chaque entrée LC_FILESET_ENTRY.
# Ghidra nomme les blocs par section seulement ; on relit donc les en-têtes Mach-O des entrées.
import struct  # noqa: E402

raw = open(KC, "rb").read()


def segments_of(fileoff):
    ncmds = struct.unpack_from("<I", raw, fileoff + 16)[0]
    o = fileoff + 32
    for _ in range(ncmds):
        cmd, sz = struct.unpack_from("<II", raw, o)
        if cmd == 0x19:
            seg = raw[o + 8 : o + 24].split(b"\0")[0].decode()
            vmaddr, vmsize = struct.unpack_from("<QQ", raw, o + 24)
            if seg != "__LINKEDIT" and vmsize:
                yield seg, vmaddr, vmsize
        o += sz


space = prog.getAddressFactory().getDefaultAddressSpace()
aset = AddressSet()
ncmds = struct.unpack_from("<I", raw, 16)[0]
o = 32
for _ in range(ncmds):
    cmd, sz = struct.unpack_from("<II", raw, o)
    if cmd == 0x80000035:
        vmaddr, fileoff, stroff = struct.unpack_from("<QQI", raw, o + 8)
        name = raw[o + stroff : raw.index(b"\0", o + stroff)].decode()
        if name in KEXTS:
            for seg, a, n in segments_of(fileoff):
                aset.add(space.getAddress(f"{a:#x}"), space.getAddress(f"{a + n - 1:#x}"))
                print(f"  {name} {seg} {a:#x}+{n:#x}")
    o += sz
if aset.isEmpty():
    sys.exit("aucune entrée fileset ne correspond aux kexts demandés")

tx = prog.startTransaction("analyse limitée")
# Analyseurs trop coûteux sur un KC de 114 Mo (réparation de flot en boucle sur le noyau entier) :
# la recherche des fonctions sans retour est désactivée ; les noms du noyau (panic…) suffisent.
mgr0 = AutoAnalysisManager.getAnalysisManager(prog)
mgr0.initializeOptions()  # enregistre les options d'analyse (sinon absentes au premier import)
opts = prog.getOptions("Analyzers")
for a in ("Non-Returning Functions - Discovered", "Decompiler Parameter ID", "Embedded Media",
          "Apply Data Archives", "Variadic Function Signature Override"):
    if opts.contains(a):
        opts.setBoolean(a, False)
        print(f"  analyseur désactivé : {a}", flush=True)
mgr = AutoAnalysisManager.getAnalysisManager(prog)
mgr.initializeOptions()
t0 = time.time()
mgr.reAnalyzeAll(aset)
mgr.startAnalysis(mon)
GhidraProgramUtilities.markProgramAnalyzed(prog)
prog.endTransaction(tx, True)
print(f"analyse: {time.time() - t0:.0f} s, {prog.getFunctionManager().getFunctionCount()} fonctions")
proj.save(prog)
proj.close()
