#!/usr/bin/env python3
"""Exploite un projet Ghidra créé par ghidra_import.py : inventaire des fonctions des kexts visés,
décompilation (LOCALE, dossier de travail, jamais commitée), balayage des constantes immédiates
(IDs de rapport, PID) et des références aux chaînes.

  python ghidra_dump.py <kc.macho> <dossier_projet> <nom_projet> <sortie> <regex_decomp> <kext>...

Sorties dans <sortie> : funcs.tsv, scalars.tsv, strings.tsv, decomp/<kext>/<fonction>.c
"""
import os
import re
import sys

import pyghidra

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from kc_extract import fileset_ranges  # noqa: E402

KC, PDIR, PNAME, OUT, DRX, KEXTS = sys.argv[1], os.path.abspath(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5], sys.argv[6:]

# Constantes recherchées : IDs de rapport inconnus/connus, PID de la famille, commandes HID_CONTROL.
REPORT_IDS = {0xD0, 0xD1, 0xD4, 0xD5, 0xD8, 0xFA, 0xFB, 0x4B, 0xF6, 0xF7, 0x09, 0x13, 0x30, 0x40, 0x41, 0x43, 0x44,
              0x45, 0x47, 0x49, 0x4A, 0x4E, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x60, 0xC6, 0xDC, 0xF0, 0xF2, 0xFE}
PIDS = {0x255, 0x256, 0x257, 0x239, 0x23A, 0x23B, 0x22C, 0x22D, 0x22E, 0x208, 0x209, 0x20A, 0x137}
TIMERS = {1000, 3500, 5000, 10000, 60000, 3600000, 14400000, 1300}
SMALL = {0x04, 0x05}  # seulement sur les comparaisons (trop fréquents sinon)

launcher = pyghidra.HeadlessPyGhidraLauncher(install_dir=os.environ.get("GHIDRA_INSTALL_DIR", "/opt/ghidra"))
launcher.add_vmargs(os.environ.get("GHIDRA_XMX", "-Xmx14g"))
launcher.start()

from ghidra.base.project import GhidraProject  # noqa: E402
from ghidra.app.decompiler import DecompInterface  # noqa: E402
from ghidra.util.task import ConsoleTaskMonitor  # noqa: E402
from ghidra.program.model.address import AddressSet  # noqa: E402
from ghidra.program.model.scalar import Scalar  # noqa: E402

mon = ConsoleTaskMonitor()
proj = GhidraProject.openProject(PDIR, PNAME, True)
prog = proj.openProgram("/", os.path.basename(KC), True)
space = prog.getAddressFactory().getDefaultAddressSpace()
fm, listing, refm = prog.getFunctionManager(), prog.getListing(), prog.getReferenceManager()
os.makedirs(OUT, exist_ok=True)

ranges = fileset_ranges(KC, KEXTS)
deci = DecompInterface()
deci.openProgram(prog)
drx = re.compile(DRX)
CMP = re.compile(r"^(cmp|cmn|ccmp|ccmn|subs|test|sub)", re.I)

ftsv = open(os.path.join(OUT, "funcs.tsv"), "w")
stsv = open(os.path.join(OUT, "scalars.tsv"), "w")
xtsv = open(os.path.join(OUT, "strings.tsv"), "w")
for kext, segs in ranges.items():
    aset = AddressSet()
    for _seg, a, n in segs:
        aset.add(space.getAddress(f"{a:#x}"), space.getAddress(f"{a + n - 1:#x}"))
    nf = nd = 0
    for f in fm.getFunctions(aset, True):
        nf += 1
        name = f.getName(True)
        ftsv.write(f"{kext}\t{f.getEntryPoint()}\t{f.getBody().getNumAddresses()}\t{name}\n")
        for ins in listing.getInstructions(f.getBody(), True):
            m = ins.getMnemonicString()
            for i in range(ins.getNumOperands()):
                for o in ins.getOpObjects(i):
                    if isinstance(o, Scalar):
                        v = o.getUnsignedValue()
                        if v in REPORT_IDS or v in PIDS or v in TIMERS or (v in SMALL and CMP.match(m)):
                            stsv.write(f"{kext}\t{name}\t{ins.getAddress()}\t{v:#x}\t{ins}\n")
            for r in ins.getReferencesFrom():
                d = listing.getDataAt(r.getToAddress())
                if d is not None and d.hasStringValue():
                    s = str(d.getValue()).replace("\n", "\\n").replace("\t", " ")[:160]
                    xtsv.write(f"{kext}\t{name}\t{ins.getAddress()}\t{s}\n")
        if drx.search(name) or drx.search(kext):
            res = deci.decompileFunction(f, 90, mon)
            if res.decompileCompleted():
                safe = re.sub(r"[^A-Za-z0-9_.~-]+", "_", name)[:150]
                dd = os.path.join(OUT, "decomp", kext)
                os.makedirs(dd, exist_ok=True)
                with open(os.path.join(dd, f"{safe}@{f.getEntryPoint()}.c"), "w") as fh:
                    fh.write(res.getDecompiledFunction().getC())
                nd += 1
    print(f"{kext}: {nf} fonctions, {nd} décompilées", flush=True)
for fh in (ftsv, stsv, xtsv):
    fh.close()

# Vtables C++ (symboles __ZTV<classe>) : offset depuis le point d'adresse de l'objet (vtable + 0x10) → méthode.
# Sert à nommer les appels virtuels `(*(*this + off))(...)` du pseudo-C.
VT = os.environ.get("VTABLES", r"IOBluetoothHIDDriver|IOAppleBluetoothHIDDriver|AppleBluetoothHIDKeyboard|"
                    r"IOHIDDevice|IOBluetoothL2CAPChannel|IOBluetoothDevice|IOHIDEventService|AppleEmbeddedKeyboard|"
                    r"IOHIDInterface|IOBluetoothHostController|IOWorkLoop|IOCommandGate|IOTimerEventSource|IOService")
vrx = re.compile(r"^__ZTV\d+(" + VT + r")$")
ptr = prog.getDefaultPointerSize()
with open(os.path.join(OUT, "vtables.tsv"), "w") as vt:
    for s in prog.getSymbolTable().getAllSymbols(False):
        n = s.getName()
        if not n.startswith("__ZTV") or not vrx.match(n):
            continue
        base = s.getAddress().add(2 * ptr)
        for i in range(0, 900):
            a = base.add(i * ptr)
            try:
                v = prog.getMemory().getLong(a)
            except Exception:
                break
            t = space.getAddress(f"{v & 0xFFFFFFFFFFFFFFFF:#x}") if v else None
            f = fm.getFunctionAt(t) if t is not None else None
            if f is None:
                if i > 8:
                    break
                continue
            vt.write(f"{n}\t{i * ptr:#x}\t{f.getName(True)}\n")
proj.close()
