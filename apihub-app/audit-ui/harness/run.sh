#!/bin/bash
# usage: run.sh <label> <seconds> [gdb] ; env knobs forwarded to fake services
# Sandbox: no /dev/hidraw*, no system bus, private session bus without service dirs.
S=${AKM_AUDIT_DIR:?export AKM_AUDIT_DIR=<dossier de travail : out/, xdg/, target-uireview/>}
H=$(dirname "$(readlink -f "$0")"); OUT=$S/out/$1; mkdir -p $OUT $S/xdg
export DISPLAY=${DISPLAY_N:-:95}
exec bwrap --ro-bind / / --unshare-net --tmpfs /usr/lib/apple-kb-monitor --tmpfs /sys/class/bluetooth --dev /dev --tmpfs /run --proc /proc --bind $S $S \
  --bind /tmp/.X11-unix /tmp/.X11-unix --die-with-parent \
  --unsetenv WAYLAND_DISPLAY --unsetenv DBUS_SESSION_BUS_ADDRESS --setenv XDG_DATA_HOME $S/xdg \
  --setenv XDG_RUNTIME_DIR $S/xdg --setenv LABEL "$1" --setenv SECS "$2" --setenv MODE "${3:-}" --setenv BIN "${BIN:-}" ${XENV:-} \
  dbus-run-session --config-file $H/session.conf -- bash $H/inner.sh
