#!/usr/bin/env python3
"""The fake daemon's GetState only sends top-level fields that akm_core's Snapshot has."""
import ast
import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parent
snap = (root.parent.parent / "apihub-app/akm-core/src/snapshot.rs").read_text()
body = re.search(r"pub struct Snapshot \{(.*?)\n\}", snap, re.S).group(1)
fields = set(re.findall(r"^\s*pub (\w+):", body, re.M))
fake = ast.parse((root / "fake_services.py").read_text())
func = next(n for n in fake.body if isinstance(n, ast.FunctionDef) and n.name == "demo_state")
bad = sorted({k.value for d in ast.walk(func) if isinstance(d, ast.Return) and isinstance(d.value, ast.Dict)
              for k in d.value.keys if isinstance(k, ast.Constant)} - fields)
if not fields or bad:
    sys.exit(f"fake_services.py demo_state: fields not in Snapshot: {bad or 'Snapshot not parsed'}")
print("fake GetState matches Snapshot")
