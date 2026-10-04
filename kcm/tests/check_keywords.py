"""Every language of the module's Name has search keywords (kcm_applekeyboard.json)."""
import json, re, sys

meta = json.load(open(sys.argv[1], encoding="utf-8"))
flat = {**meta, **meta.get("KPlugin", {})}
names = {m.group(1) for k in flat if (m := re.fullmatch(r"Name\[(\w+)\]", k))}
missing = sorted(l for l in names if not flat.get(f"X-KDE-Keywords[{l}]"))
if missing or not flat.get("X-KDE-Keywords"):
    sys.exit(f"X-KDE-Keywords missing for: {missing or ['(default)']}")
print(f"keywords in {len(names)} languages")
