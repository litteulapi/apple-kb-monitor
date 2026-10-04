#!/usr/bin/env bash
# Throwaway Plasma desktop for screenshots: Xvfb + plasmashell on a private
# session bus, private HOME/XDG directories, the widget of this tree and the
# fake daemon of plasma/tests/fake_services.py. Touches no real session.
#
#   tests/e2e/plasma-session.sh DIR [demo|nosignal|offline]
#
# Leaves DIR/env (DISPLAY, DBUS_SESSION_BUS_ADDRESS, ...) for the caller and
# runs until killed. Start it with setsid, stop the whole group with
# kill -- -"$(cat DIR/pid)".
set -euo pipefail
top=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
dir=$(realpath -m "$1"); scenario=${2:-demo}
mkdir -p "$dir"/{home,config,data/plasma/plasmoids,cache,state,run}
chmod 700 "$dir/run"
# NO_WIDGET=1: a tray from before the widget was installed (test of EnabledByDefault)
[ -n "${NO_WIDGET:-}" ] || ln -sfn "$top/plasma/com.agenceapi.devicehub" "$dir/data/plasma/plasmoids/com.agenceapi.devicehub"
# notification identity and icons of the package
install -Dm644 "$top/data/apple-kb-monitor.notifyrc" "$dir/data/knotifications6/apple-kb-monitor.notifyrc"
install -Dm644 "$top/icons/apihub-scarab.svg" "$dir/data/icons/hicolor/scalable/apps/apihub-scarab.svg"
export HOME="$dir/home" XDG_CONFIG_HOME="$dir/config" XDG_DATA_HOME="$dir/data" \
       XDG_CACHE_HOME="$dir/cache" XDG_STATE_HOME="$dir/state" XDG_RUNTIME_DIR="$dir/run" \
       LANG=${LANG_CAPTURE:-en_US.UTF-8} LANGUAGE=${LANGUAGE_CAPTURE:-en} QT_QPA_PLATFORM=xcb \
       QT_FORCE_STDERR_LOGGING=1 KDE_FULL_SESSION=true XDG_CURRENT_DESKTOP=KDE XDG_SESSION_TYPE=x11
unset WAYLAND_DISPLAY
# Breeze Dark, as most screenshots of this project
cat > "$XDG_CONFIG_HOME/kdeglobals" <<'K'
[General]
ColorScheme=BreezeDark
[KDE]
LookAndFeelPackage=org.kde.breezedark.desktop
K
printf '[Theme]\nname=breeze-dark\n' > "$XDG_CONFIG_HOME/plasmarc"
# Xvfb picks a free display and writes its number once ready: no clash with another server
rm -f "$dir/display"
Xvfb -displayfd 3 -screen 0 "${CAPTURE_SIZE:-1280x800}x24" -nolisten tcp 3>"$dir/display" >"$dir/xvfb.log" 2>&1 & xpid=$!
end=$((SECONDS + 20))
until [ -s "$dir/display" ]; do
  if ! kill -0 "$xpid" 2>/dev/null || [ $SECONDS -ge $end ]; then
    echo "Xvfb did not start:" >&2; cat "$dir/xvfb.log" >&2; exit 1
  fi
  sleep 0.1
done
DISPLAY=:$(head -n1 "$dir/display")
export DISPLAY
echo $$ > "$dir/pid"
trap 'kill $xpid 2>/dev/null' EXIT
# shellcheck disable=SC2016  # expanded by the inner shell
dbus-run-session -- bash -c '
  dir=$1; top=$2; scenario=$3
  { echo "DISPLAY=$DISPLAY"; echo "DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS"
    for v in HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_CACHE_HOME XDG_STATE_HOME XDG_RUNTIME_DIR LANG LANGUAGE QT_QPA_PLATFORM; do echo "$v=${!v}"; done; } > "$dir/env"
  python3 "$top/plasma/tests/fake_services.py" "$dir/calls" "$scenario" 2>"$dir/fake.err" &
  kded6 >"$dir/kded.log" 2>&1 &
  sleep 1
  plasmashell --no-respawn >"$dir/plasmashell.log" 2>&1 &
  wait
' bash "$dir" "$top" "$scenario"
