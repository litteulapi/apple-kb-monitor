#!/bin/bash
S=${AKM_AUDIT_DIR:?export AKM_AUDIT_DIR=<dossier de travail : out/, xdg/, target-uireview/>}; H=$(dirname "$(readlink -f "$0")"); OUT=$S/out/$LABEL; mkdir -p $OUT
export DBUS_SYSTEM_BUS_ADDRESS=$(dbus-daemon --config-file=$H/session.conf --print-address --fork | head -1)
python3 $H/fake_tray_env.py 2> $OUT/fake.log & FP=$!
for i in $(seq 50); do grep -q ready $OUT/fake.log && break; sleep 0.1; done
T=$(ls -t $S/target-uireview/debug/deps/apple_kb_monitord-* | grep -v '\.d$' | head -1)
APPLE_KB_MONITOR_TRAY=always AKM_TRAY_LIVE_SECS=$SECS timeout $((SECS+10)) $T live_tray --ignored --nocapture > $OUT/test.log 2>&1
kill $FP
