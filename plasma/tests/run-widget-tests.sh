#!/bin/sh
# Widget tests on a PRIVATE session bus (dbus-run-session): fake daemon, no
# real keyboard, no real service. Proves:
#  - StateChanged is received natively and GetState is read (`percentage`);
#  - Tray.PanelRequested opens the popup;
#  - the widget spawns NO subprocess (no orphans: dbus-monitor/timeout/sh);
#  - History(since), Device.FnMode / SetFnMode, Link.Status / Reconnect and
#    Refresh and Diagnose are called on the daemon as declared.
# Needs: qml6, python3-dbus + gi, dbus-run-session.
set -eu
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
# Static guard: the widget must never go back to a subprocess engine.
if grep -rEn "plasma5support|dbus-monitor|engine: *\"executable\"" "$here/../com.agenceapi.devicehub/contents"; then
  echo "FAIL: the widget starts subprocesses again" >&2; exit 1
fi
# The system tray only opens the popup of applets without a
# preferredRepresentation (Plasma 6 SystemTrayState.setActiveApplet): with
# one, a left click did nothing at all.
if grep -rEn "^[^/]*preferredRepresentation *:" "$here/../com.agenceapi.devicehub/contents/ui"; then
  echo "FAIL: preferredRepresentation set, the system tray popup would never open" >&2; exit 1
fi
# Displayed text is upper-cased for the user's locale (Turkish dotted I), only MACs use toUpperCase.
if grep -rn "toUpperCase()" "$here/../com.agenceapi.devicehub/contents/ui"/*.qml | grep -v "kbMac"; then
  echo "FAIL: toUpperCase() on displayed text: use toLocaleUpperCase()" >&2; exit 1
fi
# Every bus call is gated on the daemon owning its name: a call to a free name
# would make dbus-daemon auto-start the installed daemon.
if ! awk '/function [A-Za-z]+\(/{g=0} /watcher\.registered/{g=1} /SessionBus\.asyncCall/ && !g {print FILENAME": "FNR; bad=1} END{exit bad}' "$here/../com.agenceapi.devicehub/contents/ui/DaemonLink.qml"; then
  echo "FAIL: D-Bus call not gated on watcher.registered" >&2; exit 1
fi
# A stopped daemon leaves no error or check result of its old session on screen.
for v in lastError linkError linkAttempts linkFailures pairedHost diagChecks diagPassed diagTotal diagError \
         fnError renameError historyError forecastRate forecastSpanDays thresholds thresholdsText installedAt lastUpdate; do
  sed -n '/^    function clear() {/,/^    }/p' "$here/../com.agenceapi.devicehub/contents/ui/main.qml" | grep -q "root\.$v = " \
    || { echo "FAIL: clear() keeps $v" >&2; exit 1; }
done
# Display data is plain text, never rich text.
python3 "$here/check_plaintext.py" >/dev/null || { python3 "$here/check_plaintext.py" >&2; echo "FAIL: rich text in widget" >&2; exit 1; }
# ... and the guard sees the component that draws every datum.
mut=$(mktemp -d "${TMPDIR:-/tmp}/akm-plaintext.XXXXXX")
cp "$here/../com.agenceapi.devicehub/contents/ui/"*.qml "$mut/"
sed -i '/textFormat: Text.PlainText/d' "$mut/PipText.qml"
if python3 "$here/check_plaintext.py" "$mut" >/dev/null 2>&1; then rm -rf "$mut"; echo "FAIL: check_plaintext.py misses PipText without PlainText" >&2; exit 1; fi
rm -rf "$mut"
# Every i18n() text of the widget is translated in every po/<lang>.po.
python3 "$here/check_i18n.py" >/dev/null || { python3 "$here/check_i18n.py" >&2; echo "FAIL: untranslated widget text" >&2; exit 1; }
# The fake daemon sends only fields the real GetState has.
python3 "$here/check_fake_state.py" >/dev/null || { python3 "$here/check_fake_state.py" >&2; exit 1; }
# The applet's "Configure…" entry opens the System Settings module.
python3 "$here/check_configure.py" || { echo "FAIL: configure action" >&2; exit 1; }
# The template and the catalogues are what plasma/Messages.sh generates.
if command -v xgettext >/dev/null; then
  bash "$here/../Messages.sh" --check >/dev/null || { echo "FAIL: plasma/Messages.sh --check (run plasma/Messages.sh)" >&2; exit 1; }
fi
# Pure logic of the signal quality, no bus needed.
QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 qml6 "$here/tst_signal.qml" 2>&1 | grep -q "PASS signal" \
  || { echo "FAIL: tst_signal.qml" >&2; exit 1; }
# History series and Fn mode toggle of the widget, no bus needed.
QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 qml6 "$here/tst_history.qml" 2>&1 | grep -q "PASS history" \
  || { QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 qml6 "$here/tst_history.qml" 2>&1 | tail -5 >&2; echo "FAIL: tst_history.qml" >&2; exit 1; }
# Right-click menu entries, and the swap that kept destroying the new actions.
menu=$(QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 timeout 30 qml6 "$here/tst_menu.qml" 2>&1) || true
if echo "$menu" | grep -qE "module \"org\.kde\.[a-z.]+\" is not installed"; then
  echo "tst_menu.qml skipped: Plasma QML modules not installed"
elif ! echo "$menu" | grep -q "PASS menu"; then
  echo "$menu" | tail -5 >&2; echo "FAIL: tst_menu.qml" >&2; exit 1
fi
# The popup and its History / Diagnostic pages load offscreen against a
# stand-in applet, without any binding on a missing name. Needs the
# Plasma QML modules; D-Bus is not used (DBUS_SESSION_BUS_ADDRESS is unset so
# that nothing of the real session can be reached).
pages=$(env -u DBUS_SESSION_BUS_ADDRESS QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 timeout 60 qml6 "$here/tst_pages.qml" 2>&1) || true
if echo "$pages" | grep -qE "module \"org\.kde\.[a-z.]+\" is not installed"; then
  echo "tst_pages.qml skipped: Plasma QML modules not installed"
elif ! echo "$pages" | grep -q "PASS pages" || echo "$pages" | grep -E "ReferenceError|TypeError|is not defined|Unable to assign|Cannot read property|is not a type"; then
  echo "$pages" | tail -15 >&2; echo "FAIL: tst_pages.qml" >&2; exit 1
fi
# Private bus from a configuration with NO activation directory (as
# kcm/tests/private_bus.sh): the default session configuration would let the
# installed com.agenceapi.AppleKbMonitor1.service start the real daemon.
work=$(mktemp -d "${TMPDIR:-/tmp}/akm-widget-bench.XXXXXX")
trap 'rm -rf "$work"' EXIT
cat > "$work/bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir=$work</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF
mkdir -p "$work/home" "$work/config" "$work/data" "$work/cache" "$work/state" "$work/runtime"
chmod 700 "$work/runtime"
export HOME="$work/home" XDG_CONFIG_HOME="$work/config" XDG_DATA_HOME="$work/data" \
  XDG_CACHE_HOME="$work/cache" XDG_STATE_HOME="$work/state" XDG_RUNTIME_DIR="$work/runtime"
unset DISPLAY WAYLAND_DISPLAY SESSION_MANAGER DBUS_SESSION_BUS_ADDRESS
# shellcheck disable=SC2016  # expanded by the inner shell
dbus-run-session --config-file="$work/bus.conf" -- sh -eu -c '
  here="$1"
  act=$(python3 -c "import dbus; print(*dbus.SessionBus().list_activatable_names())")
  [ "$act" = org.freedesktop.DBus ] || { echo "FAIL: the test bus can activate services: $act" >&2; exit 1; }
  out=$(mktemp)
  python3 "$here/fake_services.py" "$out" 2>"$out.err" & fake=$!
  trap "kill $fake 2>/dev/null; rm -f $out" EXIT
  i=0; until python3 -c "import dbus, sys; sys.exit(not dbus.SessionBus().name_has_owner(sys.argv[1]))" com.agenceapi.AppleKbMonitor1 2>/dev/null; do
    if ! kill -0 "$fake" 2>/dev/null || [ "$i" -ge 100 ]; then echo "FAIL: the fake daemon did not take its name" >&2; cat "$out.err" >&2; exit 1; fi
    i=$((i+1)); sleep 0.1
  done
  QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 qml6 "$here/tst_link.qml" >"$out.log" 2>&1 & q=$!
  sleep 2
  kids=$(pgrep -P "$q" | wc -l)
  stray=$(pgrep -x dbus-monitor | wc -l)
  wait "$q" && rc=0 || rc=$?
  grep -q PASS "$out.log" && grep -E "RESULT|PASS" "$out.log" || cat "$out.log"
  echo "children of the widget process: $kids ; stray dbus-monitor: $stray"
  late=$(QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 timeout 30 qml6 "$here/tst_fnmode.qml" 2>&1) || rc=1
  echo "$late" | grep -E "PASS|FAIL" || echo "$late" | tail -5
  cat "$out.err" | tail -5; rm -f "$out.log" "$out.err"
  [ "$kids" -eq 0 ] && [ "$stray" -eq 0 ] && [ "$rc" -eq 0 ] && grep -q "^alias AA:BB My keyboard" "$out" && grep -q "^fnmode 2" "$out" && [ "$(grep -c "^fnmode" "$out")" -eq 1 ] && grep -q "^refresh" "$out" && grep -q "^reconnect" "$out" && grep -q "^diagnose" "$out" && grep -Eq "^history 6048[0-9][0-9]$" "$out" \
    && grep -q "^menuitems" "$out" && grep -q "^late-fnmode 3" "$out"  # StateChanged refetches the menu
' sh "$here"
