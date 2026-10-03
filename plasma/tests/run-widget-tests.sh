#!/bin/sh
# Widget tests on a PRIVATE session bus (dbus-run-session): fake daemon + fake
# window, no real keyboard, no real service. Proves (#149, #150):
#  - StateChanged is received natively and GetState is read (`percentage`);
#  - the window opens through org.freedesktop.Application.Activate;
#  - the widget spawns NO subprocess (no orphans: dbus-monitor/timeout/sh);
#  - History(since), Device.FnMode / SetFnMode, Link.Status / Reconnect and
#    Refresh and Diagnose are called on the daemon as declared (#97, #120).
# Needs: qml6, python3-dbus + gi, dbus-run-session.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# Static guard: the widget must never go back to a subprocess engine (#150).
if grep -rEn "plasma5support|dbus-monitor|engine: *\"executable\"" "$here/../com.agenceapi.devicehub/contents"; then
  echo "FAIL: the widget starts subprocesses again" >&2; exit 1
fi
# The system tray only opens the popup of applets without a
# preferredRepresentation (Plasma 6 SystemTrayState.setActiveApplet): with
# one, a left click did nothing at all.
if grep -rEn "^[^/]*preferredRepresentation *:" "$here/../com.agenceapi.devicehub/contents/ui"; then
  echo "FAIL: preferredRepresentation set, the system tray popup would never open" >&2; exit 1
fi
# Display data is plain text, never rich text (#205).
python3 "$here/check_plaintext.py" >/dev/null || { python3 "$here/check_plaintext.py" >&2; echo "FAIL: rich text in widget" >&2; exit 1; }
# Every i18n() text of the widget is translated in po/fr.po (#114).
python3 "$here/check_i18n.py" >/dev/null || { python3 "$here/check_i18n.py" >&2; echo "FAIL: untranslated widget text" >&2; exit 1; }
# Pure logic of the signal quality (#174), no bus needed.
QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 qml6 "$here/tst_signal.qml" 2>&1 | grep -q "PASS signal" \
  || { echo "FAIL: tst_signal.qml" >&2; exit 1; }
# History series and Fn mode toggle of the widget (#97, #120), no bus needed.
QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 qml6 "$here/tst_history.qml" 2>&1 | grep -q "PASS history" \
  || { QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 qml6 "$here/tst_history.qml" 2>&1 | tail -5 >&2; echo "FAIL: tst_history.qml" >&2; exit 1; }
# The popup and its History / Diagnostic pages load offscreen against a
# stand-in applet, without any binding on a missing name (#120). Needs the
# Plasma QML modules; D-Bus is not used (DBUS_SESSION_BUS_ADDRESS is unset so
# that nothing of the real session can be reached).
pages=$(env -u DBUS_SESSION_BUS_ADDRESS QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 timeout 60 qml6 "$here/tst_pages.qml" 2>&1) || true
if echo "$pages" | grep -qE "module \"org\.kde\.[a-z.]+\" is not installed"; then
  echo "tst_pages.qml skipped: Plasma QML modules not installed"
elif ! echo "$pages" | grep -q "PASS pages" || echo "$pages" | grep -E "ReferenceError|TypeError|is not defined|Unable to assign|Cannot read property|is not a type"; then
  echo "$pages" | tail -15 >&2; echo "FAIL: tst_pages.qml" >&2; exit 1
fi
exec dbus-run-session -- sh -eu -c '
  here="$1"
  out=$(mktemp)
  python3 "$here/fake_services.py" "$out" 2>"$out.err" & fake=$!
  trap "kill $fake 2>/dev/null; rm -f $out" EXIT
  sleep 1
  QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 qml6 "$here/tst_link.qml" >"$out.log" 2>&1 & q=$!
  sleep 2
  kids=$(pgrep -P "$q" | wc -l)
  stray=$(pgrep -x dbus-monitor | wc -l)
  wait "$q" && rc=0 || rc=$?
  grep -q PASS "$out.log" && grep -E "RESULT|PASS" "$out.log" || cat "$out.log"
  echo "children of the widget process: $kids ; stray dbus-monitor: $stray ; Activate calls: $(grep -c activate "$out")"
  cat "$out.err" | tail -5; rm -f "$out.log" "$out.err"
  [ "$kids" -eq 0 ] && [ "$stray" -eq 0 ] && [ "$rc" -eq 0 ] && grep -q "^activate 0" "$out" && grep -q "^alias AA:BB Mon clavier" "$out" && grep -q "^claim 42" "$out" && grep -q "^release 42" "$out" && grep -q "^fnmode 2" "$out" && [ "$(grep -c "^fnmode" "$out")" -eq 1 ] && grep -q "^refresh" "$out" && grep -q "^reconnect" "$out" && grep -q "^diagnose" "$out" && grep -Eq "^history 6048[0-9][0-9]$" "$out"
' sh "$here"
