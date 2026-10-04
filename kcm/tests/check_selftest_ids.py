"""Every message id of `akmctl selftest --json` is translated by DiagPage.qml (selftestText)."""
import re, sys

src, stale, qml = (open(p, encoding="utf-8").read() for p in sys.argv[1:4])
emitted = set(re.findall(r'\.msg\(\s*"([a-z0-9.-]+)"', src)) | set(re.findall(r'"(versions\.[a-z-]+)"', stale))
body = qml[qml.index("function selftestText("):]
body = body[:body.index("default:")]
mapped = set(re.findall(r'case "([a-z0-9.-]+)":', body))
if emitted - mapped:
    sys.exit(f"selftest ids not translated in DiagPage.qml: {sorted(emitted - mapped)}")
print(f"{len(emitted)} selftest ids translated")
