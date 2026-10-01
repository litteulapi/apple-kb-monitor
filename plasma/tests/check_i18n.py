#!/usr/bin/env python3
"""#114: every i18n() text of the widget has a French translation.

Reads the QML sources and po/fr.po (gettext), fails when
 * a text used by the widget has no entry or an empty msgstr in fr.po;
 * fr.po or the .pot keep an entry no source uses;
 * a msgstr changes the %N placeholders of its msgid;
 * the metadata.json carries no French name/description.
"""
import ast
import json
import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parent.parent
ui = root / "com.agenceapi.devicehub" / "contents" / "ui"
STR = r'"(?:[^"\\]|\\.)*"'


def unq(s):
    return ast.literal_eval(s)


used = set()
for qml in sorted(ui.glob("*.qml")):
    src = qml.read_text(encoding="utf-8")
    for m in re.finditer(r"\bi18n\(\s*(" + STR + r"(?:\s*\+\s*" + STR + r")*)", src):
        used.add("".join(unq(x) for x in re.findall(STR, m.group(1))))


def read_po(path):
    out = {}
    for block in path.read_text(encoding="utf-8").split("\n\n"):
        mid = re.search(r"^msgid ((?:" + STR + r"\s*)+)", block, re.M)
        mstr = re.search(r"^msgstr ((?:" + STR + r"\s*)+)", block, re.M)
        if not mid or not mstr:
            continue
        i = "".join(unq(x) for x in re.findall(STR, mid.group(1)))
        s = "".join(unq(x) for x in re.findall(STR, mstr.group(1)))
        if i:
            out[i] = s
    return out


fr = read_po(root / "po" / "fr.po")
pot = read_po(root / "po" / "plasma_applet_com.agenceapi.devicehub.pot")
errors = []
for t in sorted(used):
    if not fr.get(t):
        errors.append("no French translation: %r" % t)
    elif sorted(re.findall(r"%\d", fr[t])) != sorted(re.findall(r"%\d", t)):
        errors.append("placeholders differ: %r -> %r" % (t, fr[t]))
for t in sorted(set(fr) - used):
    errors.append("fr.po entry no source uses: %r" % t)
for t in sorted(set(pot) ^ used):
    errors.append("template out of sync with the sources: %r" % t)
meta = json.loads((root / "com.agenceapi.devicehub" / "metadata.json").read_text(encoding="utf-8"))["KPlugin"]
for k in ("Name", "Description"):
    if not meta.get(k + "[fr]"):
        errors.append("metadata.json lacks %s[fr]" % k)
if len(used) < 40:
    errors.append("only %d texts found, the scan is broken" % len(used))
if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)
print("i18n OK: %d widget texts translated" % len(used))
