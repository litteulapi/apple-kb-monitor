#!/usr/bin/env bash
# Builds the Arch package in a fresh archlinux container, as root: installs the
# PKGBUILD's dependencies, clones the tree for an unprivileged build user
# (makepkg refuses root), runs the essential checks, builds, checks the
# package content and writes OUT_DIR/<pkg>.pkg.tar.zst + .sha256.
#
# GitHub Actions (.github/workflows/package.yml) runs it as is; by hand:
#   podman run --rm -v "$PWD":/src:ro -v "$PWD/dist":/out archlinux:latest \
#       bash /src/scripts/release-build.sh /out
# (the mounted tree must be a git clone; only its committed state is built)
set -euo pipefail
src=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
out=${1:-/out}
[ "$(id -u)" = 0 ] || { echo "run as root in a throwaway container" >&2; exit 1; }

deps=$(bash -c '. "$1"; echo "${depends[*]} ${makedepends[*]}"' _ "$src/PKGBUILD")
read -ra deps <<<"$deps"
pacman -Syu --noconfirm --needed base-devel git python desktop-file-utils libxml2 "${deps[@]}"

git config --global --add safe.directory '*'
id builder >/dev/null 2>&1 || useradd -m builder
rm -rf /home/builder/apple-kb-monitor
git clone -q --no-local "$src" /home/builder/apple-kb-monitor
chown -R builder: /home/builder/apple-kb-monitor
cd /home/builder/apple-kb-monitor
echo "building commit $(git rev-parse --short HEAD)"

run() { su builder -s /bin/bash -c "cd /home/builder/apple-kb-monitor && $*"; }
run "bash tests/check-pkgbuild.sh"
run "sh tests/check-trademarks.sh"
run "sh tests/check-security-files.sh"
run "bash tests/check-install-scriptlet.sh"
run "python3 scripts/qa_checks.py secrets"
run "PKGDEST=/home/builder/pkgs makepkg -f --noconfirm --nosign"

pkg=$(printf '%s\n' /home/builder/pkgs/*.pkg.tar.zst | grep -v -- '-debug-' | head -1)
python3 scripts/qa_checks.py package "$pkg"
mkdir -p "$out"
cp "$pkg" "$out/"
(cd "$out" && sha256sum "$(basename "$pkg")" > "$(basename "$pkg").sha256" && cat ./*.sha256)
