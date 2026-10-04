#!/usr/bin/env bash
# The pacman scriptlet apple-kb-monitor.install, run against a fake
# root with stub commands: it enables nothing, removes the /etc/systemd links
# of our units only when there are some (pre_remove, upgrade from 3.1.0-16),
# and removes the mqtt-bridge.py leftovers only when no package owns them.
# It starts, restarts and enables no unit and writes no sysfs (user units come
# with a preset, the enable command is printed), no group is managed;
# from a fake /proc: plasmashell is told to reload the widget, an /etc udev
# rule hiding the packaged one, the links left in ~/.config/systemd/user.
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
# kde-apply binds F4 only (akm-core keymap::kde BINDINGS)
if grep -iE 'bind.*Eject|Eject to' "$top/apple-kb-monitor.install" | grep -v '^[[:space:]]*#'; then
  no "a hint promises an Eject binding that akmctl keymap kde-apply does not make"
fi
# `akmctl history import` takes a required FILE
if grep -E 'akmctl history import[[:space:]]*["'"'"']?[[:space:]]*$' "$top/apple-kb-monitor.install"; then
  no "a hint gives 'akmctl history import' without its FILE argument"
fi

# 2. stubs: every call is logged, pacman -Qqo answers from a list
mkdir -p "$tmp/bin"
for c in systemctl pacman getent udevadm; do
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
{ [ "$(grep -c . "$tmp/calls.log")" = 1 ] && grep -q -- '--global disable' "$tmp/calls.log"; } \
  || no "only the user link should be handled: $(cat "$tmp/calls.log")"
# install/upgrade keep an admin's `--global enable` of the user units
: > "$tmp/calls.log"
PATH="$tmp/bin:$PATH" bash -c '. "$1"; _akm_root=$2; _akm_drop_etc_links system' _ "$top/apple-kb-monitor.install" "$r2" >/dev/null 2>&1
grep -q -- '--global' "$tmp/calls.log" && no "install/upgrade dropped a global enable of the user units"

# 4b. post_install / post_upgrade: no systemctl call at all, no sysfs write, the enable command printed
grep -vE '^[[:space:]]*#' "$top/apple-kb-monitor.install" | grep -q '/sys/module' && no "the scriptlet writes sysfs"
: > "$tmp/calls.log"
PATH="$tmp/bin:$PATH" bash -c '. "$1"; _akm_root=$2; post_install' _ "$top/apple-kb-monitor.install" "$r1" > "$tmp/out.log" 2>&1
grep -q '^systemctl' "$tmp/calls.log" && no "post_install called systemctl: $(cat "$tmp/calls.log")"
grep -q 'systemctl --user enable --now apple-kb-monitord.service' "$tmp/out.log" || no "post_install does not print the enable command"
: > "$tmp/calls.log"
PATH="$tmp/bin:$PATH" bash -c '. "$1"; _akm_root=$2; post_upgrade 3.2.0-3 3.2.0-2' _ "$top/apple-kb-monitor.install" "$r1" > "$tmp/out.log" 2>&1
grep -q '^systemctl' "$tmp/calls.log" && no "post_upgrade called systemctl: $(cat "$tmp/calls.log")"
grep -q 'no longer enabled by the package' "$tmp/out.log" || no "upgrade from the .wants links: enable hint missing"
PATH="$tmp/bin:$PATH" bash -c '. "$1"; _akm_root=$2; post_upgrade 3.2.0-4 3.2.0-3' _ "$top/apple-kb-monitor.install" "$r1" > "$tmp/out.log" 2>&1
grep -q 'no longer enabled' "$tmp/out.log" && no "the enable hint repeats after the preset upgrade"

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
  echo "$3" > "$d/comm"; read -ra argv <<<"$5"; printf '%s\0' "${argv[@]}" > "$d/cmdline"; ln -s "$4" "$d/exe"
}
printf 'root:x:0:0::/root:/bin/sh\nalice:x:1000:1000::/home/alice:/bin/bash\nbob:x:1001:1001::/home/bob:/bin/bash\n' > "$tmp/passwd"
proc 11 1000 systemd /usr/lib/systemd/systemd "/usr/lib/systemd/systemd --user" "998 1000"
proc 12 1001 systemd /usr/lib/systemd/systemd "/usr/lib/systemd/systemd --user --deserialize=12" "998 1001"
proc 13 1000 apple-kb-monitord "/usr/bin/apple-kb-monitord (deleted)" /usr/bin/apple-kb-monitord
proc 14 1000 apihub-app "/usr/bin/apihub-app (deleted)" "/usr/bin/apihub-app --krunner"
proc 15 1000 plasmashell /usr/bin/plasmashell /usr/bin/plasmashell
proc 16 1001 apple-kb-monitord /usr/bin/apple-kb-monitord /usr/bin/apple-kb-monitord
proc 17 0 systemd /usr/lib/systemd/systemd "/usr/lib/systemd/systemd --system"
grep -E 'chgrp|usermod|sysusers|setcap|groupadd' "$top/apple-kb-monitor.install" | grep -v '^[[:space:]]*#' | grep -v groupdel \
  && no "no group, no setcap in the scriptlet (the package carries the capability)"
run "$r4" _akm_stale_notice
grep -q '^>>> alice: the ApiHub widget' "$tmp/out.log" || no "plasmashell of alice not told: $(cat "$tmp/out.log")"
grep -q 'bob' "$tmp/out.log" && no "bob runs no plasmashell: nothing to say"
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

# 7. system sleep units: package .wants links; user units: the preset, no vendor user link
for u in apple-kb-monitor-suspend.service apple-kb-monitor-resume.service; do
  grep -qE "ln -s \.\./$u " "$top/PKGBUILD" || no "PKGBUILD ships no .wants link for $u"
done
for u in apple-kb-monitord.service apple-kb-monitor-shutdown.service apple-kb-monitor-selfcheck.timer; do
  grep -qx "enable $u" "$top/systemd/90-apple-kb-monitor.preset" || no "preset does not enable $u"
  grep -qE "ln -s \.\./$u " "$top/PKGBUILD" && no "PKGBUILD still ships a user .wants link for $u"
done
grep -q 'usr/lib/systemd/user-preset/90-apple-kb-monitor.preset' "$top/PKGBUILD" || no "PKGBUILD does not install the preset"

# the install message names only what the package ships
if grep -qi krunner "$top/apple-kb-monitor.install" && ! grep -q 'krunner' "$top/PKGBUILD"; then
  no "the install message announces a KRunner plugin the package does not ship"
fi

# 8. the PKGBUILD itself (locked build, real checksums, .SRCINFO in sync)
bash "$top/tests/check-pkgbuild.sh" || no "tests/check-pkgbuild.sh"

[ $fail = 0 ] && echo "install scriptlet: no enable/start/restart, no sysfs, preset for user units, /etc links dropped on remove/upgrade, orphans cleaned, notices (plasmashell, udev override, user links) right"
exit $fail
