#!/usr/bin/env bash
# Store.qml on a private session bus with no activation: no daemon, then a
# fake slow daemon (deadlines, Device PropertiesChanged), then the test binary $1 (BusCall) if given.
# Needs qml6, dbus-run-session, python3 dbus + gi; 77 = missing.
set -u
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
command -v qml6 >/dev/null && command -v dbus-run-session >/dev/null && python3 -c 'import dbus, gi' 2>/dev/null || exit 77
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cat > "$work/bus.conf" <<XML
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir=$work</listen>
  <auth>EXTERNAL</auth>
  <policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy>
</busconfig>
XML
# A second bus whose only activation directory holds a fake service that leaves a marker: the module must never activate it.
mkdir -p "$work/act/services"
printf '#!/bin/sh\ntouch "%s/act/activated"\n' "$work" > "$work/act/fake-daemon"
chmod +x "$work/act/fake-daemon"
printf '[D-BUS Service]\nName=com.agenceapi.AppleKbMonitor1\nExec=%s/act/fake-daemon\n' "$work" > "$work/act/services/com.agenceapi.AppleKbMonitor1.service"
sed -e "s|<listen>unix:dir=$work</listen>|<listen>unix:dir=$work/act</listen><servicedir>$work/act/services</servicedir>|" "$work/bus.conf" > "$work/act/bus.conf"
# shellcheck disable=SC2016  # expanded by the inner shell
dbus-run-session --config-file="$work/act/bus.conf" -- sh -eu -c '
  out=$(QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 timeout 60 qml6 "$1/tst_store_activation.qml" 2>&1) || { echo "$out" | tail -15; exit 1; }
  echo "$out" | grep -E "PASS|FAIL"
  echo "$out" | grep -q "PASS store-activation"
' sh "$here" || exit 1
if [ -e "$work/act/activated" ]; then echo "FAIL store-activation: the fake daemon was started"; exit 1; fi
# shellcheck disable=SC2016  # expanded by the inner shell
dbus-run-session --config-file="$work/bus.conf" -- sh -eu -c '
  run() { QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 timeout 60 qml6 "$1/$2" 2>&1; }
  check() {
    out=$(run "$1" "tst_store_$2.qml") || { echo "$out" | tail -15; exit 1; }
    echo "$out" | grep -E "PASS|FAIL"
    ! echo "$out" | grep -q "FAIL" && echo "$out" | grep -q "PASS store-$2"
  }
  check "$1" absent
  python3 "$1/fake_slow_daemon.py" & fake=$!
  trap "kill $fake 2>/dev/null" EXIT
  sleep 1
  check "$1" caller
  out=$(run "$1" tst_store.qml) || { echo "$out" | tail -15; exit 1; }
  echo "$out" | grep -E "PASS|FAIL"
  ! echo "$out" | grep -q "FAIL" && echo "$out" | grep -q "PASS store"
  out=$(run "$1" tst_store_props.qml) || { echo "$out" | tail -15; exit 1; }
  echo "$out" | grep -E "PASS|FAIL"
  echo "$out" | grep -q "PASS store-props"
  if [ -n "$2" ]; then "$2"; fi
' sh "$here" "${1:-}"
