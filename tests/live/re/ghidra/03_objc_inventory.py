#!/usr/bin/env python3
"""Inventaire Objective-C d'un programme analysé par Ghidra (classes, méthodes, xrefs de sélecteurs).

  03_objc_inventory.py --projects DIR --prog IOBluetooth --out OUTDIR
     [--class-rx 'HID|Device|Keyboard|Battery|Feature|Report|Firmware|Update|Pairing|Sleep|Suspend|LinkKey|FactoryDefault']
     [--sel-rx 'setReport|getReport|sendFeatureReport|sendControl|...']

Sorties (hors dépôt) :
  <prog>.classes.tsv  : classe \t nb méthodes \t méthodes (adresse:nom)
  <prog>.selxref.tsv  : sélecteur \t fonction appelante \t adresse de l'appel
Les méthodes ObjC sont les fonctions dont le namespace parent est une classe ObjC (analyseur
« Objective-C 2 Class »). Les xrefs de sélecteurs passent par __objc_selrefs (référence de données).
"""
import argparse
import os
import re
from collections import defaultdict

import akm_ghidra

CLASS_RX = r"HID|Device|Keyboard|Battery|Feature|Report|Firmware|Update|Pairing|Sleep|Suspend|LinkKey|FactoryDefault"
SEL_RX = (r"[sS]etReport|[gG]etReport|IOHIDDeviceSetReport|IOHIDDeviceGetReport|sendFeatureReport|sendControl|"
          r"HID_?CONTROL|hidControl|FeatureReport|setFeature|ExtendedFeature|factoryDefault|deleteAllLinkKeys|"
          r"recantConnection|sendSCO|connectionCount|LLR|ConnectionInterval|batteryPercent|batteryLow|"
          r"suspend|Suspend|userMode|UserMode|DeviceName|willShutdown|WillShutdown|Keyhole|VirtualCable")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--projects", required=True)
    ap.add_argument("--prog", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--class-rx", default=CLASS_RX)
    ap.add_argument("--sel-rx", default=SEL_RX)
    a = ap.parse_args()
    pg = akm_ghidra.start()
    os.makedirs(a.out, exist_ok=True)
    crx, srx = re.compile(a.class_rx), re.compile(a.sel_rx)
    proj = pg.open_project(a.projects, f"akm_{a.prog}")
    try:
        with pg.program_context(proj, f"/{a.prog}") as prog:
            fm = prog.getFunctionManager()
            rm = prog.getReferenceManager()
            mem = prog.getMemory()
            classes = defaultdict(list)
            for f in fm.getFunctions(True):
                ns = f.getParentNamespace()
                if ns is None or ns.isGlobal():
                    continue
                cname = ns.getName(True)
                if crx.search(cname):
                    classes[cname].append((f.getEntryPoint().getOffset(), f.getName()))
            with open(os.path.join(a.out, a.prog + ".classes.tsv"), "w") as fo:
                for c in sorted(classes):
                    ms = sorted(classes[c])
                    fo.write(f"{c}\t{len(ms)}\t" + " ".join(f"{o:#x}:{n}" for o, n in ms) + "\n")
            # Sélecteurs : chaînes dans __objc_methname, référencées depuis __objc_selrefs
            n = 0
            with open(os.path.join(a.out, a.prog + ".selxref.tsv"), "w") as fo:
                for blk in mem.getBlocks():
                    if "objc_methname" not in blk.getName():
                        continue
                    from ghidra.program.util import DefinedStringIterator
                    it = prog.getListing().getDefinedData(blk.getStart(), True)
                    while it.hasNext():
                        d = it.next()
                        if not blk.contains(d.getAddress()):
                            break
                        try:
                            s = str(d.getValue())
                        except Exception:
                            continue
                        if not srx.search(s):
                            continue
                        for r in rm.getReferencesTo(d.getAddress()):
                            src = r.getFromAddress()
                            f = fm.getFunctionContaining(src)
                            if f is not None:
                                fo.write(f"{s}\t{f.getName(True)}\t{src}\n"); n += 1
                                continue
                            for r2 in rm.getReferencesTo(src):  # selref -> code
                                f2 = fm.getFunctionContaining(r2.getFromAddress())
                                fo.write(f"{s}\t{f2.getName(True) if f2 else '?'}\t{r2.getFromAddress()}\n"); n += 1
            akm_ghidra.log(f"{a.prog}: {len(classes)} classes retenues, {n} xrefs de sélecteurs")
    finally:
        proj.close()


if __name__ == "__main__":
    main()
