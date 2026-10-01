#!/usr/bin/env python3
"""Résout des entrées de vtable C++ d'un kext (ex. IOBluetoothDevice) : nom de la méthode à un décalage donné.

  07_vtable_slot.py --projects DIR --prog IOBluetoothFamily --class IOBluetoothDevice --slot 0x988 --slot 0xa50

Cherche le symbole `__ZTV<len><Classe>` (vtable), saute l'en-tête de 16 octets (offset-to-top + RTTI),
lit le pointeur (chaîné arm64e : on garde les 43 bits bas + base du fileset) et donne la fonction visée.
"""
import argparse

import akm_ghidra


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--projects", required=True)
    ap.add_argument("--prog", required=True)
    ap.add_argument("--class", dest="cls", required=True)
    ap.add_argument("--slot", action="append", default=[])
    a = ap.parse_args()
    pg = akm_ghidra.start()
    proj = pg.open_project(a.projects, f"akm_{a.prog}")
    try:
        with pg.program_context(proj, f"/{a.prog}") as prog:
            st, mem, fm = prog.getSymbolTable(), prog.getMemory(), prog.getFunctionManager()
            mangled = f"__ZTV{len(a.cls)}{a.cls}"
            syms = [s for s in st.getSymbols(mangled)] or [s for s in st.getSymbols(f"_ZTV{len(a.cls)}{a.cls}")]
            if not syms:
                cands = [s.getName() for s in st.getSymbolIterator(True) if a.cls in s.getName() and "vtable" in s.getName().lower()]
                print("vtable introuvable ; candidats :", cands[:10])
                return
            base = syms[0].getAddress().add(16)
            print(f"vtable {a.cls} @ {syms[0].getAddress()}")
            for sl in a.slot:
                ad = base.add(int(sl, 16))
                raw = mem.getLong(ad) & 0xFFFFFFFFFFFFFFFF
                # pointeur déjà corrigé par Ghidra (référence) en priorité
                refs = list(prog.getReferenceManager().getReferencesFrom(ad))
                tgt = refs[0].getToAddress() if refs else None
                f = fm.getFunctionAt(tgt) if tgt else None
                print(f"+{sl}: brut={raw:#x} -> {tgt} {f.getName(True) if f else '?'}")
    finally:
        proj.close()


if __name__ == "__main__":
    main()
