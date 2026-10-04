#!/usr/bin/env bash
# PKGBUILD / .SRCINFO: the build uses the locked dependencies, every
# source has a real checksum that matches the file, and .SRCINFO is the one
# makepkg generates from the PKGBUILD; the modprobe file is shipped as a
# backup= file and says why; prepare() refuses a tree that is not a
# commit. Builds nothing, downloads nothing; writes only throw-away git
# repositories under $TMPDIR. Exit 0 = all checks pass.
set -uo pipefail
top=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
pk="$top/PKGBUILD"
fail=0
no() { echo "FAIL: $*"; fail=1; }
# the PKGBUILD without its comments
code=$(sed -e 's/^[[:space:]]*#.*$//' "$pk")
# body of one function of the PKGBUILD
body() { printf '%s\n' "$code" | awk -v f="$1" '$0 ~ "^"f"\\(\\)" {on=1} on {print} on && /^}/ {exit}'; }

# 1. locked dependencies: every cargo call that resolves them says --locked
#    (or --frozen), and they are fetched in prepare()
while IFS= read -r line; do
  case "$line" in
    *--locked*|*--frozen*) ;;
    *) no "cargo call without --locked: $(echo "$line" | xargs)" ;;
  esac
done < <(printf '%s\n' "$code" | grep -E '\bcargo[[:space:]]+(build|fetch|test|check|install|run)\b')
body prepare | grep -qE 'cargo fetch .*--locked' || no "prepare() does not run cargo fetch --locked"
body build | grep -qE 'cargo build .*--(locked|frozen)' || no "build() does not run cargo build --locked/--frozen"
[ -f "$top/apihub-app/Cargo.lock" ] || no "apihub-app/Cargo.lock is missing"
git -C "$top" check-ignore -q apihub-app/Cargo.lock 2>/dev/null && no "apihub-app/Cargo.lock is ignored by git"

# 2. sources and checksums: same count, no SKIP, each sum is the file's
mapfile -t sources < <(bash -c 'source "$1" >/dev/null 2>&1; [ ${#source[@]} = 0 ] || printf "%s\n" "${source[@]}"' _ "$pk")
mapfile -t sums < <(bash -c 'source "$1" >/dev/null 2>&1; [ ${#sha256sums[@]} = 0 ] || printf "%s\n" "${sha256sums[@]}"' _ "$pk")
# every file is installed from $startdir: an empty source=() is valid
[ "${#sources[@]}" = "${#sums[@]}" ] || no "${#sources[@]} source(s) but ${#sums[@]} sha256sum(s)"
for i in "${!sources[@]}"; do
  s=${sources[$i]} sum=${sums[$i]:-}
  case "$s" in
    *://*) echo "$sum" | grep -qE '^[0-9a-f]{64}$' || no "remote source $s without a real sha256 ($sum)" ;;
    *)
      [ -f "$top/$s" ] || { no "local source $s is not in the tree"; continue; }
      real=$(sha256sum "$top/$s" | cut -d' ' -f1)
      [ "$sum" = "$real" ] || no "sha256 of $s is $real, the PKGBUILD says $sum (run updpkgsums)"
      ;;
  esac
done
printf '%s\n' "$code" | grep -qE "SKIP" && no "a checksum is SKIP"

# 3. .SRCINFO: generated from this PKGBUILD (version, release, sums, all of it)
if command -v makepkg >/dev/null 2>&1; then
  if ! (cd "$top" && makepkg --printsrcinfo 2>/dev/null) | diff -u "$top/.SRCINFO" - >/dev/null; then
    no ".SRCINFO differs from makepkg --printsrcinfo (run: makepkg --printsrcinfo > .SRCINFO)"
  fi
else
  # no makepkg here: at least the fields that drift
  for k in pkgver pkgrel; do
    v=$(bash -c 'source "$1" >/dev/null 2>&1; eval "printf %s \"\$$2\""' _ "$pk" "$k")
    grep -qE "^[[:space:]]*$k = $v\$" "$top/.SRCINFO" || no ".SRCINFO: $k is not $v"
  done
  for sum in "${sums[@]}"; do
    grep -qF "sha256sums = $sum" "$top/.SRCINFO" || no ".SRCINFO: sha256sums = $sum missing"
  done
fi

# 3b. run-time depends cover what the shipped QML imports and where icons go
deps=$(bash -c 'source "$1" >/dev/null 2>&1; printf "%s\n" "${depends[@]}"' _ "$pk")
need() { printf '%s\n' "$deps" | grep -qxF "$1" || no "depends lacks $1 ($2)"; }
imports=$(cat "$top"/plasma/*/contents/ui/*.qml "$top"/kcm/ui/*.qml 2>/dev/null | grep -oE '^import org\.kde\.[a-z.]+' | sort -u)
grep -qE 'org\.kde\.plasma\.(plasmoid|core|components|extras)$' <<<"$imports" && need libplasma "QML org.kde.plasma.*"
grep -qx 'import org.kde.plasma.workspace.dbus' <<<"$imports" && need plasma-workspace "QML org.kde.plasma.workspace.dbus"
grep -qx 'import org.kde.kirigami' <<<"$imports" && need kirigami "QML org.kde.kirigami"
body package | grep -q 'share/icons/hicolor/' && need hicolor-icon-theme "icons installed under hicolor"

# 3c. package() only installs (no compiler, no msgfmt) and makedepends lists
#     no base-devel member (Arch packaging guidelines)
body package | grep -qE '\b(msgfmt|gcc|cc|cargo|cmake --build)\b' && no "package() builds something: move it to build()"
mkdeps=$(bash -c 'source "$1" >/dev/null 2>&1; printf "%s\n" "${makedepends[@]}"' _ "$pk")
for b in gcc gettext make binutils pkgconf; do
  grep -qxF "$b" <<<"$mkdeps" && no "makedepends lists $b, already in base-devel"
done

# 4. why it builds from $startdir, and where the clone-based variant is
grep -q 'packaging/aur-git' "$pk" || no "the PKGBUILD no longer points to its clone-based variant packaging/aur-git"
python3 "$top/scripts/gen-aur-git.py" --check >/dev/null || no "packaging/aur-git is stale: run scripts/gen-aur-git.py"

# 5. modprobe/hid_apple.conf: still shipped, as a backup= file, with
#    exactly one active line and the reason it is kept written in it
mp="$top/modprobe/hid_apple.conf"
active=$(grep -vE '^[[:space:]]*(#|$)' "$mp")
[ "$active" = "options hid_apple fnmode=1" ] || no "modprobe/hid_apple.conf: active lines are not exactly 'options hid_apple fnmode=1': $active"
grep -q '^# Why this file is still shipped' "$mp" || no "modprobe/hid_apple.conf no longer says why it is shipped"
grep -q -- '--persist' "$mp" || no "modprobe/hid_apple.conf no longer names akmctl set fnmode --persist"
body package | grep -qF 'modprobe/hid_apple.conf"' || no "package() does not install modprobe/hid_apple.conf"
bash -c 'source "$1" >/dev/null 2>&1; printf "%s\n" "${backup[@]}"' _ "$pk" | grep -qxF 'etc/modprobe.d/hid_apple.conf' \
  || no "etc/modprobe.d/hid_apple.conf is not in backup=() (an upgrade would overwrite the persisted Fn mode)"
grep -qF 'const MODPROBE_CONF: &str = "/etc/modprobe.d/hid_apple.conf";' "$top/apihub-app/crates/akm-helper/src/main.rs" \
  || no "akm-helper no longer persists into /etc/modprobe.d/hid_apple.conf: the reason to ship the file is gone, see modprobe/hid_apple.conf"

# 6. a package is the image of one commit: prepare() calls the guard
#    first, which refuses a non-git tree and any change or untracked file,
#    unless AKM_ALLOW_DIRTY=1 is set explicitly.
body prepare | sed -n '2p' | grep -q '_akm_require_clean_tree' || no "prepare() does not start with _akm_require_clean_tree"
g=$(mktemp -d "${TMPDIR:-/tmp}/akm-pkgbuild-test.XXXXXX")
guard() {  # guard <startdir> [env...]: exit code of _akm_require_clean_tree
  local d=$1; shift
  # shellcheck disable=SC2016  # expanded by the inner shell
  env "$@" bash -c 'error() { :; }; plain() { :; }; warning() { :; }; msg2() { :; }
    eval "$(sed -n "/^_akm_require_clean_tree() {/,/^}/p" "$1")"; startdir=$2; _akm_require_clean_tree' _ "$pk" "$d" >/dev/null 2>&1
}
mkdir -p "$g/plain" "$g/repo"
git -C "$g/repo" init -q && echo a > "$g/repo/f" && git -C "$g/repo" add f \
  && git -C "$g/repo" -c user.name=t -c user.email=t@t.invalid commit -qm t
guard "$g/plain" && no "the guard accepts a tree that is not a git work tree"
guard "$g/plain" AKM_ALLOW_DIRTY=1 || no "AKM_ALLOW_DIRTY=1 does not lift the guard"
guard "$g/repo" || no "the guard refuses a clean commit"
mkdir -p "$g/repo/src" "$g/repo/pkg" && echo x > "$g/repo/src/x" && echo x > "$g/repo/pkg/x"
guard "$g/repo" || no "the guard counts makepkg's own src/ and pkg/ as changes"
echo b > "$g/repo/f"
guard "$g/repo" && no "the guard accepts a modified tracked file"
git -C "$g/repo" checkout -q f && echo n > "$g/repo/new.qml"
guard "$g/repo" && no "the guard accepts an untracked file (package() globs would ship it)"
mkdir -p "$g/repo/sub"; guard "$g/repo/sub" && no "the guard accepts a subdirectory of a work tree"
rm -rf "$g"

[ $fail = 0 ] && echo "PKGBUILD: clean-tree guard, cargo --locked, ${#sources[@]} source(s) with a matching sha256, .SRCINFO in sync, modprobe file kept as backup="
exit $fail
