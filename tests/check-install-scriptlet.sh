#!/usr/bin/env bash
# The pacman scriptlet apple-kb-monitor.install (#260), run against a fake
# root with stub commands: it enables nothing, removes the /etc/systemd links
# of our units only when there are some (pre_remove, upgrade from 3.1.0-16),
# and removes the mqtt-bridge.py leftovers only when no package owns them.
# Exit 0 = all checks pass. Touches nothing outside a temporary directory.
set -uo pipefail
top=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/akm-install-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail=0
no() { echo "FAIL: $*"; fail=1; }

# 1. static: no `enable` of any unit anywhere in the scriptlet
if grep -vE '^[[:space:]]*#' "$top/apple-kb-monitor.install" | grep -v 'echo' | grep -E 'systemctl[^#]*[[:space:]]enable'; then
  no "the scriptlet enables a unit (duplicates the package .wants links under /etc)"
fi
grep -q '^pre_remove()' "$top/apple-kb-monitor.install" || no "no pre_remove()"

# 2. stubs: every call is logged, pacman -Qqo answers from a list
mkdir -p "$tmp/bin"
for c in systemctl pacman; do
  cat > "$tmp/bin/$c" <<STUB
#!/bin/sh
echo "$c \$*" >> "$tmp/calls.log"
if [ "$c" = pacman ]; then grep -qxF "\$2" "$tmp/owned.txt" 2>/dev/null; exit \$?; fi
exit 0
STUB
  chmod +x "$tmp/bin/$c"
done
run() {  # run <root> <function>...
  : > "$tmp/calls.log"
  PATH="$tmp/bin:$PATH" bash -c '. "$1"; _akm_root=$2; shift 2; for f in "$@"; do "$f"; done' \
    _ "$top/apple-kb-monitor.install" "$@" > "$tmp/out.log" 2>&1
}

# 3. no /etc link: pre_remove calls nothing
r1="$tmp/r1"; mkdir -p "$r1/etc/systemd/system" "$r1/etc/systemd/user"
run "$r1" pre_remove
[ -s "$tmp/calls.log" ] && no "pre_remove without /etc links called: $(cat "$tmp/calls.log")"

# 4. the /etc links of 3.1.0-16: disabled (system + --global), nothing enabled
r2="$tmp/r2"
mkdir -p "$r2/etc/systemd/system/sleep.target.wants" "$r2/etc/systemd/user/default.target.wants"
ln -s /usr/lib/systemd/system/apple-kb-monitor-suspend.service "$r2/etc/systemd/system/sleep.target.wants/"
ln -s /usr/lib/systemd/user/apple-kb-monitord.service "$r2/etc/systemd/user/default.target.wants/"
run "$r2" pre_remove
grep -qx 'systemctl disable apple-kb-monitor-suspend.service apple-kb-monitor-resume.service' "$tmp/calls.log" \
  || no "system units not disabled: $(cat "$tmp/calls.log")"
grep -qx 'systemctl --global disable apple-kb-monitord.service apple-kb-monitor-shutdown.service apple-kb-monitor-selfcheck.timer' "$tmp/calls.log" \
  || no "user units not disabled globally: $(cat "$tmp/calls.log")"
grep 'enable' "$tmp/calls.log" | grep -qv disable && no "something enabled"
# only a user link: only --global disable
rm "$r2/etc/systemd/system/sleep.target.wants/apple-kb-monitor-suspend.service"
run "$r2" _akm_drop_etc_links
[ "$(grep -c . "$tmp/calls.log")" = 1 ] && grep -q -- '--global disable' "$tmp/calls.log" \
  || no "only the user link should be handled: $(cat "$tmp/calls.log")"

# 5. orphans: removed when no package owns them, kept otherwise; others untouched
r3="$tmp/r3"; d="$r3/usr/lib/apple-kb-monitor"; mkdir -p "$d"
for f in mqtt-bridge.py mqtt-bridge.py.bak-20260821-100929 akm-helper; do echo x > "$d/$f"; done
run "$r3" _akm_drop_orphans
[ -e "$d/mqtt-bridge.py" ] && no "orphan mqtt-bridge.py kept"
[ -e "$d/mqtt-bridge.py.bak-20260821-100929" ] && no "orphan .bak kept"
[ -e "$d/akm-helper" ] || no "akm-helper removed"
grep -q 'removed a leftover owned by no package' "$tmp/out.log" || no "removal not reported"
echo x > "$d/mqtt-bridge.py"; echo "$d/mqtt-bridge.py" > "$tmp/owned.txt"
run "$r3" _akm_drop_orphans
[ -e "$d/mqtt-bridge.py" ] || no "a file owned by a package was removed"

# 6. the PKGBUILD ships the .wants link of every unit the scriptlet used to enable
for u in apple-kb-monitord.service apple-kb-monitor-shutdown.service apple-kb-monitor-selfcheck.timer \
         apple-kb-monitor-suspend.service apple-kb-monitor-resume.service; do
  grep -qE "ln -s \.\./$u " "$top/PKGBUILD" || no "PKGBUILD ships no .wants link for $u"
done

[ $fail = 0 ] && echo "install scriptlet: no enable, /etc links dropped on remove/upgrade, orphans cleaned"
exit $fail
