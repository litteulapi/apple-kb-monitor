#!/usr/bin/env python3
"""Static checks of the tree for scripts/ci-local.sh (stdlib only).

    qa_checks.py secrets   [--paths FILE...]   keys, link keys/IRK, tokens, 0x4C payloads
    qa_checks.py claims                        documents vs the registry (forbidden claims)
    qa_checks.py redaction                     no Apple binary, no decompiled code
    qa_checks.py package PKG.tar.zst           content of the built package
    qa_checks.py report OUT_DIR                steps.tsv -> report.json + report.txt

Each check prints one line per finding (``path:line: [rule] message``) and
exits 1 when there is at least one finding that is not allow-listed.
Allow-lists live next to this file and must carry a reason (and an issue
number when the finding is a known bug), so silencing is never silent.
"""

from __future__ import annotations

import json
import os
import re
import stat
import subprocess
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HERE = Path(__file__).resolve().parent
MAX_TEXT = 5 * 1024 * 1024


def tree_files() -> list[Path]:
    """Tracked + untracked-not-ignored files of the working tree."""
    out = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        capture_output=True, check=True,
    ).stdout
    files = []
    for raw in out.split(b"\0"):
        if not raw:
            continue
        p = ROOT / raw.decode("utf-8", "surrogateescape")
        if p.is_file() and not p.is_symlink():
            files.append(p)
    return files


def text_of(p: Path) -> str | None:
    try:
        if p.stat().st_size > MAX_TEXT:
            return None
        data = p.read_bytes()
    except OSError:
        return None
    if b"\0" in data[:8192]:
        return None
    return data.decode("utf-8", "replace")


def rel(p: Path) -> str:
    try:
        return str(p.relative_to(ROOT))
    except ValueError:
        return str(p)


def load_allow(name: str) -> list[tuple[re.Pattern, re.Pattern, str]]:
    """Lines ``path-regex<TAB>text-regex<TAB>reason``; '#' comments."""
    rules = []
    f = HERE / name
    if not f.exists():
        return rules
    for n, line in enumerate(f.read_text().splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) < 3 or not parts[2].strip():
            sys.exit(f"{f}:{n}: allow-list entry needs path<TAB>regex<TAB>reason")
        rules.append((re.compile(parts[0]), re.compile(parts[1]), parts[2].strip()))
    return rules


def allowed(rules, path: str, text: str) -> str | None:
    for pr, tr, why in rules:
        if pr.search(path) and tr.search(text):
            return why
    return None


class Findings:
    def __init__(self, allow_file: str):
        self.allow = load_allow(allow_file)
        self.bad = 0
        self.known = 0

    def add(self, path: str, line: int, rule: str, msg: str, text: str = ""):
        why = allowed(self.allow, path, text or msg)
        if why:
            self.known += 1
            print(f"{path}:{line}: [{rule}] KNOWN ({why}): {msg}")
        else:
            self.bad += 1
            print(f"{path}:{line}: [{rule}] {msg}")

    def done(self, what: str) -> int:
        print(f"-- {what}: {self.bad} finding(s), {self.known} known/allow-listed")
        return 1 if self.bad else 0


# ── secrets ──────────────────────────────────────────────────────────────────

SECRET_RULES = [
    ("private-key", re.compile(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----")),
    ("github-token", re.compile(r"\bgh[pousr]_[A-Za-z0-9]{36,}\b")),
    ("gitlab-token", re.compile(r"\bglpat-[A-Za-z0-9_-]{20,}\b")),
    ("slack-token", re.compile(r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b")),
    ("aws-key", re.compile(r"\bAKIA[0-9A-Z]{16}\b")),
    ("api-token", re.compile(r"(?i)(?:token|secret|api[_-]?key|authorization)[\"' ]*[:=]\s*[\"']?(?:token\s+)?[0-9a-f]{40}\b")),
    ("auth-header", re.compile(r"(?i)authorization:\s*(?:token|bearer)\s+[A-Za-z0-9._-]{20,}")),
    # BlueZ storage (/var/lib/bluetooth/<adapter>/<device>/info): [LinkKey] Key=, IRK, LTK
    ("bluez-key", re.compile(r"(?m)^\s*Key\s*=\s*[0-9A-Fa-f]{32}\s*$")),
    ("bluez-irk", re.compile(r"(?i)(?:IdentityResolvingKey|LongTermKey|SlaveLongTermKey|PeripheralLongTermKey|\bIRK\b)[^\n]{0,40}[0-9A-Fa-f]{32}")),
    # debugfs .../hci0/link_keys: "<mac> <type> <32 hex> <pin_len>"
    ("debugfs-link-key", re.compile(r"(?i)\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\s+\d+\s+[0-9a-f]{32}\b")),
]

# 0x4C: 1 byte id + 1 byte (03) + 6 bytes host address are public (HID_PHYS);
# the 12 following bytes must never be published (#133).
HEX4C = re.compile(r"(?i)\b4c[ :]?03(?:[ :]?[0-9a-f]{2}){6}((?:[ :]?[0-9a-f]{2})+)")
JSON4C = re.compile(r"(?i)\"id\"\s*:\s*\"?(?:0x4c|76)\"?[^\n]*?\"hex\"\s*:\s*\"([0-9a-f ]+)")


def check_4c(text: str, emit):
    for m in HEX4C.finditer(text):
        tail = re.sub(r"[ :]", "", m.group(1))
        if tail.strip("0"):
            emit(text.count("\n", 0, m.start()) + 1, "0x4C-payload",
                 f"report 0x4C published beyond the host address ({len(tail)//2} more bytes, not zeroed)")
    for m in JSON4C.finditer(text):
        h = m.group(1).replace(" ", "")
        if len(h) > 16 and h[16:].strip("0"):
            emit(text.count("\n", 0, m.start()) + 1, "0x4C-payload",
                 "JSON record of report 0x4C carries more than id+03+host address")


def cmd_secrets(paths: list[str]) -> int:
    f = Findings("secrets-allow.tsv")
    files = [Path(p).resolve() for p in paths] if paths else tree_files()
    own_tokens = []
    tok = Path.home() / ".config/gitea/token"
    try:
        t = tok.read_text().strip()
        if len(t) >= 20:
            own_tokens.append(t)
    except OSError:
        pass
    for p in files:
        if rel(p).startswith("scripts/out/"):
            continue
        text = text_of(p)
        if text is None:
            continue
        r = rel(p)
        for rule, rx in SECRET_RULES:
            for m in rx.finditer(text):
                ln = text.count("\n", 0, m.start()) + 1
                line = text.splitlines()[ln - 1] if text else ""
                f.add(r, ln, rule, f"looks like a secret: {m.group(0)[:12]}…", line)
        for t in own_tokens:
            if t in text:
                f.add(r, text[: text.index(t)].count("\n") + 1, "gitea-token",
                      "the Gitea token of ~/.config/gitea/token is in this file")
        check_4c(text, lambda ln, rule, msg: f.add(r, ln, rule, msg, text.splitlines()[ln - 1]))
    gl = subprocess.run(["sh", "-c", "command -v gitleaks"], capture_output=True)
    if gl.returncode == 0 and not paths:
        out = HERE / "out" / "gitleaks.json"
        out.parent.mkdir(parents=True, exist_ok=True)
        rc = subprocess.run(
            ["gitleaks", "dir", str(ROOT), "--no-banner", "--redact", "--report-format", "json",
             "--report-path", str(out), "--log-level", "error"],
            capture_output=True,
        ).returncode
        if rc not in (0, 1):
            print(f"gitleaks failed to run (rc={rc})")
        elif out.exists():
            for item in json.loads(out.read_text() or "[]"):
                path = item.get("File", "")
                path = rel(Path(path)) if path.startswith("/") else path
                if path.startswith((".git/", "scripts/out/")) or "/target" in path:
                    continue
                f.add(path, item.get("StartLine", 0), "gitleaks:" + item.get("RuleID", "?"),
                      item.get("Description", ""), item.get("Match", ""))
    return f.done("secrets")


# ── document claims vs the registry ─────────────────────────────────────────

# (id, claim regex, negation regex: the line refutes or qualifies the claim,
#  message).  Product surface = user-facing docs and strings: strict.
NEG = r"(?i)(❌|\bnot\b|\bnever\b|n['’]est pas|\bpas\b|\bni\b|jamais|faux|fausse|réfut|erron|wrong|false|inconnu|unknown|à éviter|non démontré|à vérifier|déduction|≠|\bno\b|instead|au lieu|corrigé|relati|golden|sens inconnu|constante|inatteignable|unreachable)"
CLAIMS = [
    ("adc-0xF5", r"(?i)0xF5[^|\n]{0,25}\bADC\b|\bADC\b[^|\n]{0,25}0xF5",
     "0xF5 is a constant of unknown meaning, not an ADC (CONTRE-AUDIT #21)"),
    ("arm7tdmi", r"ARM7TDMI", "the BCM2042 core is an 8051 (brief 2042-PB03-R), not an ARM7TDMI (CONTRE-AUDIT #81)"),
    ("signed-fw", r"(?i)firmware\s+sign[ée]|signed\s+firmware", "firmware signature of the A1314 is unknown (CONTRE-AUDIT)"),
    ("rssi-dbm", r"(?i)RSSI[^|\n]{0,30}-?\d*\s*dBm|dBm[^|\n]{0,30}RSSI",
     "BR/EDR Read_RSSI is relative to the golden range, not a power in dBm (#174)"),
    ("identity-key", r"(?i)0x4C[^|\n]{0,30}(identity key|internal key|128-bit)",
     "0x4C = host address + 12 bytes, not an identity key (#133/#200)"),
]
DOC_GLOBS = ["README.md", "CHANGELOG.md", "docs/*.md", "udev/README.md", "plasma/**/*.qml", "plasma/**/*.js",
             "apihub-app/src/*.rs", "apihub-app/apple-kb-monitord/src/**/*.rs", "apihub-app/crates/*/src/*.rs",
             "com.agenceapi.AppleKbMonitor.desktop", "plasma/**/metadata.json"]
# Identifiers in code are not claims (rssi_dbm field kept for compatibility).
CODE_NOISE = re.compile(r"rssi_dbm|tx_power_dbm|HID_ADC_RAW|adc_raw")


def cmd_claims() -> int:
    f = Findings("claims-allow.tsv")
    neg = re.compile(NEG)
    seen = set()
    for g in DOC_GLOBS:
        for p in sorted(ROOT.glob(g)):
            if p in seen or not p.is_file() or p.name in ("QA-AUTOMATIQUE.md", "CONTRE-AUDIT.md"):  # CONTRE-AUDIT = the registry itself
                continue
            seen.add(p)
            text = text_of(p)
            if text is None:
                continue
            for ln, line in enumerate(text.splitlines(), 1):
                probe = CODE_NOISE.sub("", line)
                for cid, rx, msg in CLAIMS:
                    if re.search(rx, probe) and not neg.search(probe):
                        f.add(rel(p), ln, cid, msg + " :: " + line.strip()[:140], line)
    return f.done("claims")


# ── redaction: no Apple binary, no decompiled code ──────────────────────────

MAGICS = {
    b"\xfe\xed\xfa\xce": "Mach-O 32", b"\xfe\xed\xfa\xcf": "Mach-O 64", b"\xce\xfa\xed\xfe": "Mach-O 32 LE",
    b"\xcf\xfa\xed\xfe": "Mach-O 64 LE", b"\xca\xfe\xba\xbe": "Mach-O universal", b"xar!": "xar/pkg archive",
    b"\x7fELF": "ELF binary", b"MZ": "PE binary",
}
BAD_EXT = re.compile(r"(?i)\.(dmg|pkg|mpkg|kext|ipsw|bin|fw|dylib|o|so|a|exe|dll|img|smc|scap)$")
ALLOWED_BIN = re.compile(r"^(tests/fixtures/a1314_iso/report_descriptor\.bin)$")
DECOMP = re.compile(r"\bundefined[1248]?\s+\w|\bunaff_\w+|\bextraout_\w+|\bin_stack_\w+|WARNING: (?:Decompil|Could not recover)|\b__thiscall\b|\bCONCAT\d\d\(|\bSUB\d\d\(")


def cmd_redaction() -> int:
    f = Findings("redaction-allow.tsv")
    for p in tree_files():
        r = rel(p)
        if r.startswith("scripts/out/"):
            continue
        try:
            head = p.open("rb").read(8)
            size = p.stat().st_size
        except OSError:
            continue
        for mg, what in MAGICS.items():
            if head.startswith(mg) and not (mg == b"MZ" and text_of(p) is not None):
                f.add(r, 0, "binary", f"{what} committed ({size} bytes)")
        if size > 16 and head[-8:] == b"koly" or (size > 512 and p.suffix.lower() == ".dmg"):
            f.add(r, 0, "binary", "disk image committed")
        if BAD_EXT.search(r) and not ALLOWED_BIN.match(r):
            f.add(r, 0, "binary", "binary-looking file name (firmware, image, library)")
        text = text_of(p)
        if text is None:
            continue
        hits = [n for n, l in enumerate(text.splitlines(), 1) if DECOMP.search(l)]
        if len(hits) >= 3:
            f.add(r, hits[0], "decompiled", f"{len(hits)} lines with decompiler artefacts (Ghidra/IDA): code must be described, not pasted")
    return f.done("redaction")


# ── package content ─────────────────────────────────────────────────────────

def cargo_version() -> str:
    t = (ROOT / "apihub-app/Cargo.toml").read_text()
    m = re.search(r"\[workspace\.package\][^\[]*?version\s*=\s*\"([^\"]+)\"", t, re.S)
    return m.group(1) if m else "?"


def package_content(pkg: str, files: set, f: "Findings") -> None:
    """What the shipped files say, read by the parsers that will read them (#292, #293)."""
    import shutil
    import tempfile
    import xml.etree.ElementTree as ET
    with tempfile.TemporaryDirectory(prefix="akm-pkg-") as tmp:
        r = subprocess.run(["bsdtar", "-xf", pkg, "-C", tmp], capture_output=True, text=True)
        if r.returncode:
            f.add(pkg, 0, "extract", r.stderr.strip()[:200])
            return
        root = Path(tmp)
        # the scriptlet must at least parse
        r = subprocess.run(["bash", "-n", str(root / ".INSTALL")], capture_output=True, text=True)
        if r.returncode:
            f.add(".INSTALL", 0, "syntax", r.stderr.strip()[:200])
        # polkit: polkitd (expat) must load every declared action; DTD of polkit
        for pol in sorted(n for n in files if n.startswith("usr/share/polkit-1/actions/")):
            text = (root / pol).read_text()
            try:
                parsed = [a.get("id") for a in ET.fromstring(text).iter("action")]
            except ET.ParseError as e:
                f.add(pol, 0, "xml", f"polkitd cannot parse it, no action loads: {e}")
                continue
            declared = re.findall(r'<action id="([^"]+)"', re.sub(r"<!--.*?-->", "", text, flags=re.S))
            if parsed != declared:
                f.add(pol, 0, "polkit-actions", f"parsed {parsed} != declared {declared}")
            dtd = Path("/usr/share/polkit-1/policyconfig-1.dtd")
            if shutil.which("xmllint") and dtd.exists():
                r = subprocess.run(["xmllint", "--noout", "--nonet", "--dtdvalid", str(dtd), str(root / pol)],
                                   capture_output=True, text=True)
                if r.returncode:
                    f.add(pol, 0, "polkit-dtd", r.stderr.strip().splitlines()[-1][:200])
            else:
                f.add(pol, 0, "polkit-dtd", "xmllint or the polkit DTD missing: the policy is not validated")
        # .desktop files: what KDE / the menu / kglobalaccel / KRunner read
        for d in sorted(n for n in files if n.endswith(".desktop")):
            if not shutil.which("desktop-file-validate"):
                f.add(d, 0, "desktop", "desktop-file-validate missing: not validated")
                break
            r = subprocess.run(["desktop-file-validate", str(root / d)], capture_output=True, text=True)
            if r.returncode or r.stdout.strip() or r.stderr.strip():
                f.add(d, 0, "desktop", (r.stdout + r.stderr).strip()[:200])
        # the widget: valid JSON, a notification-area entry, its icon shipped
        for m in sorted(n for n in files if n.startswith("usr/share/plasma/plasmoids/") and n.endswith("/metadata.json")):
            try:
                meta = json.loads((root / m).read_text())
            except ValueError as e:
                f.add(m, 0, "json", f"plasmashell cannot read it: {e}")
                continue
            plugin = meta.get("KPlugin", {})
            if plugin.get("Id") != m.split("/")[4]:
                f.add(m, 0, "plasmoid-id", f"KPlugin.Id {plugin.get('Id')!r} != directory {m.split('/')[4]!r}")
            if meta.get("X-Plasma-NotificationArea") != "true":
                f.add(m, 0, "plasmoid-tray", "X-Plasma-NotificationArea must be \"true\": the widget is the notification-area icon")
            if not meta.get("X-Plasma-API-Minimum-Version"):
                f.add(m, 0, "plasmoid-api", "X-Plasma-API-Minimum-Version missing: Plasma 6 refuses the widget")
            icon = plugin.get("Icon", "")
            if not any(n.endswith(f"/{icon}.svg") for n in files):
                f.add(m, 0, "plasmoid-icon", f"icon {icon!r} is not shipped")
            ui = str(Path(m).parent / "contents/ui/main.qml")
            if ui not in files:
                f.add(m, 0, "plasmoid-main", f"{ui} missing: the widget cannot load")
        # D-Bus activation: Exec= must be a shipped executable
        for sv in sorted(n for n in files if n.startswith("usr/share/dbus-1/services/")):
            text = (root / sv).read_text()
            exe = re.search(r"(?m)^Exec=(\S+)", text)
            name = re.search(r"(?m)^Name=(\S+)", text)
            if not exe or exe.group(1).lstrip("/") not in files:
                f.add(sv, 0, "dbus-exec", f"Exec= {exe.group(1) if exe else '?'} is not a file of the package")
            if not name or f"{name.group(1)}.service" != Path(sv).name:
                f.add(sv, 0, "dbus-name", "Name= must match the file name")


def cmd_package(pkg: str) -> int:
    f = Findings("package-allow.tsv")
    expected = [l.strip() for l in (HERE / "package-expected.txt").read_text().splitlines()
                if l.strip() and not l.startswith("#")]
    listing = subprocess.run(["bsdtar", "-tvf", pkg], capture_output=True, text=True)
    if listing.returncode:
        print(listing.stderr)
        return 1
    names = {}
    for line in listing.stdout.splitlines():
        parts = line.split(None, 8)
        if len(parts) < 9:
            continue
        mode, name = parts[0], parts[8].split(" -> ")[0]
        names[name.rstrip("/")] = mode
    # #293: the package holds exactly the listed files (directories aside):
    # a missing main.qml, KCM .so or status icon fails, and so does a file
    # nobody listed (an untracked .qml picked up by a glob, a stray backup).
    files = {n for n, mode in names.items() if mode[0] != "d" and not n.startswith(".")}
    for e in sorted(set(expected) - files):
        f.add(pkg, 0, "missing", f"expected file not in the package: {e}")
    for e in sorted(files - set(expected)):
        f.add(pkg, 0, "unexpected", f"file in the package but not in scripts/package-expected.txt: {e}")
    dup = sorted({e for e in expected if expected.count(e) > 1})
    if dup:
        f.add("scripts/package-expected.txt", 0, "duplicate", f"listed twice: {dup}")
    for name, mode in names.items():
        if mode[0] != "l" and mode[8] == "w":
            f.add(name, 0, "world-writable", f"{mode} {name}")
        if mode[0] == "-" and ("s" in mode[3] + mode[6] or "S" in mode[3] + mode[6]):
            f.add(name, 0, "setuid", f"{mode} {name}: setuid/setgid shipped in the package")
    def member(n):
        return subprocess.run(["bsdtar", "-xOf", pkg, n], capture_output=True, text=True).stdout
    install = member(".INSTALL")
    if "setcap" not in install or "cap_net_admin" not in install:
        f.add(".INSTALL", 0, "setcap", "post_install must apply setcap cap_net_admin to rssi-helper")
    # #260: the units are enabled by the package's .wants links only; the
    # scriptlet enables nothing (its /etc links outlived the package) and
    # pre_remove drops any /etc link left by an older scriptlet.
    code = "\n".join(l for l in install.splitlines() if not l.lstrip().startswith("#") and "echo" not in l)
    if re.search(r"systemctl[^\n#]*\senable\b", code):
        f.add(".INSTALL", 0, "enable", "the scriptlet must not `systemctl enable` (the package ships the .wants links)")
    if not re.search(r"(?m)^pre_remove\(\)", install):
        f.add(".INSTALL", 0, "pre_remove", "pre_remove must drop the /etc/systemd links of the units")
    for name in names:
        if re.search(r"(\.bak|\.orig|\.pacsave|\.pacnew|~)([-.]|$)|mqtt-bridge", Path(name).name):
            f.add(name, 0, "leftover", f"backup or obsolete file shipped in the package: {name}")
    pkginfo = member(".PKGINFO")
    m = re.search(r"^pkgver = ([^-\n]+)-", pkginfo, re.M)
    cv = cargo_version()
    if not m or m.group(1) != cv:
        f.add(".PKGINFO", 0, "version", f"package version {m.group(1) if m else '?'} != Cargo {cv}")
    # #68: /etc/modprobe.d/hid_apple.conf is the file `akm-helper --persist`
    # rewrites. It must be in the package AND declared backup, or an upgrade
    # replaces the Fn mode the user persisted; one active line, fnmode=1, the
    # value post_install applies.
    conf = "etc/modprobe.d/hid_apple.conf"
    if conf in names:
        if not re.search(rf"(?m)^backup = {re.escape(conf)}$", pkginfo):
            f.add(".PKGINFO", 0, "modprobe-backup", f"{conf} is shipped without backup= (the persisted Fn mode would be overwritten)")
        active = [l.strip() for l in member(conf).splitlines() if l.strip() and not l.lstrip().startswith("#")]
        if active != ["options hid_apple fnmode=1"]:
            f.add(conf, 0, "modprobe-content", f"active lines must be exactly 'options hid_apple fnmode=1', found {active}")
        if "#68" not in member(conf):
            f.add(conf, 0, "modprobe-why", "the shipped file must say why it is still shipped (#68)")
        if names[conf][:10] != "-rw-r--r--":
            f.add(conf, 0, "modprobe-mode", f"{names[conf]} {conf}: must be 0644")
    if re.search(r"(?m)^depend = python", pkginfo):
        f.add(".PKGINFO", 0, "python", "the package must not depend on Python")
    package_content(pkg, files, f)
    return f.done(f"package {Path(pkg).name} ({len(names)} entries)")


# ── report ──────────────────────────────────────────────────────────────────

def cmd_report(out_dir: str) -> int:
    d = Path(out_dir)
    steps = []
    for line in (d / "steps.tsv").read_text().splitlines():
        name, status, secs, required, log, summary = (line.split("\t") + [""] * 6)[:6]
        steps.append({"step": name, "status": status, "seconds": float(secs or 0), "required": required == "1",
                      "log": log, "summary": summary})
    failed = [s["step"] for s in steps if s["status"] == "fail" and s["required"]]
    meta = json.loads((d / "meta.json").read_text()) if (d / "meta.json").exists() else {}
    rep = {"tool": "scripts/ci-local.sh", **meta, "ok": not failed, "failed": failed, "steps": steps}
    (d / "report.json").write_text(json.dumps(rep, indent=2, ensure_ascii=False) + "\n")
    w = max(len(s["step"]) for s in steps) if steps else 10
    lines = [f"ci-local {meta.get('mode', '')} {meta.get('commit', '')} {meta.get('started', '')}", ""]
    for s in steps:
        tag = {"pass": " ok ", "fail": " KO " if s["required"] else "warn", "skip": "skip", "warn": "warn"}.get(s["status"], s["status"])
        lines.append(f"[{tag}] {s['step']:<{w}} {s['seconds']:7.1f}s  {s['summary']}")
    lines += ["", "RESULT: " + ("OK" if not failed else "FAILED: " + ", ".join(failed))]
    (d / "report.txt").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    return 0 if not failed else 1


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 64
    cmd, args = sys.argv[1], sys.argv[2:]
    if cmd == "secrets":
        return cmd_secrets([a for a in args if a != "--paths"])
    if cmd == "claims":
        return cmd_claims()
    if cmd == "redaction":
        return cmd_redaction()
    if cmd == "package" and args:
        return cmd_package(args[0])
    if cmd == "report" and args:
        return cmd_report(args[0])
    print(__doc__)
    return 64


if __name__ == "__main__":
    sys.exit(main())
