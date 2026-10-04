#!/usr/bin/env python3
"""Every element that draws text (Text, TextEdit, TextArea, Label, Heading,
SelectableLabel, also as `delegate: Text {` or on one line) sets
`textFormat: Text.PlainText`, no InlineMessage or PromptDialog subtitle shows
text as markup, and the plasmoid tooltip is plain text. Static guard: otherwise
the HTML of a BlueZ alias would be rendered (<img src=http://...> = network request).
  check_plaintext.py [ui directory]   (default: the widget's contents/ui)"""
import re, sys, pathlib

ui = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else \
    pathlib.Path(__file__).resolve().parent.parent / "com.agenceapi.devicehub/contents/ui"
TEXT_TYPES = {"Text", "TextEdit", "TextArea", "Label", "Heading", "SelectableLabel"}
TOKEN = re.compile(r"([A-Z][\w.]*)\s*\{|\{|\}|(textFormat:\s*(?:Text|TextEdit)\.PlainText\b)")
bad = []
files = sorted(ui.glob("*.qml"))
if not files:
    bad.append(f"{ui}: no QML file")
for f in files:
    stack = []
    for n, line in enumerate(f.read_text().split("\n"), 1):
        line = re.sub(r'"(?:\\.|[^"\\])*"', '""', line).split("//")[0]
        if re.search(r"\bInlineMessage\s*\{", line) and f.name != "PlainMessage.qml":
            bad.append(f"{f.name}:{n} InlineMessage renders markup: use PlainMessage")
        if re.match(r"\s*subtitle\s*:", line):
            bad.append(f"{f.name}:{n} PromptDialog subtitle renders markup: use a plain-text Label")
        for m in TOKEN.finditer(line):
            if m.group(1):
                stack.append({"type": m.group(1), "line": n, "fmt": False})
            elif m.group(2):
                if stack:
                    stack[-1]["fmt"] = True
            elif m.group(0) == "{":
                stack.append({"type": None, "line": n, "fmt": False})
            elif stack:
                b = stack.pop()
                if b["type"] and b["type"].split(".")[-1] in TEXT_TYPES and not b["fmt"]:
                    bad.append(f"{f.name}:{b['line']} {b['type']} without textFormat: Text.PlainText")
main = ui / "main.qml"
if main.exists() and "PlasmoidItem" in main.read_text() and "toolTipTextFormat: Text.PlainText" not in main.read_text():
    bad.append("main.qml: toolTipTextFormat: Text.PlainText missing")
if bad:
    print("\n".join(bad), file=sys.stderr)
    sys.exit(1)
print(f"PASS plaintext ({len(files)} files)")
