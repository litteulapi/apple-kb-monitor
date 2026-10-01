#!/usr/bin/env python3
"""Toml.js (KCM) checked with node and Python's strict TOML reader (#258).

Every output of Toml.set() must be valid TOML (tomllib) and hold the value
written; the forms the editor cannot rewrite safely must be refused
(UnsupportedForm), never duplicated. Exit 0 = all cases pass, 1 = a failure,
77 = node or tomllib missing.
"""
import json
import pathlib
import shutil
import subprocess
import sys

try:
    import tomllib
except ImportError:  # Python < 3.11
    print("toml-js skipped: tomllib missing")
    sys.exit(77)
if shutil.which("node") is None:
    print("toml-js skipped: node missing")
    sys.exit(77)

TOML_JS = pathlib.Path(__file__).resolve().parent.parent / "ui" / "Toml.js"

# (name, input, section, key, value, expected outcome)
#   expected: "ok" -> valid TOML with section.key == value
#             "refuse" -> UnsupportedForm thrown
CASES = [
    ("plain", "[alerts]\nlow = 15\n", "alerts", "low", 5, "ok"),
    ("crlf", "[alerts]\r\nlow = 15\r\n", "alerts", "low", 5, "ok"),
    ("crlf-new-key", "[alerts]\r\nlow = 15\r\n", "alerts", "critical", 3, "ok"),
    ("crlf-new-table", "[ui]\r\nlang = \"fr\"\r\n", "alerts", "low", 5, "ok"),
    ("spaced-header", "[ alerts ]\nlow = 15\n", "alerts", "low", 5, "ok"),
    ("comment-kept", "[alerts]\nlow = 15 # mine\n", "alerts", "low", 5, "ok"),
    ("multiline-array-comment",
     "[alerts]\nthresholds = [\n 20,\n 10,\n] # c\nlow = 1\n", "alerts", "thresholds", [5], "ok"),
    ("missing-key", "[alerts]\ncritical = 3\n\n[ui]\nx = 1\n", "alerts", "low", 5, "ok"),
    ("quoted-key", "[alerts]\n\"low\" = 15\n", "alerts", "low", 5, "refuse"),
    ("single-quoted-key", "[alerts]\n'low' = 15\n", "alerts", "low", 5, "refuse"),
    ("inline-table", "alerts = { thresholds = [20, 10], low = 15 }\n", "alerts", "low", 5, "refuse"),
    ("dotted-top-level", "alerts.low = 15\n", "alerts", "low", 5, "refuse"),
    ("array-of-tables", "[[alerts]]\nlow = 1\n", "alerts", "low", 5, "refuse"),
    ("quoted-header", "[\"alerts\"]\nlow = 1\n", "alerts", "low", 5, "refuse"),
    ("other-quoted-key-is-fine", "[alerts]\n\"other\" = 1\nlow = 2\n", "alerts", "low", 5, "ok"),
    ("other-array-of-tables-is-fine", "[[x]]\na = 1\n\n[alerts]\nlow = 2\n", "alerts", "low", 5, "ok"),
]

DRIVER = r"""
const fs = require("fs");
const src = fs.readFileSync(process.argv[1], "utf8").replace(/^\.pragma.*$/m, "");
const Toml = new Function(src + "; return { parse: parse, set: set };")();
const cases = JSON.parse(fs.readFileSync(0, "utf8"));
const out = cases.map(function (c) {
    try {
        const text = Toml.set(c[1], c[2], c[3], c[4]);
        return { text: text, parsed: Toml.parse(text).values };
    } catch (e) {
        return { error: String(e.name) + ": " + e.message };
    }
});
process.stdout.write(JSON.stringify(out));
"""


def main() -> int:
    r = subprocess.run(
        ["node", "-e", DRIVER, str(TOML_JS)],
        input=json.dumps(CASES), capture_output=True, text=True, timeout=60,
    )
    if r.returncode != 0:
        print(r.stderr.strip()[-2000:])
        return 1
    results = json.loads(r.stdout)
    bad = 0
    for (name, src, section, key, value, want), res in zip(CASES, results):
        why = None
        if want == "refuse":
            if "error" not in res or not res["error"].startswith("UnsupportedForm"):
                why = "expected a refusal, got %r" % (res.get("text") or res.get("error"))
        elif "error" in res:
            why = "unexpected error %s" % res["error"]
        else:
            text = res["text"]
            try:
                doc = tomllib.loads(text)
            except tomllib.TOMLDecodeError as e:
                why = "tomllib rejects the output (%s): %r" % (e, text)
            else:
                if doc.get(section, {}).get(key) != value:
                    why = "tomllib reads %s.%s = %r, expected %r" % (
                        section, key, doc.get(section, {}).get(key), value)
                elif res["parsed"].get(section + "." + key) != value:
                    why = "Toml.parse reads %r" % res["parsed"].get(section + "." + key)
                elif ("\r\n" in src) != ("\r\n" in text) or ("\r\n" in src and "\n" in text.replace("\r\n", "")):
                    why = "line endings not kept: %r" % text
                elif "#" in src and src.split("#", 1)[1].split("\n")[0].strip() not in text:
                    why = "comment lost: %r" % text
        print("%-32s %s" % (name, "ok" if why is None else "FAIL: " + why))
        bad += why is not None
    print("toml-js: %d cases, %d failing" % (len(CASES), bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
