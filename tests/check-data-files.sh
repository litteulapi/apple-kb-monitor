#!/bin/sh
# Every XML, JSON and .desktop file the package installs, parsed by a real
# parser (#292). A grep cannot see that a file does not load: a double hyphen
# in an XML comment made polkitd drop the whole .policy, so the doctor-fix
# action never existed although every grep-based check was green.
# No root, nothing installed. Run from anywhere: sh tests/check-data-files.sh
set -eu
top=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
fail() { echo "FAIL: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "$1 missing (package $2): it validates the installed files"; }
need xmllint libxml2
need desktop-file-validate desktop-file-utils
need python3 python

# polkit: well-formed, valid against polkit's own DTD (local copy, no network),
# and every <action id= of the text is an action of the parsed document.
policy="$top/polkit/com.agenceapi.AppleKbMonitor.policy"
dtd=/usr/share/polkit-1/policyconfig-1.dtd
if [ -f "$dtd" ]; then
  # the external DTD named in the DOCTYPE cannot be fetched (--nonet): that
  # I/O warning is expected, the exit code is what counts
  xmllint --noout --nonet --dtdvalid "$dtd" "$policy" 2>/dev/null \
    || { xmllint --noout --nonet --dtdvalid "$dtd" "$policy" 2>&1 | grep -v 'I/O warning\|policyconfig.dtd\|^ *^' >&2 || true; fail "$(basename "$policy") is not valid against $dtd"; }
else
  xmllint --noout --nonet "$policy" 2>&1 | grep -v 'I/O warning\|policyconfig.dtd\|^ *^' >&2 || true
  xmllint --noout --nonet "$policy" 2>/dev/null || fail "$(basename "$policy") is not well-formed XML"
fi
# polkitd parses with expat, as Python's ElementTree does
python3 - "$policy" <<'PY' || fail "polkit actions: parsed list differs from the declared one"
import re, sys, xml.etree.ElementTree as ET
p = sys.argv[1]
parsed = [a.get("id") for a in ET.parse(p).getroot().iter("action")]
declared = re.findall(r'<action id="([^"]+)"', re.sub(r"<!--.*?-->", "", open(p).read(), flags=re.S))
if parsed != declared or not parsed:
    print(f"parsed {parsed} != declared {declared}", file=sys.stderr); sys.exit(1)
for a in ET.parse(p).getroot().iter("action"):
    for tag in ("description", "message", "defaults"):
        if a.find(tag) is None:
            print(f"{a.get('id')}: <{tag}> missing", file=sys.stderr); sys.exit(1)
print(f"polkit: {len(parsed)} actions load: " + ", ".join(i.rsplit('.', 1)[1] for i in parsed))
PY

# .desktop files (application, global shortcuts, KRunner plugin)
for f in "$top/com.agenceapi.AppleKbMonitor.desktop" "$top"/data/*.desktop; do
  out=$(desktop-file-validate "$f" 2>&1) || fail "desktop-file-validate $(basename "$f"): $out"
  [ -z "$out" ] || fail "desktop-file-validate $(basename "$f") warns: $out"
done

# JSON metadata: the widget (read by plasmashell) and the KCM (compiled into the .so)
for j in "$top/plasma/com.agenceapi.devicehub/metadata.json" "$top/kcm/kcm_applekeyboard.json"; do
  python3 -m json.tool "$j" >/dev/null || fail "$(basename "$j") is not valid JSON"
done

# D-Bus activation files: one Name=, one Exec= on an installed path
for s in "$top"/dbus/*.service "$top"/plasma/krunner/*.service; do
  exe=$(sed -n 's/^Exec=\([^ ]*\).*/\1/p' "$s")
  case $exe in
    /usr/bin/apihub-app|/usr/bin/apple-kb-monitord) ;;
    *) fail "$(basename "$s"): Exec=$exe is not a binary of the package" ;;
  esac
  name=$(sed -n 's/^Name=//p' "$s")
  [ "$name.service" = "$(basename "$s")" ] || fail "$(basename "$s"): Name=$name must match the file name"
done
# One licence everywhere it is declared (#297): the one of the PKGBUILD,
# GPL-2.0-or-later since the first release (LICENSE = the GPL v2 text).
lic=$(sed -n "s/^license=('\([^']*\)'.*/\1/p" "$top/PKGBUILD")
[ "$lic" = GPL-2.0-or-later ] || fail "PKGBUILD license=$lic, the project is GPL-2.0-or-later"
for j in "$top/plasma/com.agenceapi.devicehub/metadata.json" "$top/kcm/kcm_applekeyboard.json"; do
  l=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["KPlugin"].get("License",""))' "$j")
  [ "$l" = "$lic" ] || fail "$(basename "$j"): License $l != $lic"
done
for d in "$top"/data/*.desktop "$top/com.agenceapi.AppleKbMonitor.desktop"; do
  l=$(sed -n 's/^X-KDE-PluginInfo-License=//p' "$d")
  [ -z "$l" ] || [ "$l" = "$lic" ] || fail "$(basename "$d"): License $l != $lic"
done
echo "PASS data files (polkit XML + DTD, .desktop, JSON, D-Bus services, licence)"
