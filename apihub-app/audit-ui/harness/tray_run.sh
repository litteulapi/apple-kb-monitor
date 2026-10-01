#!/bin/bash
S=${AKM_AUDIT_DIR:?export AKM_AUDIT_DIR=<dossier de travail : out/, xdg/, target-uireview/>}; H=$(dirname "$(readlink -f "$0")")
exec bwrap --ro-bind / / --unshare-net --tmpfs /usr/lib/apple-kb-monitor --dev /dev --tmpfs /run --proc /proc --bind $S $S --die-with-parent \
  --unsetenv DBUS_SESSION_BUS_ADDRESS --setenv XDG_DATA_HOME $S/xdg --setenv XDG_RUNTIME_DIR $S/xdg --setenv LABEL "$1" --setenv SECS "$2" --setenv UPOWER "${UPOWER:-}" \
  dbus-run-session --config-file $H/session.conf -- bash $H/tray_inner.sh
