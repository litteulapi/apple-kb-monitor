"""Outils communs PyGhidra pour la RE des pilotes Bluetooth HID de macOS (A1314).

Aucun binaire Apple n'est lu depuis le dépôt : les chemins sont passés en argument
(cache dyld et démons copiés hors dépôt). Les sorties (pseudo-C, listes) vont dans
un dossier de travail hors dépôt ; seuls des faits courts sont reportés dans docs/.

Variables d'environnement :
  GHIDRA_INSTALL_DIR  (défaut /opt/ghidra)
  AKM_GHIDRA_XMX      (défaut 12g)
"""
from __future__ import annotations

import os
import sys

GHIDRA_DIR = os.environ.get("GHIDRA_INSTALL_DIR", "/opt/ghidra")


def start():
    import pyghidra
    if pyghidra.started():
        return pyghidra
    launcher = pyghidra.HeadlessPyGhidraLauncher(install_dir=GHIDRA_DIR)
    launcher.add_vmargs(f"-Xmx{os.environ.get('AKM_GHIDRA_XMX', '12g')}")
    launcher.start()
    return pyghidra


def set_analysis_options(program, decompiler_param_id: bool = False):
    """Active l'ObjC et la propagation de constantes ; coupe ce qui est inutile et lent."""
    import pyghidra
    opts = pyghidra.analysis_properties(program)
    wanted = {
        "Objective-C 2 Class": True,
        "Objective-C 2 Decompiler Message": True,
        "Objective-C Type Metadata Analyzer": True,
        "Objective-C Message": True,
        "Basic Constant Reference Analyzer": True,
        "AARCH64 Constant Reference Analyzer": True,
        "Decompiler Switch Analysis": True,
        "Mach-O Function Starts": True,
        "Decompiler Parameter ID": decompiler_param_id,
        "Aggressive Instruction Finder": False,
        "Function ID": False,
        "Library Identification": False,
        "Variadic Function Signature Override": False,
    }
    names = set(opts.getOptionNames())
    applied = {}
    with pyghidra.transaction(program, "akm options"):
        for k, v in wanted.items():
            if k in names:
                opts.setBoolean(k, v)
                applied[k] = v
    return applied, sorted(n for n in names if "." not in n)


def log(msg: str):
    print(msg, flush=True)
    sys.stdout.flush()
