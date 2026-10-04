#!/usr/bin/env python3
"""Every i18n() / i18np() text of the widget is translated in every po/<lang>.po.

Reads the QML sources and po/*.po (gettext), fails when
 * a text used by the widget has no entry or an empty msgstr in a catalogue;
 * a catalogue or the .pot keeps an entry no source uses;
 * a msgstr changes the %N placeholders of its msgid;
 * the metadata.json lacks the description of a catalogue language.
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
plural = {}
for qml in sorted(ui.glob("*.qml")):
    src = qml.read_text(encoding="utf-8")
    for m in re.finditer(r"\bi18n\(\s*(" + STR + r"(?:\s*\+\s*" + STR + r")*)", src):
        used.add("".join(unq(x) for x in re.findall(STR, m.group(1))))
    # i18nc(context, text): the text is the msgid.
    for m in re.finditer(r"\bi18nc\(\s*" + STR + r"\s*,\s*(" + STR + r"(?:\s*\+\s*" + STR + r")*)", src):
        used.add("".join(unq(x) for x in re.findall(STR, m.group(1))))
    # counted texts go through i18np(singular, plural, n, ...)
    for m in re.finditer(r"\bi18np\(\s*(" + STR + r")\s*,\s*(" + STR + r")", src):
        plural[unq(m.group(1))] = unq(m.group(2))


def read_po(path, forms=None):
    out = {}
    for block in path.read_text(encoding="utf-8").split("\n\n"):
        mid = re.search(r"^msgid ((?:" + STR + r"\s*)+)", block, re.M)
        mpl = re.search(r"^msgid_plural ((?:" + STR + r"\s*)+)", block, re.M)
        if mid and mpl and forms is not None:
            i = "".join(unq(x) for x in re.findall(STR, mid.group(1)))
            fs = [
                "".join(unq(x) for x in re.findall(STR, m.group(1)))
                for m in re.finditer(r"^msgstr\[\d\] ((?:" + STR + r"\s*)+)", block, re.M)
            ]
            forms[i] = ("".join(unq(x) for x in re.findall(STR, mpl.group(1))), fs)
            continue
        mstr = re.search(r"^msgstr ((?:" + STR + r"\s*)+)", block, re.M)
        if not mid or not mstr:
            continue
        i = "".join(unq(x) for x in re.findall(STR, mid.group(1)))
        s = "".join(unq(x) for x in re.findall(STR, mstr.group(1)))
        if i:
            out[i] = s
    return out


# a label shown as is (bare `text:` literal or a list of names) escapes translation.
SHOWN_AS_IS = {"APIHUB — AGENCE API"}
literals = []
for qml in sorted(ui.glob("*.qml")):
    for n, line in enumerate(qml.read_text(encoding="utf-8").splitlines(), 1):
        for m in re.finditer(r"\btext:\s*(" + STR + r")\s*$", line):
            t = unq(m.group(1))
            if re.search(r"[A-Za-z]{3}", t) and t not in SHOWN_AS_IS and not t.startswith("sudo "):
                literals.append("%s:%d: %r" % (qml.name, n, t))
        if re.search(r"\[\s*\"[A-Z]{3,}\"\s*,", line):
            literals.append("%s:%d: list of untranslated names" % (qml.name, n))

catalog_forms = {}
catalogs = {}
for po in sorted((root / "po").glob("*.po")):
    catalog_forms[po.stem] = {}
    catalogs[po.stem] = read_po(po, catalog_forms[po.stem])
pot_forms = {}
pot = read_po(root / "po" / "plasma_applet_com.agenceapi.devicehub.pot", pot_forms)
errors = ["text shown untranslated: " + x for x in literals]
if "fr" not in catalogs:
    errors.append("po/fr.po is missing")
for lang, cat in catalogs.items():
    for t in sorted(used):
        if not cat.get(t):
            errors.append("no %s translation: %r" % (lang, t))
        elif sorted(re.findall(r"%\d", cat[t])) != sorted(re.findall(r"%\d", t)):
            errors.append("%s placeholders differ: %r -> %r" % (lang, t, cat[t]))
    for t in sorted(set(cat) - used):
        errors.append("%s.po entry no source uses: %r" % (lang, t))
    forms = catalog_forms[lang]
    nplurals = re.search(r"nplurals=(\d+)", (root / "po" / (lang + ".po")).read_text(encoding="utf-8"))
    for one, many in sorted(plural.items()):
        got = forms.get(one)
        if not got or got[0] != many or not all(got[1]):
            errors.append("no %s plural translation: %r" % (lang, one))
            continue
        if not nplurals or len(got[1]) != int(nplurals.group(1)):
            errors.append("%s plural forms of %r do not match Plural-Forms" % (lang, one))
        for f in got[1]:
            if sorted(set(re.findall(r"%\d", f))) != sorted(set(re.findall(r"%\d", many))):
                errors.append("%s placeholders differ: %r -> %r" % (lang, many, f))
    for t in sorted(set(forms) - set(plural)):
        errors.append("%s.po plural entry no source uses: %r" % (lang, t))
for t in sorted(used | set(plural)):
    # "PROBLEM(S)" reads wrong whatever the count; word the text so it needs no plural.
    if re.search(r"\w\((s|S)\)", t):
        errors.append("plural written as (s): %r" % t)
for t in sorted(set(pot) ^ used):
    errors.append("template out of sync with the sources: %r" % t)
for t in sorted({(k, v[0]) for k, v in pot_forms.items()} ^ set(plural.items())):
    errors.append("template plural out of sync with the sources: %r" % (t,))
meta = json.loads((root / "com.agenceapi.devicehub" / "metadata.json").read_text(encoding="utf-8"))["KPlugin"]
for lang in catalogs:
    # Name is the brand "ApiHub": a Name[xx] identical to it is noise
    for k in ("Description",):
        if not meta.get("%s[%s]" % (k, lang)):
            errors.append("metadata.json lacks %s[%s]" % (k, lang))
if len(used) < 40:
    errors.append("only %d texts found, the scan is broken" % len(used))
if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)
print("i18n OK: %d widget texts (%d plural) translated in %d languages" % (len(used) + len(plural), len(plural), len(catalogs)))
