#!/usr/bin/env python3
"""#205 : toute donnee affichee (nom, modele, firmware, erreurs) passe par un
Label/Heading en `textFormat: Text.PlainText`, et l'infobulle du plasmoide est
en texte brut. Garde statique : sinon le HTML d'un alias BlueZ serait rendu
par plasmashell (<img src=http://...> = requete reseau)."""
import re, sys, pathlib

ui = pathlib.Path(__file__).resolve().parent.parent / "com.agenceapi.devicehub/contents/ui"
bad = []
for f in sorted(ui.glob("*.qml")):
    stack = []
    for n, line in enumerate(f.read_text().split("\n"), 1):
        m = re.match(r"\s*([\w.]+)\s*\{\s*$", line)
        if m:
            stack.append({"type": m.group(1), "text": n if False else None, "fmt": False})
            continue
        if line.strip() == "}" and stack:
            b = stack.pop()
            if b["text"] and not b["fmt"]:
                bad.append(f"{f.name}:{b['text']} {b['type']} sans textFormat: Text.PlainText")
            continue
        if stack:
            if re.match(r"\s*text:\s", line):
                stack[-1]["text"] = n
            if re.match(r"\s*textFormat:\s*Text\.PlainText\s*$", line):
                stack[-1]["fmt"] = True
    # Seuls Label/Heading sont concernes
bad = [b for b in bad if re.search(r"(Label|Heading) sans", b)]
if "toolTipTextFormat: Text.PlainText" not in (ui / "main.qml").read_text():
    bad.append("main.qml : toolTipTextFormat: Text.PlainText manquant")
if bad:
    print("\n".join(bad), file=sys.stderr)
    sys.exit(1)
print("PASS plaintext")
