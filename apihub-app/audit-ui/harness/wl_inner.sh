#!/bin/bash
S=${AKM_AUDIT_DIR:?export AKM_AUDIT_DIR=<dossier de travail : out/, xdg/, target-uireview/>}; H=$(dirname "$(readlink -f "$0")"); OUT=$S/out/$LABEL; mkdir -p $OUT
export XDG_RUNTIME_DIR=/run/rt; mkdir -p -m 700 $XDG_RUNTIME_DIR
python3 $H/fake_services.py 2> $OUT/fake.log & FP=$!
kwin_wayland --virtual --socket wl-audit --width 1280 --height 900 --no-lockscreen --no-global-shortcuts > $OUT/kwin.log 2>&1 & KP=$!
for i in $(seq 100); do [ -S $XDG_RUNTIME_DIR/wl-audit ] && break; sleep 0.1; done
export WAYLAND_DISPLAY=wl-audit; unset DISPLAY
BIN=$S/target-uireview/release/apihub-app
gdb -q -batch -ex 'set pagination off' -ex 'handle SIGUSR1 stop' -ex run -ex 'thread apply 1 bt 25' --args $BIN > $OUT/gdb.txt 2>&1 & GP=$!
sleep 4; AP=$(pgrep -n -f "^$BIN")
sed "s/(MINI)/$MINI/" $H/minimize.js > $OUT/min.js
ID=$(dbus-send --session --print-reply --dest=org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript string:$OUT/min.js string:akmaudit | awk '/int32/{print $2}')
dbus-send --session --print-reply --dest=org.kde.KWin /Scripting/Script$ID org.kde.kwin.Script.run >/dev/null 2>&1 || dbus-send --session --print-reply --dest=org.kde.KWin /$ID org.kde.kwin.Script.run > /dev/null 2>&1
for t in $(seq 1 10); do sleep 1; echo "t=$t wchan=$(cat /proc/$AP/wchan) ticks=$(awk '{print $14+$15}' /proc/$AP/stat)" >> $OUT/meta.txt; done
kill -USR1 $AP; sleep 15; kill -9 $AP; wait $GP; kill $KP $FP
