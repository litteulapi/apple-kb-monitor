#!/usr/bin/env python3
"""Désassemblage Ghidra (listing) de fonctions choisies, avec références résolues.

Sert là où le décompilateur perd les arguments des appels virtuels signés PAC
(`ldr xN,[x8,#0x90]` / `blraa xN,x16` = IOHIDDeviceInterface::setReport, `#0x98` = getReport).

  04_disasm.py --projects DIR --prog IOBluetooth --name 'AppleBluetoothHIDDevice::deleteAllLinkKeys$' [--vcalls]
--vcalls : n'affiche que, pour chaque appel indirect par vtable +0x90/+0x98, les dernières écritures de
w1 (type), w2 (ID), x3 (tampon), w4/x4 (taille ou pointeur de taille), w5 (délai) avant l'appel.
"""
import argparse
import re

import akm_ghidra


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--projects", required=True)
    ap.add_argument("--prog", required=True)
    ap.add_argument("--name", action="append", default=[])
    ap.add_argument("--addr", action="append", default=[])
    ap.add_argument("--vcalls", action="store_true")
    ap.add_argument("--max", type=int, default=400)
    a = ap.parse_args()
    pg = akm_ghidra.start()
    proj = pg.open_project(a.projects, f"akm_{a.prog}")
    try:
        with pg.program_context(proj, f"/{a.prog}") as prog:
            fm, lst = prog.getFunctionManager(), prog.getListing()
            funcs = []
            rxs = [re.compile(r) for r in a.name]
            for f in fm.getFunctions(True):
                if any(r.search(f.getName(True)) for r in rxs):
                    funcs.append(f)
            for x in a.addr:
                f = fm.getFunctionContaining(prog.getAddressFactory().getAddress(x))
                if f:
                    funcs.append(f)
            for f in funcs:
                print(f"=== {f.getName(True)} @ {f.getEntryPoint()}")
                ins = list(lst.getInstructions(f.getBody(), True))
                if not a.vcalls:
                    for i in ins[: a.max]:
                        refs = [str(r.getToAddress()) for r in i.getReferencesFrom()]
                        sym = ""
                        for r in i.getReferencesFrom():
                            s = prog.getSymbolTable().getPrimarySymbol(r.getToAddress())
                            if s:
                                sym = s.getName(True)
                        print(f"{i.getAddress()}  {i}  {('; ' + sym) if sym else ''}")
                    continue
                for k, i in enumerate(ins):
                    t = str(i)
                    m = re.match(r"ldr (x\d+),\[x\d+, #0x(90|98)\]", t.replace(" ", "").replace("ldr", "ldr ", 1))
                    if not m:
                        continue
                    reg = m.group(1)
                    # trouver le blr* qui utilise ce registre
                    for j in range(k + 1, min(k + 30, len(ins))):
                        tj = str(ins[j])
                        if tj.startswith("blr") and reg in tj:
                            regs = {}
                            for i2 in ins[max(0, j - 40): j]:
                                s2 = str(i2)
                                mm = re.match(r"\w+ ([wx][1-6]),(.*)", s2)
                                if mm:
                                    regs[mm.group(1)[1:]] = f"{s2}"
                            kind = "setReport" if m.group(2) == "90" else "getReport"
                            print(f"  {ins[j].getAddress()} {kind}: " + " | ".join(regs.get(r, '?') for r in "123456"))
                            break
    finally:
        proj.close()


if __name__ == "__main__":
    main()
