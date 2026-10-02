#!/usr/bin/env bash
# PKGBUILD / .SRCINFO (#14): the build uses the locked dependencies, every
# source has a real checksum that matches the file, and .SRCINFO is the one
# makepkg generates from the PKGBUILD; the modprobe file is shipped as a
# backup= file and says why (#68). Static: builds nothing, downloads
# nothing, writes nothing. Exit 0 = all checks pass.
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
body build | grep -qE 'cargo build .*--locked' || no "build() does not run cargo build --locked"
[ -f "$top/apihub-app/Cargo.lock" ] || no "apihub-app/Cargo.lock is missing"
git -C "$top" check-ignore -q apihub-app/Cargo.lock 2>/dev/null && no "apihub-app/Cargo.lock is ignored by git"

# 2. sources and checksums: same count, no SKIP, each sum is the file's
mapfile -t sources < <(bash -c 'source "$1" >/dev/null 2>&1; printf "%s\n" "${source[@]}"' _ "$pk")
mapfile -t sums < <(bash -c 'source "$1" >/dev/null 2>&1; printf "%s\n" "${sha256sums[@]}"' _ "$pk")
[ "${#sources[@]}" -ge 1 ] || no "no source"
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

# 4. what cannot be done without a published source repository is written down
grep -q 'no published source archive' "$pk" || no "the PKGBUILD no longer says why it builds from \$startdir"

# 5. modprobe/hid_apple.conf (#68): still shipped, as a backup= file, with
#    exactly one active line and the reason it is kept written in it
mp="$top/modprobe/hid_apple.conf"
active=$(grep -vE '^[[:space:]]*(#|$)' "$mp")
[ "$active" = "options hid_apple fnmode=1" ] || no "modprobe/hid_apple.conf: active lines are not exactly 'options hid_apple fnmode=1': $active"
grep -q '^# Why this file is still shipped (#68)' "$mp" || no "modprobe/hid_apple.conf no longer says why it is shipped"
grep -q -- '--persist' "$mp" || no "modprobe/hid_apple.conf no longer names akmctl set fnmode --persist"
body package | grep -qF 'modprobe/hid_apple.conf"' || no "package() does not install modprobe/hid_apple.conf"
bash -c 'source "$1" >/dev/null 2>&1; printf "%s\n" "${backup[@]}"' _ "$pk" | grep -qxF 'etc/modprobe.d/hid_apple.conf' \
  || no "etc/modprobe.d/hid_apple.conf is not in backup=() (an upgrade would overwrite the persisted Fn mode)"
grep -qF 'const MODPROBE_CONF: &str = "/etc/modprobe.d/hid_apple.conf";' "$top/apihub-app/crates/akm-helper/src/main.rs" \
  || no "akm-helper no longer persists into /etc/modprobe.d/hid_apple.conf: the reason to ship the file is gone, see #68"

[ $fail = 0 ] && echo "PKGBUILD: cargo --locked, ${#sources[@]} source(s) with a matching sha256, .SRCINFO in sync, modprobe file kept as backup="
exit $fail
