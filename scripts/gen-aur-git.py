#!/usr/bin/env python3
"""Writes packaging/aur-git/ (PKGBUILD, .SRCINFO, apple-kb-monitor.install) for
the AUR package apple-kb-monitor-git, derived from the root PKGBUILD so that
both install exactly the same files: the build reads the git clone in $srcdir
instead of $startdir. --check fails when the committed files are stale."""
import pathlib
import re
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "packaging/aur-git"
REPO = "https://github.com/litteulapi/apple-kb-monitor"
CLONE = '"$srcdir/apple-kb-monitor'

src = (ROOT / "PKGBUILD").read_text()
head, sep, body = src.partition("\nsource=(")
assert sep, "source=( not found"
body = "\n" + body.split("\nsha256sums=(", 1)[1].split(")\n", 1)[1]  # drop source and checksums
# the clone of a commit is clean by construction: no guard needed
body = re.sub(r"\n# A package must be the image of ONE commit.*?\n}\n", "\n", body, flags=re.S)
body = body.replace("    _akm_require_clean_tree\n", "")
head = re.sub(r"\n# makepkg only resolves local sources.*", "", head, flags=re.S)
head = head.replace("pkgname=apple-kb-monitor", "pkgname=apple-kb-monitor-git")
head = re.sub(r"\npkgrel=\d+", "\npkgrel=1", head)
head = re.sub(r'url="[^"]*"', f'url="{REPO}"', head)
pkgbuild = head + f"""
provides=('apple-kb-monitor')
conflicts=('apple-kb-monitor')
source=("apple-kb-monitor::git+{REPO}.git")
sha256sums=('SKIP')

pkgver() {{
    cd "$srcdir/apple-kb-monitor"
    printf '%s.r%s.g%s' \\
        "$(sed -n '/^\\[workspace.package\\]/,/^\\[/s/^version *= *"\\(.*\\)"/\\1/p' apihub-app/Cargo.toml | head -1)" \\
        "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"
}}
""" + body
pkgbuild = pkgbuild.replace('"$startdir', CLONE)
header = "# Generated from the PKGBUILD of the repository by scripts/gen-aur-git.py: edit that one.\n"
pkgbuild = header + pkgbuild
assert "$startdir" not in pkgbuild

check = "--check" in sys.argv
tmp = OUT if not check else pathlib.Path(subprocess.check_output(["mktemp", "-d"], text=True).strip())
tmp.mkdir(parents=True, exist_ok=True)
(tmp / "PKGBUILD").write_text(pkgbuild)
shutil.copy(ROOT / "apple-kb-monitor.install", tmp / "apple-kb-monitor.install")
if shutil.which("makepkg"):
    info = subprocess.run(["makepkg", "--printsrcinfo"], cwd=tmp, capture_output=True, text=True, check=True).stdout
    (tmp / ".SRCINFO").write_text(info)
if check:
    bad = [f for f in ("PKGBUILD", ".SRCINFO", "apple-kb-monitor.install")
           if (tmp / f).exists() and (not (OUT / f).exists() or (OUT / f).read_text() != (tmp / f).read_text())]
    shutil.rmtree(tmp)
    if bad:
        print("packaging/aur-git is stale (" + ", ".join(bad) + "): run scripts/gen-aur-git.py")
        sys.exit(1)
    print("packaging/aur-git matches the PKGBUILD")
else:
    print(f"written: {OUT.relative_to(ROOT)}/{{PKGBUILD,.SRCINFO,apple-kb-monitor.install}}")
