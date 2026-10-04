#!/usr/bin/env bash
# End-to-end test of the Plasma widget in the notification area of a throwaway
# plasmashell (Xvfb, private session bus, fake daemon): the right-click menu
# carries the daemon's entries, not only "Configure".
#   tests/e2e/run.sh [--out DIR]
# Every wait is on a condition with a deadline, never a fixed delay: under load
# plasmashell may need far more time to load the tray and fetch the menu.
set -uo pipefail
top=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
out=${TMPDIR:-/tmp}/akm-e2e
[ "${1:-}" = --out ] && out=$2
for b in Xvfb plasmashell kded6 qdbus6 xdotool import dbus-run-session; do
  command -v "$b" >/dev/null || { echo "e2e skipped: missing $b"; exit 77; }
done
rm -rf "$out"; mkdir -p "$out"
s=$out/session
export LANG_CAPTURE=en_US.UTF-8 LANGUAGE_CAPTURE=en
stop() {
  for p in $(pgrep -u "$(id -u)"); do
    [ "$p" = $$ ] && continue
    grep -qa -- "$s" "/proc/$p/environ" "/proc/$p/cmdline" 2>/dev/null && kill "$p" 2>/dev/null
  done
}
trap stop EXIT
# until SECONDS CMD...: runs CMD every 0.5 s until it succeeds, fails after SECONDS
until_ok() {
  local end=$((SECONDS + $1)); shift
  until "$@"; do [ $SECONDS -lt $end ] || return 1; sleep 0.5; done
}
shell_up() {
  # shellcheck source=/dev/null  # written by plasma-session.sh at run time
  [ -s "$s/env" ] && { set -a; . "$s/env"; set +a; } && qdbus6 org.kde.plasmashell /PlasmaShell >/dev/null 2>&1
}
# the widget instance fetched the daemon's menu (fake_services.py logs it)
menu_fetched() { grep -qx menuitems "$s/calls" 2>/dev/null; }
# the panel's notification area no longer changes between two looks 1 s apart
tray=1280x48+0+752 last=""
tray_settled() {
  local now
  now=$(import -window root -crop "$tray" +repage ppm:- 2>/dev/null | md5sum)
  [ "$now" = "$last" ] && return 0
  last=$now; sleep 0.5; return 1
}
popups() { xdotool search --onlyvisible --class plasmashell 2>/dev/null | sort; }
# height of the tallest plasmashell window opened since $before
menu_height() {
  local w g h=0
  for w in $(comm -13 <(echo "$before") <(popups)); do
    g=$(xdotool getwindowgeometry "$w" | sed -n 's/.*Geometry: [0-9]*x\([0-9]*\).*/\1/p')
    [ "${g:-0}" -gt "$h" ] && h=$g
  done
  echo "$h"
}
# a menu is open and its height is the same on two looks 0.3 s apart
menu_settled() {
  local a b
  a=$(menu_height); [ "$a" -gt 0 ] || return 1
  sleep 0.3; b=$(menu_height)
  h=$b; [ "$a" = "$b" ]
}

setsid "$top/tests/e2e/plasma-session.sh" "$s" demo >"$out/session.log" 2>&1 </dev/null &
until_ok 120 shell_up || { echo "FAIL: plasmashell did not start"; exit 1; }
qdbus6 org.kde.plasmashell /PlasmaShell org.kde.PlasmaShell.evaluateScript '
  var st = panels()[0].widgets("org.kde.plasma.systemtray")[0];
  st.currentConfigGroup = ["General"];
  // the widget alone in the notification area: its position no longer depends on the others
  st.writeConfig("extraItems", "com.agenceapi.devicehub");
  st.writeConfig("shownItems", "com.agenceapi.devicehub");
  st.reloadConfig();' >/dev/null
until_ok 120 menu_fetched || { echo "FAIL: the widget never asked the daemon for its menu"; exit 1; }
until_ok 60 tray_settled || { echo "FAIL: the notification area never settled"; exit 1; }
import -window root "$out/tray.png"
h=0
for _ in 1 2 3; do  # a click lost by a busy shell opens nothing: click again, never accept a short menu
  before=$(popups)
  xdotool mousemove "${AKM_E2E_ICON_X:-1132}" 776 click 3
  until_ok 20 menu_settled && break
  xdotool key Escape
done
import -window root "$out/menu.png"
xdotool key Escape
echo "right-click menu height: $h px (screenshot $out/menu.png)"
[ "$h" -ge 150 ] || { echo "FAIL: right-click menu too short, daemon entries missing"; exit 1; }
echo "PASS e2e widget menu"
