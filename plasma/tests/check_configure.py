#!/usr/bin/env python3
"""The applet's "Configure…" entry (internal action "configure") opens the
System Settings module instead of Plasma's generic applet settings dialog."""
import pathlib
import re
import sys

main = (pathlib.Path(__file__).resolve().parent.parent / "com.agenceapi.devicehub/contents/ui/main.qml").read_text()
m = re.search(r'Plasmoid\.setInternalAction\("configure",\s*(\w+)\)', main)
action = m and re.search(r"PlasmaCore\.Action\s*\{\s*id:\s*" + m.group(1) + r"\b(.*?)\n    \}", main, re.S)
if not action or "openSettings(" not in action.group(1):
    sys.exit("main.qml: no internal action \"configure\" opening the System Settings module")
print("configure action opens the System Settings module")
