#!/usr/bin/env bash
# The pacman scriptlet apple-kb-monitor.install (#260), run against a fake
# root with stub commands: it enables nothing, removes the /etc/systemd links
# of our units only when there are some (pre_remove, upgrade from 3.1.0-16),
# and removes the mqtt-bridge.py leftovers only when no package owns them.
# Its notices read a fake /proc: per user, the group akm (#269), what still
# runs the old version after an upgrade (#294), an /etc udev rule hiding the
# packaged one (#296), the links left in ~/.config/systemd/user (#298).
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
for c in systemctl pacman getent; do
  cat > "$tmp/bin/$c" <<STUB
#!/bin/sh
echo "$c \$*" >> "$tmp/calls.log"
if [ "$c" = pacman ]; then grep -qxF "\$2" "$tmp/owned.txt" 2>/dev/null; exit \$?; fi
if [ "$c" = getent ]; then
  if [ -z "\${2:-}" ]; then cat "$tmp/\$1" 2>/dev/null; exit 0; fi
  awk -F: -v k="\$2" '\$1 == k || \$3 == k' "$tmp/\$1" 2>/dev/null | grep . ; exit \$?
fi
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

# 6. notices from a fake /proc (nothing in it is ever written to)
r4="$tmp/r4"
proc() {  # proc <pid> <uid> <comm> <exe> <cmdline> [groups]
  local d="$r4/proc/$1"; mkdir -p "$d"
  printf 'Name:\t%s\nUid:\t%s\t%s\t%s\t%s\nGroups:\t%s\n' "$3" "$2" "$2" "$2" "$2" "${6:-}" > "$d/status"
  echo "$3" > "$d/comm"; printf '%s\0' $5 > "$d/cmdline"; ln -s "$4" "$d/exe"
}
printf 'root:x:0:0::/root:/bin/sh\nalice:x:1000:1000::/home/alice:/bin/bash\nbob:x:1001:1001::/home/bob:/bin/bash\n' > "$tmp/passwd"
printf 'akm:x:934:bob\n' > "$tmp/group"
proc 11 1000 systemd /usr/lib/systemd/systemd "/usr/lib/systemd/systemd --user" "998 1000"
proc 12 1001 systemd /usr/lib/systemd/systemd "/usr/lib/systemd/systemd --user --deserialize=12" "998 1001"
proc 13 1000 apple-kb-monitord "/usr/bin/apple-kb-monitord (deleted)" /usr/bin/apple-kb-monitord
proc 14 1000 apihub-app "/usr/bin/apihub-app (deleted)" "/usr/bin/apihub-app --krunner"
proc 15 1000 plasmashell /usr/bin/plasmashell /usr/bin/plasmashell
proc 16 1001 apple-kb-monitord /usr/bin/apple-kb-monitord /usr/bin/apple-kb-monitord
proc 17 0 systemd /usr/lib/systemd/systemd "/usr/lib/systemd/systemd --system"
mkdir -p "$r4/var/lib/systemd/linger"; touch "$r4/var/lib/systemd/linger/bob"
run "$r4" _akm_group_notice
grep -q 'alice is not in the group' "$tmp/out.log" && grep -q 'sudo usermod -aG akm alice$' "$tmp/out.log" \
  || no "alice (not in akm): no usermod command: $(cat "$tmp/out.log")"
grep -q "bob is in the group 'akm', but bob's services" "$tmp/out.log" || no "bob (in akm, old manager): not told"
grep -q 'sudo systemctl restart user@1001.service' "$tmp/out.log" || no "bob has Linger=yes: restart of user@1001 not given"
grep -q 'every session of alice is closed' "$tmp/out.log" || no "alice (no linger): log out / in not given"
grep -q 'usermod -aG akm bob' "$tmp/out.log" && no "bob is already a member: no usermod for him"
grep -q 'root' "$tmp/out.log" && no "root's (system) manager taken for a user"
sed -i 's/^Groups:.*/Groups:\t934 998 1001/' "$r4/proc/12/status"
run "$r4" _akm_group_notice
grep -q bob "$tmp/out.log" && no "bob's manager has the group: nothing to say about bob"
run "$r4" _akm_stale_notice
grep -q 'previous version, for alice' "$tmp/out.log" || no "stale processes of alice not listed: $(cat "$tmp/out.log")"
grep -q 'systemctl --user restart apple-kb-monitord.service' "$tmp/out.log" || no "stale daemon not told"
grep -q 'pkill -x apihub-app' "$tmp/out.log" || no "stale apihub-app not told"
grep -q 'restart plasma-plasmashell.service' "$tmp/out.log" || no "plasmashell restart not told"
grep -q 'for bob' "$tmp/out.log" && no "bob runs the new daemon and no plasmashell: nothing to say"
grep -E 'restart|pkill' "$tmp/calls.log" && no "the notice restarted something itself"
run "$r4" _akm_udev_override_notice
[ -s "$tmp/out.log" ] && no "no /etc udev rule: nothing to say"
mkdir -p "$r4/etc/udev/rules.d" "$r4/usr/lib/udev/rules.d"
echo pkg > "$r4/usr/lib/udev/rules.d/70-apple-kb-hidraw.rules"
echo pkg > "$r4/etc/udev/rules.d/70-apple-kb-hidraw.rules"
run "$r4" _akm_udev_override_notice
[ -s "$tmp/out.log" ] && no "an identical /etc copy is harmless: nothing to say"
echo 'KERNELS=="0005:05AC:*", TAG+="uaccess"' > "$r4/etc/udev/rules.d/70-apple-kb-hidraw.rules"
run "$r4" _akm_udev_override_notice
grep -q 'replaces the packaged udev rule' "$tmp/out.log" || no "an /etc rule hiding the packaged one is not reported"
[ -e "$r4/etc/udev/rules.d/70-apple-kb-hidraw.rules" ] || no "the /etc rule was removed (it may be deliberate)"
mkdir -p "$r4/home/alice/.config/systemd/user/default.target.wants"
ln -s /usr/lib/systemd/user/apple-kb-monitord.service "$r4/home/alice/.config/systemd/user/default.target.wants/"
run "$r4" _akm_user_links_notice
grep -q 'rm /home/alice/.config/systemd/user/default.target.wants/apple-kb-monitord.service' "$tmp/out.log" \
  || no "dangling user link of alice not given: $(cat "$tmp/out.log")"
grep -q bob "$tmp/out.log" && no "bob has no link: nothing to say"

# 7. the PKGBUILD ships the .wants link of every unit the scriptlet used to enable
for u in apple-kb-monitord.service apple-kb-monitor-shutdown.service apple-kb-monitor-selfcheck.timer \
         apple-kb-monitor-suspend.service apple-kb-monitor-resume.service; do
  grep -qE "ln -s \.\./$u " "$top/PKGBUILD" || no "PKGBUILD ships no .wants link for $u"
done

# 8. the PKGBUILD itself (#14: locked build, real checksums, .SRCINFO in sync)
bash "$top/tests/check-pkgbuild.sh" || no "tests/check-pkgbuild.sh"

[ $fail = 0 ] && echo "install scriptlet: no enable, /etc links dropped on remove/upgrade, orphans cleaned, notices (group, stale processes, udev override, user links) right"
exit $fail
