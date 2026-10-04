#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Runs one KCM test program in a sealed bench (audit of 2026-10-03):
#  - a PRIVATE session bus started from a configuration with NO activation
#    directory (no <servicedir>, no <standard_session_servicedirs/>): the
#    default session configuration would let the installed
#    com.agenceapi.AppleKbMonitor1.service start the real apple-kb-monitord;
#  - HOME and every XDG directory in a temporary directory under $TMPDIR;
#  - Qt offscreen, no display, no session manager.
# The test program puts its fake akmctl / pkexec / systemctl first in PATH.
#   private_bus.sh <program> [args...]
set -uo pipefail
command -v dbus-run-session >/dev/null || { echo "dbus-run-session missing"; exit 77; }
base="${TMPDIR:-/tmp}"
work=$(mktemp -d "$base/akm-kcm-bench.XXXXXX") || exit 1
trap 'chmod -R u+rwx "$work" 2>/dev/null; rm -rf "$work"' EXIT
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
if grep -qiE 'servicedir|includedir|<include' "$work/bus.conf"; then
  echo "bench: the private bus must have no activation directory" >&2
  exit 1
fi
mkdir -p "$work/home" "$work/config" "$work/data" "$work/cache" "$work/state" "$work/runtime" "$work/bench"
chmod 700 "$work/runtime"
export HOME="$work/home" XDG_CONFIG_HOME="$work/config" XDG_DATA_HOME="$work/data" \
  XDG_CACHE_HOME="$work/cache" XDG_STATE_HOME="$work/state" XDG_RUNTIME_DIR="$work/runtime" \
  AKM_BENCH_DIR="$work/bench" QT_QPA_PLATFORM=offscreen
unset DISPLAY WAYLAND_DISPLAY SESSION_MANAGER DBUS_SESSION_BUS_ADDRESS KDE_FULL_SESSION
# French interface whatever the caller's locale (ci-local.sh runs with LC_ALL=C)
unset LC_ALL LC_MESSAGES
export LANGUAGE=fr LANG=fr_FR.UTF-8
# gettext ignores LANGUAGE under the C locale: without fr_FR (CI container) take en_US, compiled into the bench if need be.
if ! locale -a 2>/dev/null | grep -qixE 'fr_FR\.utf-?8'; then
  LANG=en_US.UTF-8
  if ! locale -a 2>/dev/null | grep -qixE 'en_US\.utf-?8'; then
    mkdir -p "$work/locale" && localedef -i en_US -f UTF-8 "$work/locale/en_US.UTF-8" >/dev/null 2>&1 || { echo "bench: no fr_FR or en_US locale, and localedef failed"; exit 77; }
    export LOCPATH="$work/locale"
  fi
fi
# The real daemon serves the module (Settings, Keymap) on this bus, sealed:
# no hidraw, no power_supply, no system bus (BlueZ unreachable), fake akmctl
# (AKM_AKMCTL, read only by a daemon built with --features testbus).
daemon=${AKM_DAEMON:-}
[ -x "$daemon" ] || { echo "AKM_DAEMON (apple-kb-monitord binary) not set"; exit 77; }
command -v bwrap >/dev/null || { echo "bwrap missing"; exit 77; }
args="--no-bluez-provider --no-notify --no-history"
if bwrap --dev-bind / / --dev /dev --tmpfs /sys/class/power_supply --die-with-parent -- true 2>"$work/bwrap.err"; then
  export AKM_DAEMON_SEALED="bwrap --dev-bind / / --dev /dev --tmpfs /sys/class/hidraw --tmpfs /sys/bus/hid/devices --tmpfs /sys/class/power_supply --die-with-parent --setenv DBUS_SYSTEM_BUS_ADDRESS unix:path=$work/no-system-bus --setenv AKM_AKMCTL $work/bench/bin/akmctl -- $daemon $args"
elif { [ -e /.dockerenv ] || [ -e /run/.containerenv ]; } && ! compgen -G '/dev/hidraw*' >/dev/null; then
  # Docker refuses user namespaces to an unprivileged user (GitHub CI): the container is the seal there, no hidraw node and a read-only /sys.
  echo "bench: bwrap refused ($(head -c 200 "$work/bwrap.err")); container without hidraw: the daemon runs in the container's own seal" >&2
  export AKM_DAEMON_SEALED="setpriv --pdeathsig KILL -- env DBUS_SYSTEM_BUS_ADDRESS=unix:path=$work/no-system-bus AKM_AKMCTL=$work/bench/bin/akmctl $daemon $args"
else
  echo "bench: bwrap cannot seal the daemon here: $(cat "$work/bwrap.err")" >&2
  exit 1
fi
# A daemon that exits or never registers fails the bench at once: the tests would only wait for it.
# shellcheck disable=SC2016  # expanded by the inner shell
dbus-run-session --config-file="$work/bus.conf" -- sh -c '$AKM_DAEMON_SEALED >"$AKM_BENCH_DIR/daemon.log" 2>&1 & d=$!
  i=0; until busctl --user status com.agenceapi.AppleKbMonitor1 >/dev/null 2>&1; do
    if ! kill -0 $d 2>/dev/null || [ $i -ge 300 ]; then
      echo "bench: apple-kb-monitord did not register on the private bus; daemon.log:" >&2; tail -n 40 "$AKM_BENCH_DIR/daemon.log" >&2
      kill $d 2>/dev/null; exit 1
    fi
    i=$((i+1)); sleep 0.1
  done
  "$@"; rc=$?
  [ $rc = 0 ] || { echo "bench: daemon.log (last 40 lines):" >&2; tail -n 40 "$AKM_BENCH_DIR/daemon.log" >&2; }
  kill $d 2>/dev/null; wait $d 2>/dev/null; exit $rc' sh "$@"
