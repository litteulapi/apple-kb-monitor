#!/usr/bin/env python3
"""Corrige les fausses références d'appel posées par la propagation de constantes sur les appels
indirects PAC (`blr`, `blraa`, `blraaz`, `blrab`…) : Ghidra 12 y recopie la cible du `bl` précédent
(ex. `-[BluetoothHIDDevice hidDeviceInterface]`), si bien que le pseudo-C perd les arguments de
IOHIDDeviceInterface::setReport (+0x90) / getReport (+0x98).

Supprime les références de flux non externes portées par ces instructions, puis ré-enregistre.
  05_fix_vcalls.py --projects DIR --prog IOBluetooth
"""
import argparse

import akm_ghidra


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--projects", required=True)
    ap.add_argument("--prog", required=True)
    a = ap.parse_args()
    pg = akm_ghidra.start()
    proj = pg.open_project(a.projects, f"akm_{a.prog}")
    try:
        with pg.program_context(proj, f"/{a.prog}") as prog:
            rm = prog.getReferenceManager()
            n = 0
            with pg.transaction(prog, "akm fix vcalls"):
                for i in prog.getListing().getInstructions(True):
                    if not i.getMnemonicString().startswith("blr"):
                        continue
                    for r in i.getReferencesFrom():
                        if r.getReferenceType().isFlow() and not r.isExternalReference():
                            rm.delete(r)
                            n += 1
            prog.save("akm fix vcalls", pg.task_monitor())
            akm_ghidra.log(f"{a.prog}: {n} fausses références d'appel indirect supprimées")
    finally:
        proj.close()


if __name__ == "__main__":
    main()
