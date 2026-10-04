#!/usr/bin/env bash
# qmllint of the KCM pages. Exit 0 = no warning; 1 = warnings
# (printed); 77 = qmllint missing.
#
# The only names allowed without qualification are the context properties a
# KCM receives at run time: `kcm` (KQuickConfigModule, set by kcmutils) and
# i18n()/i18nc()/i18np()/i18ncp() (KLocalizedContext, set by kcmutils with the
# plugin id "kcm_applekeyboard" as translation domain). qmllint cannot see
# them; every other warning, of every category, fails.
set -uo pipefail
here=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
qmllint=${QMLLINT:-/usr/lib/qt6/bin/qmllint}
[ -x "$qmllint" ] || { echo "qmllint skipped: $qmllint missing"; exit 77; }
cd "$here/ui" || exit 1
out=$("$qmllint" -I /usr/lib/qt6/qml --context-properties disable --json - ./*.qml 2>/dev/null)
printf '%s' "$out" | python3 -c '
import json, re, sys
allowed = {"kcm", "i18n", "i18nc", "i18np", "i18ncp"}
d = json.load(sys.stdin)
bad = 0
for f in d["files"]:
    src = open(f["filename"], encoding="utf-8").read().split("\n")
    for w in f["warnings"]:
        if w.get("type") == "info":
            continue
        line, col = w.get("line", 0), w.get("column", 0)
        m = re.match(r"[A-Za-z_]\w*", src[line - 1][col - 1:]) if line else None
        if w.get("id") == "unqualified" and m and m.group(0) in allowed:
            continue
        bad += 1
        print("%s:%s:%s: %s [%s]" % (f["filename"], line, col, w.get("message"), w.get("id")))
print("qmllint: %d files, %d warnings" % (len(d["files"]), bad))
sys.exit(1 if bad else 0)
'
