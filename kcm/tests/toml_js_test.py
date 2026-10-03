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

# #284 (K3): byte for byte, only the statement of the key changes; every
# other line keeps its bytes (CRLF, no final newline, BOM, foreign sections,
# a multi-line string elsewhere that looks like a table header).
EXACT = [  # (name, input, section, key, value, expected text)
    ("no-final-newline", "[alerts]\nenabled = true", "alerts", "enabled", False, "[alerts]\nenabled = false"),
    ("no-final-newline-new-key", "[alerts]\nenabled = true", "alerts", "critical", 9,
     "[alerts]\nenabled = true\ncritical = 9"),
    ("no-final-newline-new-table", "[ddc]\nbrightness = 40", "alerts", "critical", 9,
     "[ddc]\nbrightness = 40\n\n[alerts]\ncritical = 9"),
    ("mixed-endings", "[mqtt]\r\npassword = \"p\"\n[alerts]\r\nenabled = true\n# end\r\n", "alerts", "enabled", False,
     "[mqtt]\r\npassword = \"p\"\n[alerts]\r\nenabled = false\n# end\r\n"),
    ("bom", "\ufeff[alerts]\nenabled = true\n", "alerts", "enabled", False, "\ufeff[alerts]\nenabled = false\n"),
    ("foreign-sections-kept", "[ddc]\nbrightness = 40\n[mqtt]\nhost = \"h\" # c\n[alerts]\ncritical = 5\n[monitor]\nx = [\n 1,\n]\n",
     "alerts", "critical", 7,
     "[ddc]\nbrightness = 40\n[mqtt]\nhost = \"h\" # c\n[alerts]\ncritical = 7\n[monitor]\nx = [\n 1,\n]\n"),
    ("header-inside-string", "[ddc]\nnote = \"\"\"\n[alerts]\ncritical = 1\n\"\"\"\n[alerts]\ncritical = 5\n", "alerts", "critical", 7,
     "[ddc]\nnote = \"\"\"\n[alerts]\ncritical = 1\n\"\"\"\n[alerts]\ncritical = 7\n"),
    ("key-inside-string-not-taken", "[alerts]\nnote = \"\"\"\ncritical = 1\n\"\"\"\n", "alerts", "critical", 7,
     "[alerts]\nnote = \"\"\"\ncritical = 1\n\"\"\"\ncritical = 7\n"),
    ("other-key-untouched", "[alerts]\ncritical = 99\nhysteresis = 2.5 # h\nenabled = true\n", "alerts", "enabled", False,
     "[alerts]\ncritical = 99\nhysteresis = 2.5 # h\nenabled = false\n"),
]

# Toml.parse() must read what tomllib reads (#12): every scalar by
# "section.key", with the same value. Invalid documents are read statement by
# statement (strict = false) like the daemon, and report each bad line.
PARSE_VALID = [
    "[alerts]\nthresholds = [30, 15, 5]\ncritical = 5\nhysteresis = 3.5\n",
    "[alerts]\nthresholds = [\n  30,  # first\n  15,\n]\n",
    "a = 1\n[b]\nc = 'lit\\eral'\nd = \"tab\\tq\\\"\\u00e9\"\n",
    "[x]\ns = \"\"\"\nline1\nline2\\\n   joined\"\"\"\nl = \'\'\'raw \\n\'\'\'\n",
    "[n]\nhex = 0xff\noct = 0o17\nbin = 0b101\nund = 1_000\nneg = -7\nexp = 1e3\nf = -0.5\n",
    "[a.b]\nk = true\n[a]\nz = false\n",
    "[t]\ninl = { p = 1, q = \"r\" }\ndot.ted = 2\n\"quoted key\" = 3\n",
    "[[arr]]\nv = 1\n[[arr]]\nv = 2\n[after]\nk = 1\n",
    "\ufeff[bom]\nk = 1\r\n[crlf]\r\nm = 2\r\n",
    "[m]\nmixed = [1, \"two\", [3]]\nempty = []\n",
]
PARSE_INVALID = [  # (document, values read, a warning expected)
    ("[alerts]\ncritical = 5\nbroken line\nhysteresis = 3\n", {"alerts.critical": 5, "alerts.hysteresis": 3}, True),
    ("[alerts]\nthresholds = [30, 15\n", {}, True),
    ("[a]\nx = \"unterminated\ny = 2\n", {"a.y": 2}, True),
    # a key or a table defined twice: the daemon (config.rs, statements())
    # keeps the last value without a warning, and so does Toml.parse
    ("[a]\nk = 1\nk = 2\n", {"a.k": 2}, False),
    ("[a]\nk = 1\n[a]\nj = 2\n", {"a.k": 1, "a.j": 2}, False),
]


# Toml.typed(): a value of another type is ignored, as serde does in config.rs.
TYPED = [  # (document, key, wanted kind, expected value or None)
    ("[alerts]\ncritical = 5\n", "alerts.critical", "integer", 5),
    ("[alerts]\ncritical = \"5\"\n", "alerts.critical", "integer", None),
    ("[alerts]\nenabled = 1\n", "alerts.enabled", "boolean", None),
    ("[alerts]\nenabled = false\n", "alerts.enabled", "boolean", False),
    ("[alerts]\nhysteresis = 3\n", "alerts.hysteresis", "float", 3),
    ("[alerts]\nhysteresis = nan\n", "alerts.hysteresis", "float", None),
    ("[alerts]\nthresholds = [30, 15]\n", "alerts.thresholds", "integers", [30, 15]),
    ("[alerts]\nthresholds = [30, 1.5]\n", "alerts.thresholds", "integers", None),
    ("[alerts]\nthresholds = []\n", "alerts.thresholds", "integers", []),
    ("[battery]\nchemistry = 2\n", "battery.chemistry", "string", None),
]


def flat(doc, prefix=""):
    out = {}
    for k, v in doc.items():
        if isinstance(v, dict):
            out.update(flat(v, k if prefix == "" else prefix + "." + k))
        elif isinstance(v, list) and v and all(isinstance(e, dict) for e in v):
            continue  # arrays of tables are left out by Toml.parse
        else:
            out[prefix + "." + k] = v
    return out


PARSE_DRIVER = r"""
const fs = require("fs");
const src = fs.readFileSync(process.argv[1], "utf8").replace(/^\.pragma.*$/m, "");
const Toml = new Function(src + "; return { parse: parse, typed: typed };")();
const input = JSON.parse(fs.readFileSync(0, "utf8"));
process.stdout.write(JSON.stringify({
    docs: input.docs.map(function (d) {
        try { return Toml.parse(d); } catch (e) { return { error: String(e.name) + ": " + e.message }; }
    }),
    typed: input.typed.map(function (c) {
        const v = Toml.typed(Toml.parse(c[0]), c[1], c[2]);
        return v === undefined ? null : v;
    }),
}));
"""


def parse_cases() -> int:
    docs = PARSE_VALID + [d for d, _, _ in PARSE_INVALID]
    r = subprocess.run(["node", "-e", PARSE_DRIVER, str(TOML_JS)],
                       input=json.dumps({"docs": docs, "typed": TYPED}),
                       capture_output=True, text=True, timeout=60)
    if r.returncode != 0:
        print(r.stderr.strip()[-2000:])
        return 1
    out = json.loads(r.stdout)
    res = out["docs"]
    bad = 0
    for (doc, key, kind, want), got in zip(TYPED, out["typed"]):
        ok = got == want and type(got) is type(want)
        print("%-32s %s" % ("typed-%s-%s" % (key, kind), "ok" if ok else "FAIL: %r, expected %r" % (got, want)))
        bad += not ok
    for i, doc in enumerate(PARSE_VALID):
        got, why = res[i], None
        want = flat(tomllib.loads(doc.lstrip("\ufeff")))
        if "error" in got:
            why = got["error"]
        elif not got["strict"] or got["warnings"]:
            why = "read as invalid: %r" % got["warnings"]
        elif got["values"] != want:
            why = "values %r, tomllib %r" % (got["values"], want)
        print("%-32s %s" % ("parse-valid-%d" % i, "ok" if why is None else "FAIL: " + why))
        bad += why is not None
    for j, (doc, want, warn) in enumerate(PARSE_INVALID):
        got, why = res[len(PARSE_VALID) + j], None
        try:
            tomllib.loads(doc)
            why = "tomllib accepts this case: fix the test"
        except tomllib.TOMLDecodeError:
            pass
        if why is None and "error" in got:
            why = got["error"]
        elif why is None and got["strict"]:
            why = "read as valid: %r" % got
        elif why is None and bool(got["warnings"]) != warn:
            why = "warnings %r, expected %s" % (got["warnings"], "some" if warn else "none")
        elif why is None:
            for k, v in want.items():
                if got["values"].get(k) != v:
                    why = "%s = %r, expected %r" % (k, got["values"].get(k), v)
        print("%-32s %s" % ("parse-invalid-%d" % j, "ok" if why is None else "FAIL: " + why))
        bad += why is not None
    print("toml-js parse: %d cases, %d failing" % (len(docs) + len(TYPED), bad))
    return bad


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


def exact_cases() -> int:
    r = subprocess.run(
        ["node", "-e", DRIVER, str(TOML_JS)],
        input=json.dumps([c[:5] + ("ok",) for c in EXACT]), capture_output=True, text=True, timeout=60,
    )
    if r.returncode != 0:
        print(r.stderr.strip()[-2000:])
        return 1
    bad = 0
    for (name, src, section, key, value, want), res in zip(EXACT, json.loads(r.stdout)):
        got = res.get("text", res.get("error"))
        why = None if got == want else "got %r, expected %r" % (got, want)
        print("%-32s %s" % ("exact-" + name, "ok" if why is None else "FAIL: " + why))
        bad += why is not None
    print("toml-js exact: %d cases, %d failing" % (len(EXACT), bad))
    return bad


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
    bad += exact_cases()
    bad += parse_cases()
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
