#!/usr/bin/env python3
"""Importe des images du cache dyld (DyldCacheExtractLoader) ou des Mach-O autonomes
dans UN projet Ghidra par binaire, puis lance l'analyse complète (ObjC + constantes).

Usage :
  01_import_analyze.py --projects DIR --fileset kc.macho --entry com.apple.driver.IOBluetoothHIDDriver
  01_import_analyze.py --projects DIR --dsc CACHE --image /System/Library/Frameworks/IOBluetooth.framework/Versions/A/IOBluetooth
  01_import_analyze.py --projects DIR --bin bluetoothd.arm64e

Option du loader : addLibobjc (fusionne libobjc pour résoudre les sélecteurs), addOptionalComponents
non utilisée (ajoute les dépendances, trop lourd).
"""
import argparse
import os
import time

import akm_ghidra


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--projects", required=True)
    ap.add_argument("--dsc")
    ap.add_argument("--image", action="append", default=[])
    ap.add_argument("--fileset", help="kernelcache MH_FILESET décompressé (IM4P/LZFSE retiré)")
    ap.add_argument("--entry", action="append", default=[], help="entrée du fileset, ex. com.apple.driver.IOBluetoothHIDDriver")
    ap.add_argument("--list", action="store_true", help="liste les entrées du conteneur et sort")
    ap.add_argument("--bin", action="append", default=[])
    ap.add_argument("--no-analyze", action="store_true")
    a = ap.parse_args()

    pg = akm_ghidra.start()
    from java.io import File
    from ghidra.formats.gfilesystem import FileSystemService

    jobs = []
    if a.image:
        fs = pg.open_filesystem(a.dsc)
        for p in a.image:
            f = fs.lookup(p)
            if f is None:
                akm_ghidra.log(f"ABSENT {p}")
                continue
            jobs.append((os.path.basename(p), f.getFSRL(), "DyldCacheExtractLoader"))
    if a.fileset:
        fs = pg.open_filesystem(a.fileset)
        if a.list:
            for f in fs.getListing(None):
                akm_ghidra.log(f"ENTREE {f.getPath()}")
            return
        for e in a.entry:
            f = fs.lookup("/" + e) or fs.lookup(e)
            if f is None:
                akm_ghidra.log(f"ABSENT {e}")
                continue
            jobs.append((e.split(".")[-1], f.getFSRL(), "MachoFileSetExtractLoader"))
    for b in a.bin:
        fsrl = FileSystemService.getInstance().getLocalFSRL(File(os.path.abspath(b)))
        jobs.append((os.path.basename(b).replace(".arm64e", ""), fsrl, "MachoLoader"))

    os.makedirs(a.projects, exist_ok=True)
    for name, fsrl, loader in jobs:
        t0 = time.time()
        proj = pg.open_project(a.projects, f"akm_{name}", create=True)
        try:
            path = f"/{name}"
            if proj.getProjectData().getFile(path) is None:
                from ghidra.app.util.importer import MessageLog
                mlog = MessageLog()
                b = pg.program_loader().source(fsrl).project(proj).projectFolderPath("/") \
                    .name(name).loaders(loader).log(mlog)
                if loader == "DyldCacheExtractLoader":
                    b = b.addLoaderArg("Add libobjc.dylib", "true")
                with b.load() as res:
                    res.save(pg.task_monitor())
                akm_ghidra.log(str(mlog.toString())[:1500])
                akm_ghidra.log(f"IMPORT {name} ok ({time.time()-t0:.0f}s)")
            if a.no_analyze:
                continue
            with pg.program_context(proj, path) as prog:
                applied, allnames = akm_ghidra.set_analysis_options(prog)
                akm_ghidra.log(f"OPTIONS {name} {applied}")
                akm_ghidra.log("ANALYSEURS objc/const: " + str([n for n in allnames if "bjective" in n or "onstant" in n]))
                logtxt = pg.analyze(prog)
                prog.save("akm analyse", pg.task_monitor())
                fm = prog.getFunctionManager()
                akm_ghidra.log(f"ANALYSE {name} fonctions={fm.getFunctionCount()} "
                               f"({time.time()-t0:.0f}s) log={len(logtxt)}o")
        finally:
            proj.close()


if __name__ == "__main__":
    main()
