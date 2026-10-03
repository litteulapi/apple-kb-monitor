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
dbus-run-session --config-file="$work/bus.conf" -- "$@"
