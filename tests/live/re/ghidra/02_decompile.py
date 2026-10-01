#!/usr/bin/env python3
"""Décompile (pseudo-C Ghidra) les fonctions d'un programme analysé, vers un dossier HORS dépôt.

Sélection :
  --all                       toutes les fonctions (images moyennes : IOBluetooth, BluetoothServices…)
  --name REGEX                fonctions dont le nom complet (namespace::nom) correspond
  --string REGEX              fonctions qui référencent une chaîne correspondante (+ --callees N niveaux)
  --addr 0x...                fonctions contenant ces adresses
Sorties : <out>/<prog>.c (blocs « //== FUNC <nom> @ <adresse> ») et <out>/<prog>.idx (adresse\tnom\ttaille).
Ne jamais commiter ces sorties : elles contiennent du code Apple décompilé.
"""
import argparse
import os
import re
import concurrent.futures as cf

import akm_ghidra


def collect(prog, a):
    fm = prog.getFunctionManager()
    sel = {}
    if a.all:
        for f in fm.getFunctions(True):
            if not f.isThunk() and not f.isExternal():
                sel[f.getEntryPoint().getOffset()] = f
    if a.name:
        rx = re.compile(a.name)
        for f in fm.getFunctions(True):
            if rx.search(f.getName(True)):
                sel[f.getEntryPoint().getOffset()] = f
    if a.string:
        rx = re.compile(a.string)
        from ghidra.program.util import DefinedStringIterator
        rm = prog.getReferenceManager()
        for d in DefinedStringIterator.forProgram(prog):
            try:
                v = str(d.getValue())
            except Exception:
                continue
            if not rx.search(v):
                continue
            for r in rm.getReferencesTo(d.getAddress()):
                f = fm.getFunctionContaining(r.getFromAddress())
                if f is not None:
                    sel[f.getEntryPoint().getOffset()] = f
                else:  # chaîne référencée par un CFString : remonter d'un cran
                    for r2 in rm.getReferencesTo(r.getFromAddress()):
                        f2 = fm.getFunctionContaining(r2.getFromAddress())
                        if f2 is not None:
                            sel[f2.getEntryPoint().getOffset()] = f2
    for x in a.addr:
        f = fm.getFunctionContaining(prog.getAddressFactory().getAddress(x))
        if f is not None:
            sel[f.getEntryPoint().getOffset()] = f
    from ghidra.util.task import TaskMonitor
    frontier = list(sel.values())
    for _ in range(a.callees):
        nxt = []
        for f in frontier:
            for c in f.getCalledFunctions(TaskMonitor.DUMMY):
                if c.isExternal() or c.isThunk():
                    continue
                k = c.getEntryPoint().getOffset()
                if k not in sel:
                    sel[k] = c
                    nxt.append(c)
        frontier = nxt
    return [sel[k] for k in sorted(sel)]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--projects", required=True)
    ap.add_argument("--prog", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--name")
    ap.add_argument("--string")
    ap.add_argument("--addr", action="append", default=[])
    ap.add_argument("--callees", type=int, default=0)
    ap.add_argument("--threads", type=int, default=6)
    ap.add_argument("--timeout", type=int, default=60)
    ap.add_argument("--suffix", default="")
    a = ap.parse_args()

    pg = akm_ghidra.start()
    from ghidra.app.decompiler import DecompInterface, DecompileOptions
    from ghidra.util.task import TaskMonitor

    os.makedirs(a.out, exist_ok=True)
    proj = pg.open_project(a.projects, f"akm_{a.prog}")
    try:
        with pg.program_context(proj, f"/{a.prog}") as prog:
            funcs = collect(prog, a)
            akm_ghidra.log(f"{a.prog}: {len(funcs)} fonctions à décompiler")
            chunks = [funcs[i::a.threads] for i in range(a.threads)]

            def work(chunk):
                di = DecompInterface()
                opt = DecompileOptions()
                opt.grabFromProgram(prog)
                di.setOptions(opt)
                di.toggleCCode(True)
                di.toggleSyntaxTree(False)
                di.openProgram(prog)
                res = []
                for f in chunk:
                    r = di.decompileFunction(f, a.timeout, TaskMonitor.DUMMY)
                    c = r.getDecompiledFunction().getC() if r and r.decompileCompleted() else f"/* ECHEC: {r.getErrorMessage() if r else '?'} */"
                    res.append((f.getEntryPoint().getOffset() & 0xffffffffffffffff, f.getName(True), f.getBody().getNumAddresses(), str(c)))
                di.dispose()
                return res

            out = []
            with cf.ThreadPoolExecutor(a.threads) as ex:
                for r in ex.map(work, chunks):
                    out.extend(r)
            out.sort()
            base = os.path.join(a.out, a.prog + a.suffix)
            with open(base + ".c", "w") as fc, open(base + ".idx", "w") as fi:
                for off, name, size, c in out:
                    fc.write(f"//== FUNC {name} @ {off:#x}\n{c}\n")
                    fi.write(f"{off & 0xffffffffffffffff:#x}\t{name}\t{size}\n")
            akm_ghidra.log(f"ECRIT {base}.c ({len(out)} fonctions)")
    finally:
        proj.close()


if __name__ == "__main__":
    main()
